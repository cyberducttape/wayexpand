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
//! This tradeoff prioritizes compositor and toolkit compatibility over
//! speed. It does not make a multi-event replacement atomic or verify the
//! text at the caret; replacement assurance is a route-level property of the
//! capture and focus contracts. Very large keysym fallbacks are refused
//! before erasing the trigger rather than occupying the daemon's synchronous
//! keyboard path for several seconds.
//! If latency is a concern:
//! - Prefer ei_text when available (no per-character delay)
//! - Consider enabling ei_text support in your EIS server if you control it
//! - Use the input-method-v2 backend as an alternative (if supported by your compositor)
//! - File an issue if your EIS server supports ei_text but doesn't resume devices with it

use reis::{ei, enumflags2::BitFlags, event::DeviceCapability};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use wayexpand_core::{InjectorCapabilities, InjectorError, KeyEventState, Modifiers};
#[cfg(test)]
use xkbcommon_rs::{Context, Keymap as XkbKeymap};

mod connection;
mod events;
mod fcitx5;
mod injection;
mod keymap;
mod lifecycle;
mod portal_token;
mod unicode;
mod worker;

use connection::{connect_portal, PortalKeepalive};
use events::{handshake_with_cancellation, EventPump};
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
pub use worker::LibeiInjector;

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
// A keysym fallback longer than `wayexpand_core::keystroke_max_chars()`
// (the user's `keystroke_max_chars`, default 250: about three seconds) is
// refused before anything is sent. Native `ei_text` is unaffected.

// XKB keycodes carry the legacy X11 offset of 8 over the Linux evdev codes
// that `ei_keyboard.key()` expects (see the existing KEY_BACKSPACE handling
// below, which is already evdev-numbered).
const XKB_KEYCODE_OFFSET: u32 = 8;

fn run_checkpointed<E>(
    count: usize,
    mut checkpoint: impl FnMut() -> Result<(), E>,
    mut action: impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    for _ in 0..count {
        checkpoint()?;
        action()?;
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum LibeiError {
    #[error("libei connection attempt was cancelled")]
    Cancelled,
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
    #[error("typing stopped before the end of the text: {0}")]
    Interrupted(String),
    #[error(
        "keysym fallback for {characters} characters would take about {estimated_ms} ms; \
         use an ei_text-capable backend or shorten the replacement"
    )]
    FallbackTooSlow {
        characters: usize,
        estimated_ms: u64,
    },
    #[error(
        "the EIS server is still sending device updates; the expansion was not typed against \
         keyboard state that may be out of date"
    )]
    EventBacklog,
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

struct LibeiSession {
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
    fcitx5: Option<fcitx5::Client>,
    /// Run before every paced keysym character; a failure stops typing.
    checkpoint: wayexpand_core::OutputCheckpoint,
}

/// Controls how the RemoteDesktop portal session is restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibeiOptions {
    pub persist_portal_token: bool,
    /// Enable the optional Fcitx5 exact surrounding-text bridge.
    pub fcitx5_direct_commit: bool,
    /// Token path selected by the service. `None` keeps the standalone XDG
    /// default used by CLI/library callers.
    pub portal_token_path: Option<PathBuf>,
}

/// Cooperative cancellation shared by a connection owner and its worker.
/// Portal requests observe cancellation promptly; protocol setup checks it
/// between bounded reads.
#[derive(Clone, Debug, Default)]
pub struct ConnectCancellation(Arc<AtomicBool>);

impl ConnectCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Default for LibeiOptions {
    fn default() -> Self {
        Self {
            persist_portal_token: true,
            fcitx5_direct_commit: false,
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

impl LibeiSession {
    /// Apply everything the server reported since the last call, then make
    /// sure the keyboard state still matches what we would type against.
    /// Called before any text-producing request: a layout switch, Caps Lock,
    /// or a replaced device must never be typed through with stale
    /// assumptions, so this rebuilds the keysym map or refuses (fail closed).
    fn sync_server_state(&mut self) -> Result<(), LibeiError> {
        let Self {
            events: pump,
            mode,
            device,
            device_unavailable,
            ..
        } = self;
        pump.drain(events::DrainBudget::SYNC, |event| {
            match event {
                reis::event::EiEvent::KeyboardModifiers(modifiers)
                    if modifiers.device == *device =>
                {
                    if let TextMode::Keysym(typer) = mode {
                        typer.note_modifiers(modifiers.group, modifiers.locked);
                    }
                }
                reis::event::EiEvent::DevicePaused(paused) if paused.device == *device => {
                    *device_unavailable = Some("paused by the server");
                }
                reis::event::EiEvent::DeviceResumed(resumed) if resumed.device == *device => {
                    if *device_unavailable == Some("paused by the server") {
                        *device_unavailable = None;
                    }
                }
                reis::event::EiEvent::DeviceRemoved(removed) if removed.device == *device => {
                    // A keymap change replaces the device; its old keymap
                    // must not be used again.
                    *device_unavailable = Some("removed by the server");
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
            Ok(())
        })?;
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
            if characters > wayexpand_core::keystroke_max_chars() {
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

    /// Sends text through `ei_text`, checking focus before each protocol chunk
    /// and flushing it before checking again. Keysym output uses `type_keys`
    /// because synthesized key events also need inter-key pacing.
    fn send_text_chunks(
        &mut self,
        text_interface: &ei::Text,
        text: &str,
    ) -> Result<(), LibeiError> {
        for chunk in split_text_chunks(text) {
            self.checkpoint.check().map_err(LibeiError::Interrupted)?;
            let serial = self.connection.serial();
            self.device.device().start_emulating(serial, self.sequence);
            self.sequence = self.sequence.checked_add(1).unwrap_or(1);
            text_interface.utf8(chunk);
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.device.device().stop_emulating(serial);
            self.connection
                .flush()
                .map_err(|error| LibeiError::Flush(error.to_string()))?;
        }
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<(), LibeiError> {
        validate_text(text)?;
        self.ensure_representable(text)?;
        let TextMode::Text(text_interface) = &self.mode else {
            return self.type_keys(text);
        };
        let text_interface = text_interface.clone();
        // Split on newlines and tabs since ei_text doesn't handle control characters.
        // We'll send printable text via ei_text and handle control chars via keyboard events.
        let mut current_text = String::new();
        for c in text.chars() {
            match c {
                '\n' | '\t' => {
                    self.send_text_chunks(&text_interface, &current_text)?;
                    current_text.clear();
                    self.checkpoint.check().map_err(LibeiError::Interrupted)?;
                    self.send_control_char(c);
                    self.connection
                        .flush()
                        .map_err(|error| LibeiError::Flush(error.to_string()))?;
                }
                _ => current_text.push(c),
            }
        }
        self.send_text_chunks(&text_interface, &current_text)
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
            // Each character lands at least 12 ms after the previous one, so
            // focus can move or the transaction can be abandoned mid-text:
            // stop before typing anything more into the wrong place.
            self.checkpoint.check().map_err(LibeiError::Interrupted)?;
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
        let checkpoint = self.checkpoint.clone();
        run_checkpointed(
            chars,
            || checkpoint.check().map_err(LibeiError::Interrupted),
            || {
                self.send_backspaces_unflushed(1);
                self.connection
                    .flush()
                    .map_err(|error| LibeiError::Flush(error.to_string()))
            },
        )
    }

    /// Sends `count` Left-arrow key presses, for `{{cursor}}` placement
    /// after a replacement has already been typed in full.
    fn send_left_arrows(&mut self, count: usize) -> Result<(), LibeiError> {
        if count == 0 {
            return Ok(());
        }
        let checkpoint = self.checkpoint.clone();
        run_checkpointed(
            count,
            || checkpoint.check().map_err(LibeiError::Interrupted),
            || {
                let serial = self.connection.serial();
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
                self.connection
                    .flush()
                    .map_err(|error| LibeiError::Flush(error.to_string()))
            },
        )
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

#[cfg(test)]
mod tests;
