//! `wx-e2e-sink APP_ID STATE_FILE`
//!
//! A minimal Wayland window that records exactly what its keyboard focus
//! receives, as the oracle for the headless KWin end-to-end tests. It keeps
//! a text buffer with a caret: printable keys insert at the caret, BackSpace
//! and Delete remove one character, Left/Right/Home/End move the caret, and
//! Return inserts a newline. After every change it atomically rewrites
//! STATE_FILE as JSON: `{"app_id", "focused", "text", "cursor", "keys"}`.
//!
//! It deliberately implements no key repeat, input method, or clipboard, so
//! the recorded text is a deterministic function of the key events KWin
//! delivered.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsFd, OwnedFd},
    path::PathBuf,
};

use anyhow::{bail, Context, Result};
use wayland_client::{
    protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool,
        wl_surface,
    },
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use xkbcommon::xkb;

const WIDTH: i32 = 320;
const HEIGHT: i32 = 200;

struct Sink {
    app_id: String,
    state_path: PathBuf,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    buffer: Option<wl_buffer::WlBuffer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    xkb: Option<xkb::State>,
    focused: bool,
    text: Vec<char>,
    cursor: usize,
    keys: u64,
    closed: bool,
}

impl Sink {
    fn write_state(&self) {
        let state = serde_json::json!({
            "app_id": self.app_id,
            "focused": self.focused,
            "text": self.text.iter().collect::<String>(),
            "cursor": self.cursor,
            "keys": self.keys,
        });
        let temporary = self.state_path.with_extension("tmp");
        let written = File::create(&temporary)
            .and_then(|mut file| file.write_all(state.to_string().as_bytes()))
            .and_then(|()| std::fs::rename(&temporary, &self.state_path));
        if let Err(error) = written {
            eprintln!("wx-e2e-sink: could not write state: {error}");
        }
    }

    fn press(&mut self, keycode: xkb::Keycode) {
        let Some(state) = self.xkb.as_ref() else {
            return;
        };
        let symbol = state.key_get_one_sym(keycode);
        let text = state.key_get_utf8(keycode);
        match symbol.raw() {
            xkb::keysyms::KEY_BackSpace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.text.remove(self.cursor);
                }
            }
            xkb::keysyms::KEY_Delete => {
                if self.cursor < self.text.len() {
                    self.text.remove(self.cursor);
                }
            }
            xkb::keysyms::KEY_Left => self.cursor = self.cursor.saturating_sub(1),
            xkb::keysyms::KEY_Right => self.cursor = (self.cursor + 1).min(self.text.len()),
            xkb::keysyms::KEY_Home => self.cursor = 0,
            xkb::keysyms::KEY_End => self.cursor = self.text.len(),
            xkb::keysyms::KEY_Return | xkb::keysyms::KEY_KP_Enter => self.insert("\n"),
            _ if !text.is_empty() && !text.chars().any(char::is_control) => self.insert(&text),
            _ => {}
        }
        self.keys += 1;
    }

    fn insert(&mut self, text: &str) {
        for character in text.chars() {
            self.text.insert(self.cursor, character);
            self.cursor += 1;
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        let (Some(shm), Some(surface)) = (self.shm.as_ref(), self.surface.as_ref()) else {
            bail!("compositor did not offer wl_shm and a surface");
        };
        let stride = WIDTH * 4;
        let size = stride * HEIGHT;
        let fd: OwnedFd = rustix::fs::memfd_create("wx-e2e-sink", rustix::fs::MemfdFlags::CLOEXEC)
            .context("memfd_create")?;
        let mut file = File::from(fd);
        // A mid-grey frame, so screenshots show where the window is.
        file.write_all(&vec![0x80; size as usize])
            .context("filling the frame")?;
        let pool = shm.create_pool(file.as_fd(), size, qh, ());
        let buffer = pool.create_buffer(0, WIDTH, HEIGHT, stride, wl_shm::Format::Xrgb8888, qh, ());
        pool.destroy();
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, WIDTH, HEIGHT);
        surface.commit();
        self.buffer = Some(buffer);
        Ok(())
    }
}

fn read_keymap(fd: OwnedFd, size: u32) -> Result<xkb::Keymap> {
    let mut file = File::from(fd);
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    let text = String::from_utf8(bytes).context("keymap is not UTF-8")?;
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_string(
        &context,
        text,
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .context("compositor keymap does not compile")
}

impl Dispatch<wl_registry::WlRegistry, ()> for Sink {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_compositor" => {
                state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
            "xdg_wm_base" => state.wm_base = Some(registry.bind(name, version.min(2), qh, ())),
            "wl_seat" => {
                let _: wl_seat::WlSeat = registry.bind(name, version.min(7), qh, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Sink {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Sink {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { fd, size, .. } => match read_keymap(fd, size) {
                Ok(keymap) => state.xkb = Some(xkb::State::new(&keymap)),
                Err(error) => eprintln!("wx-e2e-sink: {error:#}"),
            },
            wl_keyboard::Event::Enter { .. } => {
                state.focused = true;
                state.write_state();
            }
            wl_keyboard::Event::Leave { .. } => {
                state.focused = false;
                state.write_state();
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                let keycode = xkb::Keycode::new(key + 8);
                if key_state == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    state.press(keycode);
                    if let Some(xkb) = state.xkb.as_mut() {
                        xkb.update_key(keycode, xkb::KeyDirection::Down);
                    }
                    state.write_state();
                } else if let Some(xkb) = state.xkb.as_mut() {
                    xkb.update_key(keycode, xkb::KeyDirection::Up);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(xkb) = state.xkb.as_mut() {
                    xkb.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Sink {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Sink {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if state.buffer.is_none() {
                if let Err(error) = state.draw(qh) {
                    eprintln!("wx-e2e-sink: {error:#}");
                    state.closed = true;
                }
            } else if let Some(surface) = state.surface.as_ref() {
                surface.commit();
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Sink {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.closed = true;
        }
    }
}

macro_rules! ignore_events {
    ($($proxy:ty),*) => {$(
        impl Dispatch<$proxy, ()> for Sink {
            fn event(
                _: &mut Self,
                _: &$proxy,
                _: <$proxy as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer
);

fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let (Some(app_id), Some(state_path), None) =
        (arguments.next(), arguments.next(), arguments.next())
    else {
        bail!("usage: wx-e2e-sink APP_ID STATE_FILE");
    };
    let connection = Connection::connect_to_env().context("connecting to Wayland")?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut sink = Sink {
        app_id: app_id.clone(),
        state_path: PathBuf::from(state_path),
        compositor: None,
        shm: None,
        wm_base: None,
        surface: None,
        buffer: None,
        keyboard: None,
        xkb: None,
        focused: false,
        text: Vec::new(),
        cursor: 0,
        keys: 0,
        closed: false,
    };
    queue.roundtrip(&mut sink)?;
    let (Some(compositor), Some(wm_base)) = (sink.compositor.clone(), sink.wm_base.clone()) else {
        bail!("compositor lacks wl_compositor or xdg_wm_base");
    };
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_app_id(app_id.clone());
    toplevel.set_title(format!("WayExpand e2e sink {app_id}"));
    surface.commit();
    sink.surface = Some(surface);
    sink.write_state();
    while !sink.closed {
        queue.blocking_dispatch(&mut sink)?;
    }
    Ok(())
}
