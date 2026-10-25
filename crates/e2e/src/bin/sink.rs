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
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    zwp_text_input_v3::{ContentHint, ContentPurpose, ZwpTextInputV3},
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
    text_input_manager: Option<ZwpTextInputManagerV3>,
    seat: Option<wl_seat::WlSeat>,
    text_input: Option<ZwpTextInputV3>,
    surface: Option<wl_surface::WlSurface>,
    buffer: Option<wl_buffer::WlBuffer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    xkb: Option<xkb::State>,
    focused: bool,
    text_input_enabled: bool,
    password_field: bool,
    text_input_active: bool,
    pending_commit: Option<String>,
    pending_delete: Option<(u32, u32)>,
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
            "text_input_active": self.text_input_active,
            "content_purpose": if self.password_field { "password" } else { "normal" },
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
            eprintln!("wx-e2e-sink: key {} without a keymap", keycode.raw());
            return;
        };
        let symbol = state.key_get_one_sym(keycode);
        let text = state.key_get_utf8(keycode);
        // The log is the first thing to read when a scenario's text is
        // wrong: it shows what the compositor delivered and how it decoded.
        eprintln!(
            "wx-e2e-sink: key {} sym {:#x} text {text:?}",
            keycode.raw(),
            symbol.raw()
        );
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

    fn start_text_input(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        let (Some(manager), Some(seat)) = (self.text_input_manager.as_ref(), self.seat.as_ref())
        else {
            bail!("compositor lacks text-input-v3 or a Wayland seat");
        };
        let text_input = manager.get_text_input(seat, qh, ());
        self.text_input = Some(text_input);
        Ok(())
    }

    fn enable_text_input(&self) {
        let Some(text_input) = self.text_input.as_ref() else {
            return;
        };
        text_input.enable();
        self.set_text_input_content(text_input);
        self.set_surrounding_text(text_input);
        text_input.commit();
    }

    fn set_text_input_content(&self, text_input: &ZwpTextInputV3) {
        if self.password_field {
            text_input.set_content_type(
                ContentHint::SensitiveData | ContentHint::HiddenText,
                ContentPurpose::Password,
            );
        } else {
            text_input.set_content_type(ContentHint::empty(), ContentPurpose::Normal);
        }
    }

    fn set_surrounding_text(&self, text_input: &ZwpTextInputV3) {
        let text: String = self.text.iter().collect();
        let cursor_byte: i32 = self.text[..self.cursor]
            .iter()
            .map(|character| character.len_utf8())
            .sum::<usize>()
            .try_into()
            .unwrap_or(i32::MAX);
        text_input.set_surrounding_text(text, cursor_byte, cursor_byte);
    }

    fn apply_text_input_batch(&mut self) {
        if let Some((before, after)) = self.pending_delete.take() {
            let cursor_byte = self.text[..self.cursor]
                .iter()
                .map(|character| character.len_utf8())
                .sum::<usize>();
            let text: String = self.text.iter().collect();
            let start_byte = cursor_byte.saturating_sub(before as usize);
            let end_byte = cursor_byte.saturating_add(after as usize).min(text.len());
            if text.is_char_boundary(start_byte) && text.is_char_boundary(end_byte) {
                let start = text[..start_byte].chars().count();
                let end = text[..end_byte].chars().count();
                self.text.drain(start..end);
                self.cursor = start;
            }
        }
        if let Some(text) = self.pending_commit.take() {
            self.insert(&text);
        }
        if let Some(text_input) = self.text_input.clone() {
            self.set_surrounding_text(&text_input);
            text_input.commit();
        }
        self.write_state();
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

fn read_keymap(fd: OwnedFd, size: u32) -> Result<String> {
    let mut file = File::from(fd);
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    String::from_utf8(bytes).context("keymap is not UTF-8")
}

fn compile_keymap(text: &str) -> Result<xkb::Keymap> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_string(
        &context,
        text.to_owned(),
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
                state.seat = Some(registry.bind(name, version.min(7), qh, ()));
            }
            "zwp_text_input_manager_v3" => {
                state.text_input_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpTextInputManagerV3, ()> for Sink {
    fn event(
        _: &mut Self,
        _: &ZwpTextInputManagerV3,
        _: <ZwpTextInputManagerV3 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpTextInputV3, ()> for Sink {
    fn event(
        state: &mut Self,
        _: &ZwpTextInputV3,
        event: <ZwpTextInputV3 as wayland_client::Proxy>::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::Event;
        match event {
            Event::Enter { .. } => {
                state.text_input_active = true;
                state.write_state();
            }
            Event::Leave { .. } => {
                state.text_input_active = false;
                state.write_state();
            }
            Event::CommitString { text } => state.pending_commit = text,
            Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => state.pending_delete = Some((before_length, after_length)),
            Event::Done { .. } => state.apply_text_input_batch(),
            Event::PreeditString { .. } => {}
            _ => {}
        }
        let _ = connection.flush();
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
            let has_keyboard = capabilities.contains(wl_seat::Capability::Keyboard);
            if has_keyboard && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            } else if !has_keyboard {
                // Like a real client, rebind when a keyboard returns: on
                // headless Sway the seat loses its keyboard whenever the
                // last virtual keyboard goes (e.g. the daemon restarts),
                // and the old wl_keyboard stays inert.
                if let Some(keyboard) = state.keyboard.take() {
                    keyboard.release();
                }
                state.xkb = None;
                state.focused = false;
                state.write_state();
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
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, .. }
                if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) =>
            {
                // wlroots sends no_keymap for a virtual keyboard that has
                // not uploaded one yet.
                eprintln!("wx-e2e-sink: keyboard without a keymap");
                state.xkb = None;
            }
            wl_keyboard::Event::Keymap { fd, size, .. } => {
                let compiled = read_keymap(fd, size).and_then(|text| {
                    compile_keymap(&text).inspect_err(|_| {
                        // Keep what failed: real clients compile keymaps
                        // with the same libxkbcommon.
                        let _ =
                            std::fs::write(state.state_path.with_extension("bad-keymap"), &text);
                    })
                });
                match compiled {
                    Ok(keymap) => {
                        eprintln!("wx-e2e-sink: new keymap ({size} bytes)");
                        state.xkb = Some(xkb::State::new(&keymap));
                    }
                    Err(error) => {
                        // Never decode later keys with the previous keymap.
                        state.xkb = None;
                        eprintln!("wx-e2e-sink: {error:#}");
                    }
                }
            }
            wl_keyboard::Event::Enter { .. } => {
                state.focused = true;
                if state.text_input_enabled {
                    state.enable_text_input();
                    let _ = connection.flush();
                }
                state.write_state();
            }
            wl_keyboard::Event::Leave { .. } => {
                state.focused = false;
                if state.text_input_enabled {
                    if let Some(text_input) = state.text_input.as_ref() {
                        text_input.disable();
                        text_input.commit();
                        let _ = connection.flush();
                    }
                }
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
    let Some(app_id) = arguments.next() else {
        bail!("usage: wx-e2e-sink APP_ID STATE_FILE [--text-input] [--password]");
    };
    let Some(state_path) = arguments.next() else {
        bail!("usage: wx-e2e-sink APP_ID STATE_FILE [--text-input] [--password]");
    };
    let mut text_input_enabled = false;
    let mut password_field = false;
    for argument in arguments {
        match argument.as_str() {
            "--text-input" => text_input_enabled = true,
            "--password" => {
                text_input_enabled = true;
                password_field = true;
            }
            _ => bail!("unknown wx-e2e-sink option {argument:?}"),
        }
    }
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
        text_input_manager: None,
        seat: None,
        text_input: None,
        surface: None,
        buffer: None,
        keyboard: None,
        xkb: None,
        focused: false,
        text_input_enabled,
        password_field,
        text_input_active: false,
        pending_commit: None,
        pending_delete: None,
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
    if text_input_enabled {
        sink.start_text_input(&qh)?;
        connection.flush()?;
    }
    sink.write_state();
    while !sink.closed {
        queue.blocking_dispatch(&mut sink)?;
    }
    Ok(())
}
