//! Output backend for compositors exposing `zwp_virtual_keyboard_v1`.
//!
//! This is deliberately an output-only backend. It does not capture input;
//! pair it with a permitted input source such as the RemoteDesktop/libei
//! backend when building a complete daemon.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::fd::{AsFd, BorrowedFd},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
use wayexpand_core::{InjectorError, TextInjector};
use wayland_client::{
    protocol::{wl_callback, wl_keyboard, wl_registry, wl_seat::WlSeat},
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

const BACKEND_NAME: &str = "wlroots-virtual-keyboard";
// Reserved alongside the BackSpace keycode (8) in every generated keymap,
// for `{{cursor}}` placement after a replacement has already been typed.
const LEFT_KEYCODE: u32 = 9;
const MAX_OUTPUT_CHARS: usize = 8192;
const INITIAL_ROUNDTRIP_TIMEOUT: Duration = Duration::from_secs(5);
static KEYMAP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum WlrootsError {
    #[error("could not connect to Wayland: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    #[error("Wayland dispatch failed: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    #[error("compositor does not advertise {0}")]
    MissingGlobal(&'static str),
    #[error("could not create keymap: {0}")]
    Keymap(std::io::Error),
    #[error("text contains unsupported control character U+{0:04X}")]
    ControlCharacter(u32),
    #[error("text contains {length} characters; maximum is {maximum}")]
    TextTooLong { length: usize, maximum: usize },
    #[error("Wayland flush failed: {0}")]
    Flush(String),
    #[error("Wayland startup timed out: {0}")]
    Timeout(String),
    #[error("Wayland transport failed: {0}")]
    Transport(String),
}

impl WlrootsError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Connect(_)
                | Self::Dispatch(_)
                | Self::Flush(_)
                | Self::Timeout(_)
                | Self::Transport(_)
        )
    }
}

struct State {
    seat: Option<WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
    roundtrip_done: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
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
            "wl_seat" if state.seat.is_none() => {
                state.seat = Some(registry.bind(name, version.min(7), qh, ()))
            }
            "zwp_virtual_keyboard_manager_v1" if state.manager.is_none() => {
                state.manager = Some(registry.bind(name, version.min(1), qh, ()))
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.roundtrip_done = true;
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wayland_client::protocol::wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardManagerV1,
        _: wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardV1,
        _: wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

pub struct WlrootsInjector {
    connection: Connection,
    event_queue: EventQueue<State>,
    state: State,
    keyboard: ZwpVirtualKeyboardV1,
    keymap: Option<File>,
    mappings: HashMap<char, u32>,
    minimal_keymap: File,
}

impl WlrootsInjector {
    /// Probe globals without creating a virtual keyboard object.
    pub fn probe() -> Result<(), WlrootsError> {
        let connection = Connection::connect_to_env()?;
        let mut event_queue = connection.new_event_queue();
        let qh = event_queue.handle();
        let mut state = State {
            seat: None,
            manager: None,
            roundtrip_done: false,
        };
        connection.display().get_registry(&qh, ());
        roundtrip_with_timeout(
            &connection,
            &mut event_queue,
            &mut state,
            INITIAL_ROUNDTRIP_TIMEOUT,
        )?;
        state.seat.ok_or(WlrootsError::MissingGlobal("wl_seat"))?;
        state.manager.ok_or(WlrootsError::MissingGlobal(
            "zwp_virtual_keyboard_manager_v1",
        ))?;
        Ok(())
    }

    pub fn connect() -> Result<Self, WlrootsError> {
        let connection = Connection::connect_to_env()?;
        let mut event_queue = connection.new_event_queue();
        let qh = event_queue.handle();
        let mut state = State {
            seat: None,
            manager: None,
            roundtrip_done: false,
        };
        connection.display().get_registry(&qh, ());
        roundtrip_with_timeout(
            &connection,
            &mut event_queue,
            &mut state,
            INITIAL_ROUNDTRIP_TIMEOUT,
        )?;
        let seat = state
            .seat
            .clone()
            .ok_or(WlrootsError::MissingGlobal("wl_seat"))?;
        let manager = state.manager.clone().ok_or(WlrootsError::MissingGlobal(
            "zwp_virtual_keyboard_manager_v1",
        ))?;
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        connection
            .flush()
            .map_err(|error| WlrootsError::Flush(error.to_string()))?;
        let minimal_keymap = create_minimal_keymap()?;
        Ok(Self {
            connection,
            event_queue,
            state,
            keyboard,
            keymap: None,
            mappings: HashMap::new(),
            minimal_keymap,
        })
    }

    fn upload_keymap(&mut self, chars: impl Iterator<Item = char>) -> Result<(), WlrootsError> {
        let (keymap, mappings) = build_keymap(chars)?;
        self.mappings = mappings;
        let mut file = tempfile_keymap()?;
        file.write_all(keymap.as_bytes())
            .map_err(WlrootsError::Keymap)?;
        file.write_all(&[0]).map_err(WlrootsError::Keymap)?;
        file.flush().map_err(WlrootsError::Keymap)?;
        file.seek(SeekFrom::Start(0))
            .map_err(WlrootsError::Keymap)?;
        let size = file.metadata().map_err(WlrootsError::Keymap)?.len() as u32;
        self.keyboard
            .keymap(wl_keyboard::KeymapFormat::XkbV1.into(), file.as_fd(), size);
        self.keymap = Some(file);
        self.connection
            .flush()
            .map_err(|error| WlrootsError::Flush(error.to_string()))
    }

    fn send_minimal_keymap(&mut self) -> Result<(), WlrootsError> {
        self.mappings.clear();
        self.minimal_keymap
            .seek(SeekFrom::Start(0))
            .map_err(WlrootsError::Keymap)?;
        let size = self
            .minimal_keymap
            .metadata()
            .map_err(WlrootsError::Keymap)?
            .len() as u32;
        self.keyboard.keymap(
            wl_keyboard::KeymapFormat::XkbV1.into(),
            self.minimal_keymap.as_fd(),
            size,
        );
        self.keymap = None;
        self.connection
            .flush()
            .map_err(|error| WlrootsError::Flush(error.to_string()))
    }

    fn send_key_unflushed(&mut self, keycode: u32) {
        self.keyboard
            .key(0, keycode, wl_keyboard::KeyState::Pressed.into());
        self.keyboard
            .key(0, keycode, wl_keyboard::KeyState::Released.into());
    }

    fn send_keys<I>(&mut self, keycodes: I) -> Result<(), WlrootsError>
    where
        I: IntoIterator<Item = u32>,
    {
        for keycode in keycodes {
            self.send_key_unflushed(keycode);
        }
        self.connection
            .flush()
            .map_err(|error| WlrootsError::Flush(error.to_string()))?;
        self.event_queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }
}

fn roundtrip_with_timeout(
    connection: &Connection,
    event_queue: &mut EventQueue<State>,
    state: &mut State,
    timeout: Duration,
) -> Result<(), WlrootsError> {
    let qh = event_queue.handle();
    state.roundtrip_done = false;
    connection.display().sync(&qh, ());
    connection
        .flush()
        .map_err(|error| WlrootsError::Flush(error.to_string()))?;
    let deadline = Instant::now() + timeout;

    while !state.roundtrip_done {
        event_queue.dispatch_pending(state)?;
        if state.roundtrip_done {
            break;
        }
        let Some(guard) = connection.prepare_read() else {
            continue;
        };
        wait_for_readable(guard.connection_fd(), deadline)?;
        guard
            .read()
            .map_err(|error| WlrootsError::Transport(error.to_string()))?;
    }
    Ok(())
}

fn wait_for_readable(fd: BorrowedFd<'_>, deadline: Instant) -> Result<(), WlrootsError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(WlrootsError::Timeout(
            "Wayland registry roundtrip timed out".into(),
        ));
    }
    let timeout = rustix::event::Timespec {
        tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
        tv_nsec: remaining.subsec_nanos().into(),
    };
    let mut fds = [rustix::event::PollFd::new(
        &fd,
        rustix::event::PollFlags::IN
            | rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    )];
    if rustix::event::poll(&mut fds, Some(&timeout))
        .map_err(|error| WlrootsError::Transport(error.to_string()))?
        == 0
    {
        return Err(WlrootsError::Timeout(
            "Wayland registry roundtrip timed out".into(),
        ));
    }
    let revents = fds[0].revents();
    if revents.intersects(
        rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    ) {
        return Err(WlrootsError::Transport(format!(
            "Wayland connection became unavailable ({revents:?})"
        )));
    }
    Ok(())
}

fn tempfile_keymap() -> Result<File, WlrootsError> {
    let sequence = KEYMAP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "wayexpand-keymap-{}-{sequence}",
        std::process::id()
    ));
    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(&path)
        .map_err(WlrootsError::Keymap)?;
    let _ = std::fs::remove_file(path);
    Ok(file)
}

fn create_minimal_keymap() -> Result<File, WlrootsError> {
    let (keymap, _) = build_keymap(std::iter::empty())?;
    let mut file = tempfile_keymap()?;
    file.write_all(keymap.as_bytes())
        .map_err(WlrootsError::Keymap)?;
    file.write_all(&[0]).map_err(WlrootsError::Keymap)?;
    file.flush().map_err(WlrootsError::Keymap)?;
    file.seek(SeekFrom::Start(0))
        .map_err(WlrootsError::Keymap)?;
    Ok(file)
}

impl TextInjector for WlrootsInjector {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.send_minimal_keymap().map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })?;
        self.send_keys(std::iter::repeat_n(8, trigger.graphemes(true).count()))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.upload_keymap(text.chars())
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })?;
        let keycodes: Vec<_> = text
            .chars()
            .map(|character| self.mappings[&character])
            .collect();
        self.send_keys(keycodes).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        self.upload_keymap(text.chars())
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })?;
        let mut keycodes = vec![8; trigger.graphemes(true).count()];
        keycodes.extend(text.chars().map(|character| self.mappings[&character]));
        self.send_keys(keycodes).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        self.send_minimal_keymap().map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })?;
        self.send_keys(std::iter::repeat_n(LEFT_KEYCODE, count))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }
}

fn build_keymap(
    chars: impl Iterator<Item = char>,
) -> Result<(String, HashMap<char, u32>), WlrootsError> {
    let mut mappings = HashMap::new();
    let mut next_keycode = 10;
    let mut character_count = 0;
    let mut keycodes = String::from(
        "xkb_keymap {\n xkb_keycodes \"(unnamed)\" { minimum = 8; maximum = 255;\n <K8> = 8;\n <K9> = 9;\n",
    );
    let mut symbols = String::from(
        "xkb_symbols \"(unnamed)\" {\n key <K8> { [ BackSpace ] };\n key <K9> { [ Left ] };\n",
    );
    for character in chars {
        character_count += 1;
        if character_count > MAX_OUTPUT_CHARS {
            return Err(WlrootsError::TextTooLong {
                length: character_count,
                maximum: MAX_OUTPUT_CHARS,
            });
        }
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            return Err(WlrootsError::ControlCharacter(character as u32));
        }
        if mappings.contains_key(&character) {
            continue;
        }
        if next_keycode >= 255 {
            return Err(WlrootsError::Keymap(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "too many unique characters",
            )));
        }
        let keycode = next_keycode;
        next_keycode += 1;
        mappings.insert(character, keycode);
        keycodes.push_str(&format!(" <K{keycode}> = {keycode};\n"));
        let keysym = match character {
            '\n' | '\r' => "Return".to_string(),
            '\t' => "Tab".to_string(),
            character => format!("U{:X}", character as u32),
        };
        symbols.push_str(&format!(" key <K{keycode}> {{ [ {keysym} ] }};\n"));
    }
    keycodes.push_str(" };\n");
    symbols.push_str(" };\n xkb_types \"(unnamed)\" { include \"complete\" };\n xkb_compatibility \"(unnamed)\" { include \"complete\" };\n};\n");
    keycodes.push_str(&symbols);
    Ok((keycodes, mappings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        os::unix::net::UnixStream,
        time::{Duration, Instant},
    };

    #[test]
    fn keymap_contains_unicode_and_control_symbols() {
        let (keymap, mappings) = build_keymap("a🙂\n".chars()).unwrap();
        assert_eq!(mappings.len(), 3);
        assert!(keymap.contains("[ U61 ]"));
        assert!(keymap.contains("[ U1F642 ]"));
        assert!(keymap.contains("[ Return ]"));
        assert!(keymap.contains("[ BackSpace ]"));
        assert!(keymap.contains("[ Left ]"));
    }

    #[test]
    fn generated_keymap_is_accepted_by_xkbcommon() {
        let (keymap, _) = build_keymap("a🙂\n\t".chars()).unwrap();
        let compiled = xkbcommon_rs::Keymap::new_from_string(
            xkbcommon_rs::Context::new(0).unwrap(),
            &keymap,
            xkbcommon_rs::KeymapFormat::TextV1,
            0,
        );
        assert!(compiled.is_ok(), "generated XKB keymap must compile");
    }

    #[test]
    fn unsupported_controls_are_rejected() {
        assert!(matches!(
            build_keymap("\u{0001}".chars()),
            Err(WlrootsError::ControlCharacter(1))
        ));
    }

    #[test]
    fn oversized_output_is_rejected_before_keymap_generation() {
        assert!(matches!(
            build_keymap("a".repeat(MAX_OUTPUT_CHARS + 1).chars()),
            Err(WlrootsError::TextTooLong { .. })
        ));
    }

    #[test]
    fn startup_roundtrip_has_a_bounded_deadline() {
        assert_eq!(INITIAL_ROUNDTRIP_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn idle_roundtrip_waits_until_the_requested_deadline() {
        let (_server, client) = UnixStream::pair().unwrap();
        let timeout = Duration::from_millis(25);
        let started = Instant::now();
        let error = wait_for_readable(client.as_fd(), started + timeout).unwrap_err();

        assert!(matches!(error, WlrootsError::Timeout(_)));
        assert!(started.elapsed() >= Duration::from_millis(15));
    }

    #[test]
    fn disconnected_roundtrip_fails_fast_and_is_retryable() {
        let (mut server, client) = UnixStream::pair().unwrap();
        let server_thread = std::thread::spawn(move || {
            let mut request = [0_u8; 4096];
            let _ = server.read(&mut request);
        });
        let connection = Connection::from_socket(client).unwrap();
        let mut event_queue = connection.new_event_queue();
        let mut state = State {
            seat: None,
            manager: None,
            roundtrip_done: false,
        };
        let started = Instant::now();
        let error = roundtrip_with_timeout(
            &connection,
            &mut event_queue,
            &mut state,
            Duration::from_secs(1),
        )
        .unwrap_err();

        assert!(
            error.is_retryable(),
            "disconnected error must be retryable: {error:?}"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        drop(event_queue);
        drop(connection);
        server_thread.join().unwrap();
    }

    #[test]
    fn transport_failures_are_retryable_but_keymap_errors_are_not() {
        assert!(WlrootsError::Transport("closed".into()).is_retryable());
        assert!(WlrootsError::Timeout("startup".into()).is_retryable());
        assert!(!WlrootsError::ControlCharacter(1).is_retryable());
        assert!(!WlrootsError::MissingGlobal("wl_seat").is_retryable());
    }
}
