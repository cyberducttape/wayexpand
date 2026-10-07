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
//! reachable on the *current* layout -- unmodified or with the layout's own
//! Shift, AltGr (ISO Level3) and Level5 keys -- can be typed. A character the layout cannot produce is reported as an
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
//! tools (`xdotool`, `wtype`, `ydotool`) behave. Direct library callers spend
//! O(N×12ms) in the injector per expansion, where N is the number of
//! characters. The daemon's non-exclusive evdev path places this injector
//! behind a bounded serialized output actor, so pacing does not block physical
//! input capture or matching.
//!
//! This tradeoff prioritizes correctness over speed for bounded replacements.
//! Very large keysym fallbacks are refused before erasing the trigger rather
//! than occupying the daemon's synchronous keyboard path for several seconds.
//! If latency is a concern:
//! - Prefer ei_text when available (no per-character delay)
//! - Consider enabling ei_text support in your EIS server if you control it
//! - Use the input-method-v2 backend as an alternative (if supported by your compositor)
//! - File an issue if your EIS server supports ei_text but doesn't resume devices with it

use reis::{ei, enumflags2::BitFlags, event::DeviceCapability};
use std::{
    os::unix::net::UnixStream,
    path::PathBuf,
    time::{Duration, Instant},
};
use thiserror::Error;
use wayexpand_core::{InjectorCapabilities, InjectorError, KeyEventState, Modifiers, TextInjector};
#[cfg(test)]
use xkbcommon_rs::{Context, Keymap as XkbKeymap};

mod connection;
mod events;
mod keymap;
mod portal_token;
mod unicode;

use connection::{connect_portal, PortalKeepalive};
use events::{handshake_with_timeout, EventPump};
use keymap::{decode_keymap, find_keycode_for_keysym, KeyStroke, KeysymTyper};
#[cfg(test)]
use keymap::{MOD_LEVEL3, MOD_SHIFT};
pub use portal_token::{portal_token_path, reset_portal_token};
#[cfg(test)]
use portal_token::{
    read_portal_token_at, reset_portal_token_at, store_portal_token_at,
    token_parent_mode_is_secure, validate_token_parent_chain, MAX_PORTAL_TOKEN_BYTES,
    PORTAL_TOKEN_FILENAME,
};
use portal_token::{read_portal_token_if_enabled, store_portal_token_if_enabled};
use unicode::{erase_grapheme_count, split_text_chunks, validate_text};

const BACKEND_NAME: &str = "libei";
const KEY_BACKSPACE: u32 = 14;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTALT: u32 = 56;
// Linux evdev keycode for the Left arrow, used for `{{cursor}}` placement.
const KEY_LEFT: u32 = 105;
const KEY_LEFTMETA: u32 = 125;
const EI_TEXT_MAX_UTF8_BYTES: usize = 254;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_KEYMAP_BYTES: u32 = 4 * 1024 * 1024;
const EIS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Pause between synthesized key events in the `ei_keyboard` fallback. Set
/// from the same ballpark as other synthetic-input tools (`xdotool`
/// defaults to 12ms); below roughly this, compositors and toolkits start
/// dropping keys out of a burst.
const KEY_EVENT_INTERVAL: Duration = Duration::from_millis(12);
/// Refuse a keysym fallback that would occupy the synchronous injection path
/// for roughly three seconds or more. Native `ei_text` is unaffected.
const MAX_KEYSYM_FALLBACK_CHARS: usize = 250;

// XKB keycodes carry the legacy X11 offset of 8 over the Linux evdev codes
// that `ei_keyboard.key()` expects (see the existing KEY_BACKSPACE handling
// below, which is already evdev-numbered).
const XKB_KEYCODE_OFFSET: u32 = 8;

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
    #[error("the EIS keyboard device is unavailable ({0}); the expansion was not typed")]
    DeviceUnavailable(&'static str),
    #[error(
        "the EIS keymap has {0} layouts but the server has not reported which one is active; \
         the ei_keyboard fallback will not guess, so the expansion was not typed"
    )]
    UnknownActiveLayout(usize),
    #[error(
        "character U+{0:04X} is not reachable on the current keyboard layout via the ei_keyboard \
         fallback (no ei_text interface was offered); the expansion was not typed"
    )]
    UnsupportedCharacter(u32),
    #[error("text is {length} bytes; maximum is {maximum}")]
    TextTooLarge { length: usize, maximum: usize },
    #[error("text contains unsupported control character U+{0:04X}")]
    ControlCharacter(u32),
    #[error(
        "keysym fallback for {characters} characters would take about {estimated_ms} ms; \
         use an ei_text-capable backend or shorten the replacement"
    )]
    FallbackTooSlow {
        characters: usize,
        estimated_ms: u64,
    },
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
            // A removed or paused device is recovered by reconnecting, which
            // reads the server's current device and keymap.
            Self::Disconnected(_) | Self::Flush(_) | Self::DeviceUnavailable(_) => true,
            Self::Handshake(reis::Error::Io(_)) => true,
            _ => false,
        }
    }
}

pub struct LibeiInjector {
    connection: reis::event::Connection,
    /// Kept after the handshake so server notifications (modifier/layout
    /// changes, device pause/removal) are seen before anything is typed.
    events: EventPump,
    /// Set while the server has paused or removed our device.
    device_unavailable: Option<&'static str>,
    device: reis::event::Device,
    mode: TextMode,
    keyboard: ei::Keyboard,
    sequence: u32,
    started_at: Instant,
    return_keycode: u32,
    tab_keycode: u32,
    _portal: Option<PortalKeepalive>,
}

/// Controls how the RemoteDesktop portal session is restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibeiOptions {
    pub persist_portal_token: bool,
    /// Token path selected by the service. `None` keeps the standalone XDG
    /// default used by CLI/library callers.
    pub portal_token_path: Option<PathBuf>,
}

impl Default for LibeiOptions {
    fn default() -> Self {
        Self {
            persist_portal_token: true,
            portal_token_path: None,
        }
    }
}

enum TextMode {
    /// Direct UTF-8 insertion. Layout-independent; used whenever the EIS
    /// server offers it.
    Text(ei::Text),
    /// Fallback for a server that only offers `ei_keyboard`: individual
    /// characters are looked up in the keymap the server itself sent and
    /// typed as key presses. Limited to whatever that layout can produce.
    Keysym(Box<KeysymTyper>),
}

impl TextMode {
    fn status_detail(&self) -> &'static str {
        match self {
            Self::Text(_) => "ei_text (UTF-8 insertion)",
            Self::Keysym(_) => "ei_keyboard keysym fallback (12ms key pacing)",
        }
    }
}

impl LibeiInjector {
    /// Connect to a direct EIS socket from `LIBEI_SOCKET`, or use the XDG
    /// RemoteDesktop portal when that variable is absent.
    ///
    /// Portal use is deliberately explicit because it may display a consent
    /// dialog and grants desktop input-control capability for the session.
    pub fn connect(options: LibeiOptions) -> Result<Self, LibeiError> {
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
            connect_portal(options)?
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
                    break (resumed.device, TextMode::Keysym(Box::new(typer)), keyboard);
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
            events,
            device_unavailable: None,
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

    /// Apply everything the server reported since the last call, then make
    /// sure the keyboard state still matches what we would type against.
    /// Called before any text-producing request: a layout switch, Caps Lock,
    /// or a replaced device must never be typed through with stale
    /// assumptions, so this rebuilds the keysym map or refuses (fail closed).
    fn sync_server_state(&mut self) -> Result<(), LibeiError> {
        for event in self.events.drain()? {
            match event {
                reis::event::EiEvent::KeyboardModifiers(modifiers)
                    if modifiers.device == self.device =>
                {
                    if let TextMode::Keysym(typer) = &mut self.mode {
                        typer.note_modifiers(modifiers.group, modifiers.locked);
                    }
                }
                reis::event::EiEvent::DevicePaused(paused) if paused.device == self.device => {
                    self.device_unavailable = Some("paused by the server");
                }
                reis::event::EiEvent::DeviceResumed(resumed) if resumed.device == self.device => {
                    if self.device_unavailable == Some("paused by the server") {
                        self.device_unavailable = None;
                    }
                }
                reis::event::EiEvent::DeviceRemoved(removed) if removed.device == self.device => {
                    // A keymap change replaces the device; its old keymap
                    // must not be used again.
                    self.device_unavailable = Some("removed by the server");
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
        }
        if let Some(reason) = self.device_unavailable {
            return Err(LibeiError::DeviceUnavailable(reason));
        }
        if let TextMode::Keysym(typer) = &mut self.mode {
            typer.refresh()?;
        }
        Ok(())
    }

    /// Rejects a character the current mode cannot type before anything is
    /// sent, rather than typing part of a replacement and failing partway
    /// through it. `Text` mode can insert any validated UTF-8, so this only
    /// constrains `Keysym` mode, which is limited to the server's own
    /// keymap.
    fn ensure_representable(&self, text: &str) -> Result<(), LibeiError> {
        if let TextMode::Keysym(typer) = &self.mode {
            let characters = text.chars().count();
            if characters > MAX_KEYSYM_FALLBACK_CHARS {
                return Err(LibeiError::FallbackTooSlow {
                    characters,
                    estimated_ms: (characters.saturating_sub(1) as u64)
                        .saturating_mul(KEY_EVENT_INTERVAL.as_millis() as u64),
                });
            }
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
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            let stroke = match c {
                '\n' => KeyStroke {
                    keycode: typer.return_keycode,
                    modifiers: 0,
                },
                '\t' => KeyStroke {
                    keycode: typer.tab_keycode,
                    modifiers: 0,
                },
                _ => typer.chars[&c],
            };

            let serial = self.connection.serial();
            self.device.device().start_emulating(serial, self.sequence);
            self.sequence = self.sequence.checked_add(1).unwrap_or(1);
            for modifier in typer.modifiers_for(stroke) {
                self.keyboard.key(modifier, ei::keyboard::KeyState::Press);
            }
            self.keyboard
                .key(stroke.keycode, ei::keyboard::KeyState::Press);
            self.keyboard
                .key(stroke.keycode, ei::keyboard::KeyState::Released);
            for modifier in typer.modifiers_for(stroke).rev() {
                self.keyboard
                    .key(modifier, ei::keyboard::KeyState::Released);
            }
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.device.device().stop_emulating(serial);
            self.connection
                .flush()
                .map_err(|error| LibeiError::Flush(error.to_string()))?;
            if chars.peek().is_some() {
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

    fn send_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), LibeiError> {
        let serial = self.connection.serial();
        self.device.device().start_emulating(serial, self.sequence);
        self.sequence = self.sequence.checked_add(1).unwrap_or(1);
        for modifier in modifier_keycodes(modifiers) {
            self.keyboard.key(modifier, ei::keyboard::KeyState::Press);
        }
        self.keyboard.key(keycode, ei::keyboard::KeyState::Press);
        self.device
            .device()
            .frame(serial, self.started_at.elapsed().as_micros() as u64);
        self.keyboard.key(keycode, ei::keyboard::KeyState::Released);
        for modifier in modifier_keycodes(modifiers).into_iter().rev() {
            self.keyboard
                .key(modifier, ei::keyboard::KeyState::Released);
        }
        self.device
            .device()
            .frame(serial, self.started_at.elapsed().as_micros() as u64);
        self.device.device().stop_emulating(serial);
        self.connection
            .flush()
            .map_err(|error| LibeiError::Flush(error.to_string()))
    }
}

fn modifier_keycodes(modifiers: Modifiers) -> Vec<u32> {
    let mut keycodes = Vec::with_capacity(4);
    if modifiers.ctrl {
        keycodes.push(KEY_LEFTCTRL);
    }
    if modifiers.alt {
        keycodes.push(KEY_LEFTALT);
    }
    if modifiers.shift {
        keycodes.push(KEY_LEFTSHIFT);
    }
    if modifiers.super_key {
        keycodes.push(KEY_LEFTMETA);
    }
    keycodes
}

impl TextInjector for LibeiInjector {
    fn shutdown(mut self: Box<Self>) {
        // Remove the portal keepalive before dropping the injector so its
        // session can be closed while the Tokio runtime is still available.
        let portal = self._portal.take();
        drop(self);
        if let Some(portal) = portal {
            portal.close();
        }
    }

    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn capabilities(&self) -> InjectorCapabilities {
        let keysym_fallback = matches!(self.mode, TextMode::Keysym(_));
        InjectorCapabilities {
            insertion_mode: if keysym_fallback {
                "libei keysym fallback"
            } else {
                "ei_text"
            },
            max_text_chars: if keysym_fallback {
                MAX_KEYSYM_FALLBACK_CHARS
            } else {
                0
            },
            expected_throughput_chars_per_sec: keysym_fallback.then_some(83),
            // Even a single ei_text flush can fail after the target has
            // processed part of the transaction; libei has no rollback
            // primitive for arbitrary application text.
            atomic_replace: false,
            // Backspaces are sent without seeing the target's text.
            replacement_guarantee: wayexpand_core::ReplacementGuarantee::BestEffort,
            full_unicode: matches!(self.mode, TextMode::Text(_)),
            cursor_reposition: true,
            key_passthrough: true,
        }
    }

    fn status_detail(&self) -> &'static str {
        self.mode.status_detail()
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.sync_server_state()
            .and_then(|()| self.send_backspaces(erase_grapheme_count(trigger)))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.sync_server_state()
            .and_then(|()| self.send_text(text))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        // Synchronized before the representability check below, so that
        // check (and the trigger erase it guards) uses the current layout.
        self.sync_server_state().map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })?;
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
        self.send_backspaces_unflushed(erase_grapheme_count(trigger));
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
        self.sync_server_state()
            .and_then(|()| self.send_left_arrows(count))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    fn inject_key(&mut self, keycode: u32) -> Result<(), InjectorError> {
        self.send_key_with_modifiers(keycode, Modifiers::default())
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        self.send_key_with_modifiers(keycode, modifiers)
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    fn inject_key_event(
        &mut self,
        keycode: u32,
        _modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        let serial = self.connection.serial();
        self.device.device().start_emulating(serial, self.sequence);
        self.sequence = self.sequence.checked_add(1).unwrap_or(1);
        self.keyboard.key(
            keycode,
            match state {
                KeyEventState::Pressed => ei::keyboard::KeyState::Press,
                KeyEventState::Released => ei::keyboard::KeyState::Released,
            },
        );
        self.device
            .device()
            .frame(serial, self.started_at.elapsed().as_micros() as u64);
        self.device.device().stop_emulating(serial);
        self.connection.flush().map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: LibeiError::Flush(error.to_string()).to_string(),
            retryable: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::{fs::PermissionsExt, net::UnixStream},
        time::Duration,
    };

    #[test]
    fn erase_count_treats_decomposed_accent_and_devanagari_as_one() {
        for grapheme in ["e\u{301}", "ü", "u\u{308}", "👍🏽", "👩‍💻", "क्ष", "क्‍ष"]
        {
            assert_eq!(super::erase_grapheme_count(grapheme), 1, "{grapheme:?}");
        }
    }

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
        assert!(!super::events::poll_context(&context, Duration::from_millis(10)).unwrap());
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
        // The token store rejects any group/other-writable ancestor, so the
        // test base must live under a trusted chain. That depends on where
        // the tests run (a sandbox temp directory, or a Debian build tree
        // unpacked under a 0775 directory), so use the first candidate whose
        // whole chain the real validation accepts.
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_dir = manifest_dir.parent().unwrap().parent().unwrap();
        let candidates = [std::env::temp_dir(), workspace_dir.join("target")];
        let base = candidates
            .iter()
            .filter_map(|root| fs::canonicalize(root).ok())
            .find(|root| super::validate_token_parent_chain(root).is_ok())
            .expect("no trusted directory chain for portal token tests")
            .join(format!(
                "wayexpand-libei-token-tests-{}",
                rustix::process::geteuid().as_raw()
            ));
        fs::create_dir_all(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let parent = base.join(format!(
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
    fn portal_token_parent_mode_security_does_not_depend_on_owner() {
        assert!(super::token_parent_mode_is_secure(0o700));
        assert!(super::token_parent_mode_is_secure(0o755));
        assert!(super::token_parent_mode_is_secure(0o1777));
        assert!(!super::token_parent_mode_is_secure(0o777));
        assert!(!super::token_parent_mode_is_secure(0o775));
    }

    #[test]
    fn portal_token_rejects_root_owned_writable_parent_when_possible() {
        if rustix::process::geteuid().as_raw() != 0 {
            return;
        }

        let parent = token_test_parent("root-owned-unsafe");
        let path = parent.join(super::PORTAL_TOKEN_FILENAME);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();

        let error = super::store_portal_token_at(&path, "new-token").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);

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
    fn disabled_portal_token_persistence_neither_reads_nor_writes() {
        let parent = token_test_parent("disabled");
        let path = parent.join(super::PORTAL_TOKEN_FILENAME);
        fs::write(&path, "existing-token").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(
            super::read_portal_token_if_enabled(false, Some(&path)).unwrap(),
            None,
            "disabled persistence must not read the existing token"
        );
        super::store_portal_token_if_enabled(false, Some(&path), "replacement-token").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "existing-token");

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn explicit_portal_token_path_is_used_for_persistence() {
        let parent = token_test_parent("explicit-path");
        let path = parent
            .join("custom-config")
            .join(super::PORTAL_TOKEN_FILENAME);

        super::store_portal_token_if_enabled(true, Some(&path), "custom-token").unwrap();
        assert_eq!(
            super::read_portal_token_if_enabled(true, Some(&path))
                .unwrap()
                .as_deref(),
            Some("custom-token")
        );

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn portal_token_survives_config_dir_writable_in_read_only_home() {
        // Verify that libei can store and retrieve tokens in a writable config
        // directory. This tests the systemd hardening scenario: ProtectHome=read-only
        // with ReadWritePaths=%h/.config/wayexpand allows token persistence after
        // restart. The security validation checks the entire parent chain; all
        // intermediate directories must be owned by the current user and not
        // writable by group/other (no 0o022 bits).
        let parent = token_test_parent("systemd-hardening");
        let config_base = parent.join(".config");
        let config_dir = config_base.join("wayexpand");
        fs::create_dir_all(&config_dir).unwrap();
        fs::set_permissions(&config_base, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let token_path = config_dir.join(super::PORTAL_TOKEN_FILENAME);
        let test_token = "test-restoration-token";

        super::store_portal_token_at(&token_path, test_token).unwrap();
        assert!(token_path.exists());
        assert_eq!(
            fs::metadata(&token_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // Retrieve token and verify it matches
        let retrieved = super::read_portal_token_at(&token_path).unwrap();
        assert_eq!(retrieved.as_deref(), Some(test_token));

        // Clean up
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
        let lower = typer.chars[&'a'];
        let upper = typer.chars[&'A'];
        assert_eq!(lower.modifiers, 0);
        assert_eq!(upper.modifiers, super::MOD_SHIFT);
        assert_eq!(
            lower.keycode, upper.keycode,
            "'a' and 'A' are the same physical key, differing only by Shift"
        );
        assert_ne!(
            Some(upper.keycode),
            typer.modifier_keycodes[0],
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

    fn layout_keymap(layout: &str) -> super::XkbKeymap {
        super::XkbKeymap::new_from_names(
            super::Context::new(0).unwrap(),
            Some(xkbcommon_rs::xkb_keymap::RuleNames {
                rules: None,
                model: None,
                layout: Some(layout.into()),
                variant: None,
                options: None,
            }),
            0,
        )
        .unwrap()
    }

    #[test]
    fn keysym_typer_reaches_altgr_characters_on_a_german_layout() {
        let keymap = layout_keymap("de");
        let typer = super::KeysymTyper::build(&keymap).unwrap();
        let q = typer.chars[&'q'];
        let at = typer.chars[&'@'];
        assert_eq!(q.modifiers, 0);
        assert_eq!(
            at,
            super::KeyStroke {
                keycode: q.keycode,
                modifiers: super::MOD_LEVEL3,
            },
            "'@' is AltGr+Q on a German layout"
        );
        assert!(typer.chars.contains_key(&'€'));
        assert_eq!(typer.chars[&'{'].modifiers, super::MOD_LEVEL3);
        assert_eq!(typer.chars[&'ä'].modifiers, 0);
        assert_eq!(typer.chars[&'Ä'].modifiers, super::MOD_SHIFT);
        let level3 = typer.modifier_keycodes[1].expect("German layout defines AltGr");
        assert_eq!(
            typer.modifiers_for(at).collect::<Vec<_>>(),
            vec![level3],
            "only the AltGr key is held for '@'"
        );
    }

    #[test]
    fn keysym_typer_prefers_the_simplest_chord() {
        let keymap = layout_keymap("de");
        let typer = super::KeysymTyper::build(&keymap).unwrap();
        // Digits exist unmodified; they must not be mapped to a modified
        // chord on some other key.
        assert_eq!(typer.chars[&'1'].modifiers, 0);
        assert_eq!(typer.chars[&'!'].modifiers, super::MOD_SHIFT);
    }

    #[test]
    fn keysym_typer_refuses_to_guess_the_active_layout_of_a_multi_layout_keymap() {
        let keymap = layout_keymap("us,de");
        let mut typer = super::KeysymTyper::build(&keymap).unwrap();
        assert!(matches!(
            typer.refresh(),
            Err(super::LibeiError::UnknownActiveLayout(2))
        ));
        // A single-layout keymap has only one possible group.
        let mut single = super::KeysymTyper::build(&layout_keymap("de")).unwrap();
        assert!(single.refresh().is_ok());
    }

    #[test]
    fn keysym_typer_follows_a_reported_layout_switch() {
        let keymap = layout_keymap("us,de");
        let mut typer = super::KeysymTyper::build(&keymap).unwrap();
        typer.note_modifiers(0, 0);
        typer.refresh().unwrap();
        let us_z = typer.chars[&'z'];
        assert!(!typer.chars.contains_key(&'ä'), "US layout has no 'ä'");

        // US -> German: 'z' moves to the key US calls 'y', '@' becomes AltGr+Q.
        typer.note_modifiers(1, 0);
        typer.refresh().unwrap();
        let de_z = typer.chars[&'z'];
        assert_ne!(us_z.keycode, de_z.keycode, "QWERTZ swaps z and y");
        assert_eq!(typer.chars[&'y'].keycode, us_z.keycode);
        assert_eq!(typer.chars[&'@'].modifiers, super::MOD_LEVEL3);
        assert_eq!(typer.chars[&'ä'].modifiers, 0);

        // And back again.
        typer.note_modifiers(0, 0);
        typer.refresh().unwrap();
        assert_eq!(typer.chars[&'z'], us_z);
    }

    #[test]
    fn keysym_typer_accounts_for_caps_lock() {
        let keymap = layout_keymap("us");
        let lock = 1
            << keymap
                .mod_get_index("Lock")
                .expect("keymap has a Lock modifier");
        let mut typer = super::KeysymTyper::build(&keymap).unwrap();
        typer.note_modifiers(0, lock);
        typer.refresh().unwrap();
        // With Caps Lock on, the unmodified letter key types uppercase.
        assert_eq!(typer.chars[&'A'].modifiers, 0);
        assert_eq!(typer.chars[&'a'].modifiers, super::MOD_SHIFT);
        assert_eq!(typer.chars[&'1'].modifiers, 0);
    }

    #[test]
    fn keysym_typer_rejects_a_layout_group_the_keymap_does_not_have() {
        let keymap = layout_keymap("us");
        let mut typer = super::KeysymTyper::build(&keymap).unwrap();
        typer.note_modifiers(3, 0);
        assert!(matches!(typer.refresh(), Err(super::LibeiError::Keymap(_))));
    }

    #[test]
    fn unavailable_device_errors_are_retryable_and_unknown_layout_is_not() {
        assert!(super::LibeiError::DeviceUnavailable("removed by the server").is_retryable());
        assert!(!super::LibeiError::UnknownActiveLayout(2).is_retryable());
    }
}
