//! Output backend using the libei/EIS protocol.
//!
//! This backend prefers the `ei_text` interface, so insertion is UTF-8
//! rather than keyboard-layout-dependent key synthesis. When the EIS server
//! offers a keyboard device without `ei_text` (as of this writing, some
//! portal backends -- e.g. xdg-desktop-portal-kde on KWin 6.6 -- connect but
//! never resume a device with `ei_text`), this backend falls back to
//! synthesizing individual key presses over `ei_keyboard` instead. That
//! fallback is layout-dependent and strictly weaker: the EIS server, not
//! this client, owns the keyboard's keymap, so only characters already
//! reachable on the *current* layout via an unshifted or Shift-level keysym
//! can be typed. A character the layout cannot produce is reported as an
//! error before anything is typed, rather than silently dropped or
//! mistyped. It accepts a direct `LIBEI_SOCKET` or the XDG RemoteDesktop
//! portal, but portal access is only attempted when this backend is
//! explicitly selected.
//!
//! ## Performance note: ei_keyboard fallback latency
//!
//! When using the ei_keyboard fallback (no ei_text available), a 12ms delay
//! is inserted between synthetic key events. This is necessary for compositor
//! and toolkit compatibility: many desktop environments silently drop key
//! events delivered in rapid bursts, similar to how other synthetic-input
//! tools (`xdotool`, `wtype`, `ydotool`) behave. This results in O(N×12ms)
//! daemon thread blocking per expansion, where N is the number of characters.
//!
//! This tradeoff prioritizes correctness over speed: a slow expansion that
//! completes successfully is preferable to a fast one with dropped characters.
//! If latency is a concern:
//! - Prefer ei_text when available (no per-character delay)
//! - Consider enabling ei_text support in your EIS server if you control it
//! - Use the input-method-v2 backend as an alternative (if supported by your compositor)
//! - File an issue if your EIS server supports ei_text but doesn't resume devices with it

use reis::{ei, enumflags2::BitFlags, event::DeviceCapability};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek, SeekFrom, Write},
    os::fd::OwnedFd,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
use wayexpand_core::{InjectorError, TextInjector};
use xkbcommon_rs::{Context, Keymap as XkbKeymap, KeymapFormat};

const BACKEND_NAME: &str = "libei";
const KEY_BACKSPACE: u32 = 14;
// Linux evdev keycode for the Left arrow, used for `{{cursor}}` placement.
const KEY_LEFT: u32 = 105;
const EI_TEXT_MAX_UTF8_BYTES: usize = 254;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_KEYMAP_BYTES: u32 = 4 * 1024 * 1024;
const EIS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Pause between synthesized key events in the `ei_keyboard` fallback. Set
/// from the same ballpark as other synthetic-input tools (`xdotool`
/// defaults to 12ms); below roughly this, compositors and toolkits start
/// dropping keys out of a burst.
const KEY_EVENT_INTERVAL: Duration = Duration::from_millis(12);
// XKB keycodes carry the legacy X11 offset of 8 over the Linux evdev codes
// that `ei_keyboard.key()` expects (see the existing KEY_BACKSPACE handling
// below, which is already evdev-numbered).
const XKB_KEYCODE_OFFSET: u32 = 8;
const PORTAL_TOKEN_FILENAME: &str = "libei-portal-token";
const MAX_PORTAL_TOKEN_BYTES: usize = 4096;
const PORTAL_TOKEN_TEMP_ATTEMPTS: usize = 16;

#[derive(Debug, Error)]
pub enum LibeiError {
    #[error("relative LIBEI_SOCKET requires XDG_RUNTIME_DIR")]
    RelativeSocketNeedsRuntime,
    #[error("could not connect to EIS socket: {0}")]
    Connect(#[from] std::io::Error),
    #[error("portal connection failed: {0}")]
    Portal(String),
    #[error("libei handshake failed: {0}")]
    Handshake(#[from] reis::Error),
    #[error("libei server disconnected: {0}")]
    Disconnected(String),
    #[error("libei connection flush failed: {0}")]
    Flush(String),
    #[error("EIS server did not provide a device with ei_text or ei_keyboard")]
    MissingRequiredDevice,
    #[error("could not decode the EIS keyboard keymap: {0}")]
    Keymap(String),
    #[error(
        "character U+{0:04X} is not reachable on the current keyboard layout via the ei_keyboard \
         fallback (no ei_text interface was offered); the expansion was not typed"
    )]
    UnsupportedCharacter(u32),
    #[error("text is {length} bytes; maximum is {maximum}")]
    TextTooLarge { length: usize, maximum: usize },
    #[error("text contains unsupported control character U+{0:04X}")]
    ControlCharacter(u32),
}

impl LibeiError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Connect(error) => matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::AddrNotAvailable
                    | std::io::ErrorKind::BrokenPipe
            ),
            Self::Disconnected(_) | Self::Flush(_) => true,
            Self::Handshake(reis::Error::Io(_)) => true,
            _ => false,
        }
    }
}

struct PortalKeepalive {
    _proxy: ashpd::desktop::remote_desktop::RemoteDesktop<'static>,
    _session:
        ashpd::desktop::Session<'static, ashpd::desktop::remote_desktop::RemoteDesktop<'static>>,
    // Keep the Tokio reactor alive until the portal proxies have been dropped.
    _runtime: tokio::runtime::Runtime,
}

pub struct LibeiInjector {
    connection: reis::event::Connection,
    device: reis::event::Device,
    mode: TextMode,
    keyboard: ei::Keyboard,
    sequence: u32,
    started_at: Instant,
    return_keycode: u32,
    tab_keycode: u32,
    _portal: Option<PortalKeepalive>,
}

enum TextMode {
    /// Direct UTF-8 insertion. Layout-independent; used whenever the EIS
    /// server offers it.
    Text(ei::Text),
    /// Fallback for a server that only offers `ei_keyboard`: individual
    /// characters are looked up in the keymap the server itself sent and
    /// typed as key presses. Limited to whatever that layout can produce.
    Keysym(KeysymTyper),
}

/// Maps characters to a keycode (and whether Shift is needed) reachable on
/// the EIS server's own keymap, built once at connect time.
struct KeysymTyper {
    /// Evdev keycode of a Shift key, pressed around characters that need it.
    shift_keycode: u32,
    /// Evdev keycode for Return/Enter key (for newlines).
    return_keycode: u32,
    /// Evdev keycode for Tab key (for horizontal tabs).
    tab_keycode: u32,
    /// Linux evdev keycode, and whether the Shift level was needed to reach
    /// it, keyed by the character it produces.
    chars: HashMap<char, (u32, bool)>,
}

impl KeysymTyper {
    fn build(keymap: &XkbKeymap) -> Result<Self, LibeiError> {
        let shift_keycode = find_keycode_for_keysym(keymap, xkeysym::key::Shift_L)
            .or_else(|| find_keycode_for_keysym(keymap, xkeysym::key::Shift_R))
            .ok_or_else(|| LibeiError::Keymap("current keymap has no Shift key".into()))?;
        let return_keycode = find_keycode_for_keysym(keymap, xkeysym::key::Return)
            .ok_or_else(|| LibeiError::Keymap("current keymap has no Return key".into()))?;
        let tab_keycode = find_keycode_for_keysym(keymap, xkeysym::key::Tab)
            .ok_or_else(|| LibeiError::Keymap("current keymap has no Tab key".into()))?;
        let mut chars = HashMap::new();
        for &xkb_keycode in keymap.iter_keycodes() {
            let Some(evdev_keycode) = xkb_keycode.checked_sub(XKB_KEYCODE_OFFSET) else {
                continue;
            };
            for (level, shift) in [(0, false), (1, true)] {
                let Ok(syms) = keymap.key_get_syms_by_level(xkb_keycode, 0, level) else {
                    continue;
                };
                for sym in syms {
                    let Some(character) = sym.key_char() else {
                        continue;
                    };
                    // Prefer the lowest (first-found) level for a character,
                    // matching the simplest, most likely-correct chord.
                    chars.entry(character).or_insert((evdev_keycode, shift));
                }
            }
        }
        Ok(Self {
            shift_keycode,
            return_keycode,
            tab_keycode,
            chars,
        })
    }
}

fn find_keycode_for_keysym(keymap: &XkbKeymap, raw_keysym: xkeysym::RawKeysym) -> Option<u32> {
    let target = xkeysym::Keysym::new(raw_keysym);
    keymap.iter_keycodes().find_map(|&xkb_keycode| {
        let syms = keymap.key_get_syms_by_level(xkb_keycode, 0, 0).ok()?;
        syms.contains(&target)
            .then(|| xkb_keycode.checked_sub(XKB_KEYCODE_OFFSET))
            .flatten()
    })
}

fn decode_keymap(fd: OwnedFd, size: u32) -> Result<XkbKeymap, LibeiError> {
    if size == 0 || size > MAX_KEYMAP_BYTES {
        return Err(LibeiError::Keymap(format!(
            "invalid keymap size {size} bytes"
        )));
    }
    let mut file = File::from(fd);
    file.seek(SeekFrom::Start(0))
        .map_err(|error| LibeiError::Keymap(error.to_string()))?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| LibeiError::Keymap(error.to_string()))?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    let text = String::from_utf8(bytes).map_err(|error| LibeiError::Keymap(error.to_string()))?;
    XkbKeymap::new_from_string(
        Context::new(0).map_err(|error| LibeiError::Keymap(error.to_string()))?,
        &text,
        KeymapFormat::TextV1,
        0,
    )
    .map_err(|error| LibeiError::Keymap(error.to_string()))
}

struct EventPump {
    context: ei::Context,
    converter: reis::event::EiEventConverter,
}

impl EventPump {
    fn next(&mut self, timeout: Duration) -> Result<reis::event::EiEvent, LibeiError> {
        let deadline = Instant::now() + timeout;
        loop {
            while let Some(result) = self.context.pending_event() {
                match result {
                    reis::PendingRequestResult::Request(request) => self
                        .converter
                        .handle_event(request)
                        .map_err(|error| LibeiError::Handshake(error.into()))?,
                    reis::PendingRequestResult::ParseError(error) => {
                        return Err(LibeiError::Handshake(error.into()))
                    }
                    reis::PendingRequestResult::InvalidObject(object) => {
                        return Err(LibeiError::Handshake(
                            reis::handshake::HandshakeError::InvalidObject(object).into(),
                        ))
                    }
                }
            }
            if let Some(event) = self.converter.next_event() {
                return Ok(event);
            }
            if !poll_context(
                &self.context,
                deadline.saturating_duration_since(Instant::now()),
            )? {
                return Err(LibeiError::Handshake(reis::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "EIS event deadline expired",
                ))));
            }
            match self.context.read() {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(LibeiError::Disconnected("EIS socket closed".into()))
                }
                Err(error) => return Err(LibeiError::Handshake(error.into())),
            }
        }
    }
}

fn poll_context(context: &ei::Context, timeout: Duration) -> Result<bool, LibeiError> {
    let timeout = rustix::event::Timespec {
        tv_sec: timeout.as_secs().try_into().unwrap_or(i64::MAX),
        tv_nsec: timeout.subsec_nanos().into(),
    };
    let mut fds = [rustix::event::PollFd::new(
        context,
        rustix::event::PollFlags::IN
            | rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    )];
    if rustix::event::poll(&mut fds, Some(&timeout))
        .map_err(|error| LibeiError::Handshake(reis::Error::Io(error.into())))?
        == 0
    {
        return Ok(false);
    }
    let revents = fds[0].revents();
    if revents.intersects(
        rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    ) {
        return Err(LibeiError::Disconnected(format!(
            "EIS connection became unavailable ({revents:?})"
        )));
    }
    Ok(true)
}

fn handshake_with_timeout(
    context: &ei::Context,
    timeout: Duration,
) -> Result<(reis::event::Connection, EventPump), LibeiError> {
    let mut handshaker =
        reis::handshake::EiHandshaker::new("wayexpand", ei::handshake::ContextType::Sender);
    let deadline = Instant::now() + timeout;
    loop {
        while let Some(result) = context.pending_event() {
            let request = match result {
                reis::PendingRequestResult::Request(request) => request,
                reis::PendingRequestResult::ParseError(error) => {
                    return Err(LibeiError::Handshake(error.into()))
                }
                reis::PendingRequestResult::InvalidObject(object) => {
                    return Err(LibeiError::Handshake(
                        reis::handshake::HandshakeError::InvalidObject(object).into(),
                    ))
                }
            };
            if let Some(response) = handshaker
                .handle_event(request)
                .map_err(|error| LibeiError::Handshake(error.into()))?
            {
                let converter = reis::event::EiEventConverter::new(context, response);
                let connection = converter.connection().clone();
                return Ok((
                    connection,
                    EventPump {
                        context: context.clone(),
                        converter,
                    },
                ));
            }
        }
        if !poll_context(context, deadline.saturating_duration_since(Instant::now()))? {
            return Err(LibeiError::Handshake(reis::Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "EIS handshake deadline expired",
            ))));
        }
        context
            .read()
            .map_err(|error| LibeiError::Handshake(reis::Error::Io(error)))?;
    }
}

impl LibeiInjector {
    /// Connect to a direct EIS socket from `LIBEI_SOCKET`, or use the XDG
    /// RemoteDesktop portal when that variable is absent.
    ///
    /// Portal use is deliberately explicit because it may display a consent
    /// dialog and grants desktop input-control capability for the session.
    pub fn connect() -> Result<Self, LibeiError> {
        let (stream, portal) = if let Some(socket) = std::env::var_os("LIBEI_SOCKET") {
            let socket = PathBuf::from(socket);
            let socket = if socket.is_relative() {
                let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                    .ok_or(LibeiError::RelativeSocketNeedsRuntime)?;
                PathBuf::from(runtime).join(socket)
            } else {
                socket
            };
            (UnixStream::connect(socket)?, None)
        } else {
            connect_portal()?
        };
        // Handshake and event polling use explicit deadlines. Keep later
        // protocol flushes from blocking indefinitely when an EIS server
        // stops consuming input; WouldBlock is classified as retryable.
        stream.set_nonblocking(true)?;
        let context = ei::Context::new(stream)?;
        let (connection, mut events) = handshake_with_timeout(&context, EIS_HANDSHAKE_TIMEOUT)?;

        let device_deadline = Instant::now() + EIS_HANDSHAKE_TIMEOUT;
        let (device, mode, keyboard) = loop {
            let event = match events.next(device_deadline.saturating_duration_since(Instant::now()))
            {
                Ok(event) => event,
                Err(LibeiError::Handshake(reis::Error::Io(error)))
                    if error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Err(LibeiError::MissingRequiredDevice)
                }
                Err(error) => return Err(error),
            };
            match event {
                reis::event::EiEvent::SeatAdded(seat) => {
                    seat.seat.bind_capabilities(
                        BitFlags::from(DeviceCapability::Text)
                            | BitFlags::from(DeviceCapability::Keyboard),
                    );
                    connection
                        .flush()
                        .map_err(|error| LibeiError::Flush(error.to_string()))?;
                }
                reis::event::EiEvent::DeviceResumed(resumed) => {
                    let Some(keyboard) = resumed.device.interface::<ei::Keyboard>() else {
                        continue;
                    };
                    if let Some(text) = resumed.device.interface::<ei::Text>() {
                        break (resumed.device, TextMode::Text(text), keyboard);
                    }
                    // No ei_text on this device: fall back to keysym
                    // synthesis if the server also gave us a keymap to
                    // synthesize against. A keyboard device without either
                    // is not usable and keeps waiting for another device.
                    let Some(keymap) = resumed.device.keymap() else {
                        continue;
                    };
                    let keymap_fd = rustix::io::dup(&keymap.fd)
                        .map_err(|error| LibeiError::Keymap(error.to_string()))?;
                    let xkb_keymap = decode_keymap(keymap_fd, keymap.size)?;
                    let typer = KeysymTyper::build(&xkb_keymap)?;
                    break (resumed.device, TextMode::Keysym(typer), keyboard);
                }
                reis::event::EiEvent::Disconnected(disconnected) => {
                    return Err(LibeiError::Disconnected(
                        disconnected
                            .explanation
                            .unwrap_or_else(|| "no explanation".into()),
                    ));
                }
                _ => {}
            }
        };

        // Extract control keycodes from keymap if available (for Text mode to use)
        let (return_keycode, tab_keycode) = match &mode {
            TextMode::Keysym(typer) => (typer.return_keycode, typer.tab_keycode),
            TextMode::Text(_) => {
                // Try to get them from the device's keymap if available
                if let Some(keymap_data) = device.keymap() {
                    let keymap_fd = rustix::io::dup(&keymap_data.fd)
                        .map_err(|error| LibeiError::Keymap(error.to_string()))?;
                    let xkb_keymap = decode_keymap(keymap_fd, keymap_data.size)?;
                    let return_code = find_keycode_for_keysym(&xkb_keymap, xkeysym::key::Return)
                        .ok_or_else(|| LibeiError::Keymap("no Return key in keymap".into()))?;
                    let tab_code = find_keycode_for_keysym(&xkb_keymap, xkeysym::key::Tab)
                        .ok_or_else(|| LibeiError::Keymap("no Tab key in keymap".into()))?;
                    (return_code, tab_code)
                } else {
                    // Fallback: use standard Linux evdev keycodes
                    // KEY_RETURN = 28, KEY_TAB = 15
                    (28, 15)
                }
            }
        };

        Ok(Self {
            connection,
            device,
            mode,
            keyboard,
            sequence: 1,
            started_at: Instant::now(),
            return_keycode,
            tab_keycode,
            _portal: portal,
        })
    }

    /// Rejects a character the current mode cannot type before anything is
    /// sent, rather than typing part of a replacement and failing partway
    /// through it. `Text` mode can insert any validated UTF-8, so this only
    /// constrains `Keysym` mode, which is limited to the server's own
    /// keymap.
    fn ensure_representable(&self, text: &str) -> Result<(), LibeiError> {
        if let TextMode::Keysym(typer) = &self.mode {
            if let Some(character) = text.chars().find(|c| {
                // Newline and tab are handled specially in type_keys
                !matches!(c, '\n' | '\t') && !typer.chars.contains_key(c)
            }) {
                return Err(LibeiError::UnsupportedCharacter(character as u32));
            }
        }
        Ok(())
    }

    /// Queues `text` through the `ei_text` interface. Only valid in
    /// `TextMode::Text`; `TextMode::Keysym` types through `type_keys`
    /// instead, because synthesized key events have to be paced.
    fn send_text_unflushed(&mut self, text: &str) {
        let TextMode::Text(text_interface) = &self.mode else {
            return;
        };
        let text_interface = text_interface.clone();
        // Split on newlines and tabs since ei_text doesn't handle control characters.
        // We'll send printable text via ei_text and handle control chars via keyboard events.
        let mut current_text = String::new();
        for c in text.chars() {
            match c {
                '\n' | '\t' => {
                    // Send accumulated text first
                    if !current_text.is_empty() {
                        for chunk in split_text_chunks(&current_text) {
                            let serial = self.connection.serial();
                            self.device.device().start_emulating(serial, self.sequence);
                            self.sequence = self.sequence.checked_add(1).unwrap_or(1);
                            text_interface.utf8(chunk);
                            self.device
                                .device()
                                .frame(serial, self.started_at.elapsed().as_micros() as u64);
                            self.device.device().stop_emulating(serial);
                        }
                        current_text.clear();
                    }
                    // Send control character as keyboard event
                    // (Will be flushed and handled separately)
                    self.send_control_char(c);
                }
                _ => current_text.push(c),
            }
        }
        // Send any remaining text
        if !current_text.is_empty() {
            for chunk in split_text_chunks(&current_text) {
                let serial = self.connection.serial();
                self.device.device().start_emulating(serial, self.sequence);
                self.sequence = self.sequence.checked_add(1).unwrap_or(1);
                text_interface.utf8(chunk);
                self.device
                    .device()
                    .frame(serial, self.started_at.elapsed().as_micros() as u64);
                self.device.device().stop_emulating(serial);
            }
        }
    }

    fn send_control_char(&mut self, c: char) {
        let keycode = match c {
            '\n' => self.return_keycode,
            '\t' => self.tab_keycode,
            _ => return,
        };
        let serial = self.connection.serial();
        self.device.device().start_emulating(serial, self.sequence);
        self.sequence = self.sequence.checked_add(1).unwrap_or(1);
        self.keyboard.key(keycode, ei::keyboard::KeyState::Press);
        self.keyboard.key(keycode, ei::keyboard::KeyState::Released);
        self.device
            .device()
            .frame(serial, self.started_at.elapsed().as_micros() as u64);
        self.device.device().stop_emulating(serial);
    }

    /// Types `text` one character at a time over `ei_keyboard`, flushing and
    /// pausing between characters.
    ///
    /// The pacing is not incidental. A burst of synthesized key events
    /// delivered back-to-back is silently dropped in part by compositors and
    /// toolkits -- which is why every synthetic-input tool has an inter-key
    /// delay (`xdotool --delay`, which defaults to 12ms, `wtype -d`,
    /// `ydotool --key-delay`). Without it, a replacement loses a variable
    /// number of characters from wherever the receiving side stopped
    /// keeping up.
    ///
    /// This blocks the caller for `KEY_EVENT_INTERVAL` per character. That
    /// is a deliberate trade: a correct expansion that takes a moment beats
    /// an instant mangled one.
    fn type_keys(&mut self, text: &str) -> Result<(), LibeiError> {
        let TextMode::Keysym(typer) = &self.mode else {
            return Ok(());
        };
        let shift_keycode = typer.shift_keycode;
        let return_keycode = typer.return_keycode;
        let tab_keycode = typer.tab_keycode;

        let chars: Vec<char> = text.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            let (keycode, shift) = match c {
                '\n' => (return_keycode, false),
                '\t' => (tab_keycode, false),
                _ => typer.chars[c],
            };

            let serial = self.connection.serial();
            self.device.device().start_emulating(serial, self.sequence);
            self.sequence = self.sequence.checked_add(1).unwrap_or(1);
            if shift {
                self.keyboard
                    .key(shift_keycode, ei::keyboard::KeyState::Press);
            }
            self.keyboard.key(keycode, ei::keyboard::KeyState::Press);
            self.keyboard.key(keycode, ei::keyboard::KeyState::Released);
            if shift {
                self.keyboard
                    .key(shift_keycode, ei::keyboard::KeyState::Released);
            }
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.device.device().stop_emulating(serial);
            self.connection
                .flush()
                .map_err(|error| LibeiError::Flush(error.to_string()))?;
            if i < chars.len() - 1 {
                std::thread::sleep(KEY_EVENT_INTERVAL);
            }
        }
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<(), LibeiError> {
        validate_text(text)?;
        self.ensure_representable(text)?;
        if matches!(self.mode, TextMode::Keysym(_)) {
            return self.type_keys(text);
        }
        self.send_text_unflushed(text);
        if !text.is_empty() {
            self.connection
                .flush()
                .map_err(|error| LibeiError::Flush(error.to_string()))?;
        }
        Ok(())
    }

    fn send_backspaces_unflushed(&mut self, chars: usize) {
        let serial = self.connection.serial();
        for _ in 0..chars {
            self.device.device().start_emulating(serial, self.sequence);
            self.sequence = self.sequence.checked_add(1).unwrap_or(1);
            // EI keyboard keycodes are Linux evdev codes. KEY_BACKSPACE is 14.
            self.keyboard
                .key(KEY_BACKSPACE, ei::keyboard::KeyState::Press);
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.keyboard
                .key(KEY_BACKSPACE, ei::keyboard::KeyState::Released);
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.device.device().stop_emulating(serial);
        }
    }

    fn send_backspaces(&mut self, chars: usize) -> Result<(), LibeiError> {
        if chars == 0 {
            return Ok(());
        }
        self.send_backspaces_unflushed(chars);
        self.connection
            .flush()
            .map_err(|error| LibeiError::Flush(error.to_string()))
    }

    /// Sends `count` Left-arrow key presses, for `{{cursor}}` placement
    /// after a replacement has already been typed in full.
    fn send_left_arrows(&mut self, count: usize) -> Result<(), LibeiError> {
        if count == 0 {
            return Ok(());
        }
        let serial = self.connection.serial();
        for _ in 0..count {
            self.device.device().start_emulating(serial, self.sequence);
            self.sequence = self.sequence.checked_add(1).unwrap_or(1);
            self.keyboard.key(KEY_LEFT, ei::keyboard::KeyState::Press);
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.keyboard
                .key(KEY_LEFT, ei::keyboard::KeyState::Released);
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.device.device().stop_emulating(serial);
        }
        self.connection
            .flush()
            .map_err(|error| LibeiError::Flush(error.to_string()))
    }
}

fn split_text_chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = start;
        for (offset, character) in text[start..].char_indices() {
            let candidate = start + offset + character.len_utf8();
            if candidate - start > EI_TEXT_MAX_UTF8_BYTES {
                break;
            }
            end = candidate;
        }
        debug_assert!(end > start, "a Unicode scalar must fit in an EI text chunk");
        chunks.push(&text[start..end]);
        start = end;
    }
    chunks
}

/// Get the path where portal session tokens are stored.
/// Returns None if XDG_CONFIG_HOME is not set and home directory cannot be determined.
pub fn portal_token_path() -> Option<PathBuf> {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        let mut path = PathBuf::from(config_home);
        path.push("wayexpand");
        path.push(PORTAL_TOKEN_FILENAME);
        return Some(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        let mut path = PathBuf::from(home);
        path.push(".config/wayexpand");
        path.push(PORTAL_TOKEN_FILENAME);
        return Some(path);
    }
    None
}

pub fn reset_portal_token() -> std::io::Result<bool> {
    let path = match secure_portal_token_path(false) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    reset_portal_token_at(&path)
}

fn reset_portal_token_at(path: &Path) -> std::io::Result<bool> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no parent directory",
        )
    })?;
    validate_token_parent_chain(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_token_metadata(path, &metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Read a stored portal session token after validating both its parent and
/// the opened file descriptor. Invalid or unsafe token state is reported to
/// the caller instead of being silently treated as an absent token.
fn read_portal_token() -> std::io::Result<Option<String>> {
    let path = match secure_portal_token_path(false) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    read_portal_token_at(&path)
}

fn read_portal_token_at(path: &Path) -> std::io::Result<Option<String>> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no parent directory",
        )
    })?;
    validate_token_parent_chain(parent)?;
    let mut file = OpenOptions::new();
    file.read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    let file = match file.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    validate_token_metadata(path, &metadata)?;
    if metadata.len() > MAX_PORTAL_TOKEN_BYTES as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "portal restoration token is too large",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_PORTAL_TOKEN_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PORTAL_TOKEN_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "portal restoration token is too large",
        ));
    }
    let token = String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "portal restoration token is not valid UTF-8",
        )
    })?;
    if token.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "portal restoration token is empty",
        ));
    }
    Ok(Some(token))
}

/// Store a portal session token with restricted permissions (0600), using an
/// atomic durable replacement. The target is never truncated in place.
fn store_portal_token(token: &str) -> std::io::Result<()> {
    if token.is_empty() || token.len() > MAX_PORTAL_TOKEN_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal restoration token has an invalid size",
        ));
    }
    if token.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal restoration token contains NUL",
        ));
    }

    let path = secure_portal_token_path(true)?;
    store_portal_token_at(&path, token)
}

fn store_portal_token_at(path: &Path, token: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no parent directory",
        )
    })?;
    validate_token_parent_chain(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_token_metadata(path, &metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut temporary = None;
    let mut file = None;
    for attempt in 0..PORTAL_TOKEN_TEMP_ATTEMPTS {
        let candidate = parent.join(format!(
            ".{PORTAL_TOKEN_FILENAME}.tmp.{}.{}",
            std::process::id(),
            attempt
        ));
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        match options.open(&candidate) {
            Ok(created) => {
                temporary = Some(candidate);
                file = Some(created);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let temporary = temporary.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique portal token temporary file",
        )
    })?;
    let result = (|| -> std::io::Result<()> {
        let mut file = file.expect("temporary token file must exist");
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn secure_portal_token_path(create_parent: bool) -> std::io::Result<PathBuf> {
    let raw_path = portal_token_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cannot determine config directory for portal token",
        )
    })?;
    let raw_parent = raw_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no parent directory",
        )
    })?;
    let parent = trusted_token_parent(raw_parent, create_parent)?;
    let filename = raw_path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no file name",
        )
    })?;
    Ok(parent.join(filename))
}

fn trusted_token_parent(path: &Path, create: bool) -> std::io::Result<PathBuf> {
    if create {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "portal token parent is not a directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(path)?;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            }
            Err(error) => return Err(error),
        }
    }
    let resolved = fs::canonicalize(path)?;
    validate_token_parent_chain(&resolved)?;
    Ok(resolved)
}

fn validate_token_parent_chain(path: &Path) -> std::io::Result<()> {
    let current_uid = rustix::process::geteuid().as_raw();
    let mut current = path;
    loop {
        let metadata = fs::symlink_metadata(current)?;
        if !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "portal token parent is not a directory",
            ));
        }
        // Skip ownership check for system directories (/, /home, /run) where
        // containerization may cause unexpected UID ownership. User-owned
        // directories still validate strictly.
        let is_system_dir = current == Path::new("/")
            || current == Path::new("/home")
            || current == Path::new("/run");
        if !is_system_dir && metadata.uid() != current_uid && metadata.uid() != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "portal token parent has an untrusted owner",
            ));
        }
        // Only check sticky bit for non-root-owned directories; root-owned
        // directories like / are inherently safe even without sticky bit.
        if metadata.uid() != 0 {
            let mode = metadata.permissions().mode() & 0o7777;
            if mode & 0o022 != 0 && mode & 0o1000 == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "portal token parent is writable by group or other users",
                ));
            }
        }
        if current == Path::new("/") {
            break;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
    Ok(())
}

fn validate_token_metadata(path: &Path, metadata: &fs::Metadata) -> std::io::Result<()> {
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("portal token is not a regular file: {}", path.display()),
        ));
    }
    let current_uid = rustix::process::geteuid().as_raw();
    if metadata.uid() != current_uid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "portal token has an untrusted owner",
        ));
    }
    if metadata.permissions().mode() & 0o7777 != 0o600 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "portal token permissions must be 0600",
        ));
    }
    Ok(())
}

fn connect_portal() -> Result<(UnixStream, Option<PortalKeepalive>), LibeiError> {
    use ashpd::desktop::{
        remote_desktop::{DeviceType, RemoteDesktop},
        PersistMode,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| LibeiError::Portal(error.to_string()))?;
    let (stream, proxy, session) = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), async {
            let proxy: RemoteDesktop<'static> = RemoteDesktop::new()
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

            // Create a fresh session (restoration happens via select_devices token)
            let session = proxy
                .create_session()
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

            // Request explicitly-revoked persistent mode to enable session restoration tokens.
            // If we have a stored token, pass it to skip the consent dialog on reconnect.
            // The portal will restore permissions from the token if it's valid.
            let stored_token = read_portal_token().map_err(|error| {
                LibeiError::Portal(format!(
                    "could not securely read portal restoration token: {error}"
                ))
            })?;
            proxy
                .select_devices(
                    &session,
                    DeviceType::Keyboard.into(),
                    stored_token.as_deref(),
                    PersistMode::ExplicitlyRevoked,
                )
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

            let start_response = proxy
                .start(&session, None)
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?
                .response()
                .map_err(|error| LibeiError::Portal(error.to_string()))?;

            // Extract and store restoration token for next connection
            if let Some(token) = start_response.restore_token() {
                store_portal_token(token).map_err(|error| {
                    LibeiError::Portal(format!(
                        "could not securely store portal restoration token: {error}"
                    ))
                })?;
            }

            let fd = proxy
                .connect_to_eis(&session)
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;
            Ok::<_, LibeiError>((UnixStream::from(fd), proxy, session))
        })
        .await
        .map_err(|_| LibeiError::Portal("portal session timed out after 30 seconds".into()))?
    })?;

    Ok((
        stream,
        Some(PortalKeepalive {
            _proxy: proxy,
            _session: session,
            _runtime: runtime,
        }),
    ))
}

impl TextInjector for LibeiInjector {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.send_backspaces(trigger.graphemes(true).count())
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.send_text(text).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        validate_text(text).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })?;
        // Checked before erasing the trigger: if the replacement cannot be
        // typed, the trigger should not be removed either.
        self.ensure_representable(text)
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })?;
        self.send_backspaces_unflushed(trigger.graphemes(true).count());
        if matches!(self.mode, TextMode::Keysym(_)) {
            // Send the erase on its own and let it land before typing: in
            // keysym mode both halves are key events on the same device, so
            // batching them into one flush lets a late-applied backspace eat
            // a character that was already typed.
            self.connection.flush().map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: LibeiError::Flush(error.to_string()).to_string(),
                retryable: true,
            })?;
            std::thread::sleep(KEY_EVENT_INTERVAL);
            return self.type_keys(text).map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            });
        }
        self.send_text_unflushed(text);
        if !trigger.is_empty() || !text.is_empty() {
            self.connection.flush().map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: LibeiError::Flush(error.to_string()).to_string(),
                retryable: true,
            })?;
        }
        Ok(())
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        self.send_left_arrows(count).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })
    }
}

fn validate_text(text: &str) -> Result<(), LibeiError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(LibeiError::TextTooLarge {
            length: text.len(),
            maximum: MAX_TEXT_BYTES,
        });
    }
    if let Some(character) = text
        .chars()
        .find(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(LibeiError::ControlCharacter(character as u32));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::{fs::PermissionsExt, net::UnixStream},
        time::Duration,
    };

    #[test]
    fn backend_name_is_stable() {
        assert_eq!(super::BACKEND_NAME, "libei");
        assert_eq!(super::KEY_BACKSPACE, 14);
        assert_eq!(super::EIS_HANDSHAKE_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn event_poll_has_a_bounded_deadline() {
        let (client, _server) = UnixStream::pair().unwrap();
        let context = super::ei::Context::new(client).unwrap();
        assert!(!super::poll_context(&context, Duration::from_millis(10)).unwrap());
    }

    #[test]
    fn handshake_has_a_bounded_deadline() {
        let (client, _server) = UnixStream::pair().unwrap();
        let context = super::ei::Context::new(client).unwrap();
        let error = match super::handshake_with_timeout(&context, Duration::from_millis(10)) {
            Err(error) => error,
            Ok(_) => panic!("idle EIS endpoint must time out"),
        };
        assert!(matches!(error, super::LibeiError::Handshake(_)));
        assert!(error.to_string().contains("deadline expired"));
    }

    #[test]
    fn text_chunks_are_utf8_safe_and_protocol_sized() {
        let text = "🙂".repeat(200);
        let chunks = super::split_text_chunks(&text);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| {
            chunk.len() <= super::EI_TEXT_MAX_UTF8_BYTES
                && std::str::from_utf8(chunk.as_bytes()).is_ok()
        }));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn empty_text_has_no_protocol_chunk() {
        assert!(super::split_text_chunks("").is_empty());
    }

    fn token_test_parent(name: &str) -> std::path::PathBuf {
        let parent = std::env::temp_dir().join(format!(
            "wayexpand-libei-token-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        parent
    }

    #[test]
    fn portal_token_is_readable_only_after_private_atomic_save() {
        let parent = token_test_parent("secure");
        let path = parent.join(super::PORTAL_TOKEN_FILENAME);
        super::store_portal_token_at(&path, "restoration-token").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            super::read_portal_token_at(&path).unwrap().as_deref(),
            Some("restoration-token")
        );
        assert!(super::reset_portal_token_at(&path).unwrap());
        assert!(!path.exists());
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn portal_token_rejects_symlinks_and_insecure_parents() {
        let parent = token_test_parent("unsafe");
        let path = parent.join(super::PORTAL_TOKEN_FILENAME);
        let target = parent.join("target");
        fs::write(&target, "do not replace").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(super::store_portal_token_at(&path, "new-token").is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "do not replace");
        fs::remove_file(&path).unwrap();

        fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(super::store_portal_token_at(&path, "new-token").is_err());
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn portal_token_read_rejects_oversized_or_wrongly_permissioned_files() {
        let parent = token_test_parent("validation");
        let path = parent.join(super::PORTAL_TOKEN_FILENAME);
        fs::write(&path, "token").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(super::read_portal_token_at(&path).is_err());

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, "x".repeat(super::MAX_PORTAL_TOKEN_BYTES + 1)).unwrap();
        assert!(super::read_portal_token_at(&path).is_err());
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn text_size_is_bounded_before_injection() {
        assert!(super::validate_text(&"a".repeat(super::MAX_TEXT_BYTES)).is_ok());
        assert!(matches!(
            super::validate_text(&"a".repeat(super::MAX_TEXT_BYTES + 1)),
            Err(super::LibeiError::TextTooLarge { .. })
        ));
    }

    #[test]
    fn control_characters_are_rejected_before_injection() {
        assert!(matches!(
            super::validate_text("\u{0001}"),
            Err(super::LibeiError::ControlCharacter(1))
        ));
        assert!(super::validate_text("safe\ntext\r\t").is_ok());
    }

    #[test]
    fn transport_failures_are_retryable_but_validation_is_not() {
        assert!(super::LibeiError::Connect(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "not ready",
        ))
        .is_retryable());
        assert!(!super::LibeiError::Connect(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "not allowed",
        ))
        .is_retryable());
        assert!(super::LibeiError::Disconnected("closed".into()).is_retryable());
        assert!(super::LibeiError::Flush("closed".into()).is_retryable());
        assert!(
            super::LibeiError::Handshake(reis::Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timeout"
            ),))
            .is_retryable()
        );
        assert!(!super::LibeiError::Handshake(reis::Error::Handshake(
            reis::handshake::HandshakeError::MissingInterface,
        ))
        .is_retryable());
        assert!(!super::LibeiError::MissingRequiredDevice.is_retryable());
        assert!(!super::LibeiError::Portal("permission denied".into()).is_retryable());
        assert!(!super::LibeiError::TextTooLarge {
            length: 2,
            maximum: 1,
        }
        .is_retryable());
        assert!(!super::LibeiError::ControlCharacter(1).is_retryable());
        assert!(!super::LibeiError::Keymap("bad keymap".into()).is_retryable());
        assert!(!super::LibeiError::UnsupportedCharacter('a' as u32).is_retryable());
    }

    fn default_keymap() -> super::XkbKeymap {
        super::XkbKeymap::new_from_names(super::Context::new(0).unwrap(), None, 0).unwrap()
    }

    #[test]
    fn keysym_typer_maps_shifted_and_unshifted_letters_to_the_same_key() {
        let keymap = default_keymap();
        let typer = super::KeysymTyper::build(&keymap).unwrap();
        let (lower_keycode, lower_shift) = typer.chars[&'a'];
        let (upper_keycode, upper_shift) = typer.chars[&'A'];
        assert!(!lower_shift);
        assert!(upper_shift);
        assert_eq!(
            lower_keycode, upper_keycode,
            "'a' and 'A' are the same physical key, differing only by Shift"
        );
        assert_ne!(
            upper_keycode, typer.shift_keycode,
            "Shift itself must not be reported as a typeable character's key"
        );
    }

    #[test]
    fn keysym_typer_does_not_claim_unreachable_characters() {
        let keymap = default_keymap();
        let typer = super::KeysymTyper::build(&keymap).unwrap();
        // No ordinary keyboard layout has a direct, unshifted/Shift-level
        // keysym for CJK ideographs -- those need an input method, which the
        // fallback deliberately cannot provide (see the module docs).
        assert!(!typer.chars.contains_key(&'中'));
    }
}
