//! Input-method-v2 input source.
//!
//! This backend is intentionally separate from the core and from the
//! wlroots virtual-keyboard injector. It receives key events only while the
//! compositor has activated the input method and grants its keyboard grab.
//! Printable text, Backspace, Return, and Tab are forwarded through the
//! input-method commit contract. Unsupported non-text keys and shortcut-like
//! modified keys use a separate, mandatory key-event injector for pass-through via libei;
//! the source preserves virtual press/repeat/release state and releases any
//! held keys during focus loss or transport teardown. If that injector is
//! missing or fails, this source reports an error rather than silently
//! discarding keys.

mod composition;
mod protocol;

use composition::{ComposeUpdate, LocalCompose};
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::fd::{AsFd, BorrowedFd},
    time::{Duration, Instant},
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
use wayexpand_core::{
    InjectorCapabilities, InjectorError, InputEvent, InputSource, InputSourceCapabilities,
    InputSourceError, KeyChord, KeyEventState, Modifiers, TextInjector,
};
use wayland_client::{
    protocol::{wl_callback, wl_keyboard, wl_registry, wl_seat::WlSeat},
    Connection, EventQueue, WEnum,
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2, zwp_input_method_v2::ZwpInputMethodV2,
};
use xkbcommon_rs::xkb_state::{KeyDirection, StateComponent};
use xkbcommon_rs::{keysym::keysym_get_name, Context, Keymap, KeymapFormat, State};

const SOURCE_NAME: &str = "input-method-v2";
const MAX_KEYMAP_BYTES: u32 = 4 * 1024 * 1024;
const MAX_COMMIT_TEXT_BYTES: usize = 4000;
const MAX_QUEUED_EVENTS: usize = 4096;
const MAX_PENDING_KEY_PASS_THROUGH: usize = 512;
const INITIAL_ROUNDTRIP_TIMEOUT: Duration = Duration::from_secs(5);

fn connection_poll_failed(flags: rustix::event::PollFlags) -> bool {
    flags.intersects(
        rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    )
}

/// Wait for Wayland connection activity or a reactor wakeup. The wake
/// descriptor is drained here so its readiness behavior can be tested using
/// synthetic eventfds without a live compositor connection.
fn poll_connection_with_wake(
    connection: BorrowedFd<'_>,
    wake: Option<BorrowedFd<'_>>,
    timeout: Duration,
) -> Result<Option<(rustix::event::PollFlags, bool)>, rustix::io::Errno> {
    let timeout = rustix::event::Timespec {
        tv_sec: timeout.as_secs().try_into().unwrap_or(i64::MAX),
        tv_nsec: timeout.subsec_nanos().into(),
    };
    let mut fds = vec![rustix::event::PollFd::new(
        &connection,
        rustix::event::PollFlags::IN
            | rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    )];
    if let Some(wake_fd) = wake.as_ref() {
        fds.push(rustix::event::PollFd::new(
            wake_fd,
            rustix::event::PollFlags::IN,
        ));
    }
    if rustix::event::poll(&mut fds, Some(&timeout))? == 0 {
        return Ok(None);
    }
    let connection_flags = fds[0].revents();
    let woken = fds
        .get(1)
        .is_some_and(|fd| fd.revents().contains(rustix::event::PollFlags::IN));
    drop(fds);
    if woken {
        if let Some(wake) = wake {
            let mut buffer = [0_u8; 8];
            let _ = rustix::io::read(wake, &mut buffer);
        }
    }
    Ok(Some((connection_flags, woken)))
}

/// Shared with `wayexpand-backend-evdev`, which drives the same xkb
/// `State` from raw evdev keycodes instead of Wayland `wl_keyboard` events.
/// Kept here rather than in `wayexpand-core` so the core engine stays
/// independent of xkbcommon.
pub fn classify_keysym(raw_keysym: u32) -> Option<InputEvent> {
    if raw_keysym == xkeysym::key::BackSpace {
        return Some(InputEvent::Backspace);
    }

    if raw_keysym == xkeysym::key::Return
        || raw_keysym == xkeysym::key::KP_Enter
        || raw_keysym == xkeysym::key::Tab
    {
        return Some(InputEvent::Delimiter(if raw_keysym == xkeysym::key::Tab {
            '\t'
        } else {
            '\n'
        }));
    }

    None
}

fn content_type_is_sensitive(
    hint: WEnum<wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentHint>,
    purpose: WEnum<
        wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentPurpose,
    >,
) -> bool {
    let sensitive_hint = match hint {
        WEnum::Value(value) => {
            value.contains(
                wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentHint::HiddenText,
            ) || value.contains(
                wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentHint::SensitiveData,
            )
        }
        WEnum::Unknown(_) => true,
    };
    sensitive_hint
        || match purpose {
        WEnum::Value(
            wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentPurpose::
                Password,
        )
        | WEnum::Unknown(_) => true,
        WEnum::Value(_) => false,
    }
}

/// Shared with `wayexpand-backend-evdev`; see `classify_keysym`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    Delete,
    Commit(&'static str),
    Text(String),
    Ignore,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingKeyPassThrough {
    keycode: u32,
    modifiers: Modifiers,
    state: KeyEventState,
}

#[derive(Debug, Clone)]
struct SurroundingText {
    text: String,
    cursor: u32,
    anchor: u32,
}

#[derive(Debug, Error)]
pub enum InputMethodError {
    #[error("could not connect to Wayland: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    #[error("Wayland dispatch failed: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    #[error("compositor does not advertise {0}")]
    MissingGlobal(&'static str),
    #[error("input method became unavailable")]
    Unavailable,
    #[error("input method keymap could not be decoded: {0}")]
    Keymap(String),
    #[error("input method protocol error: {0}")]
    Protocol(String),
    #[error("input method transport failed: {0}")]
    Transport(String),
    #[error("Wayland startup timed out: {0}")]
    Timeout(String),
    #[error("input-method key pass-through failed: {message}")]
    PassThrough { message: String, retryable: bool },
}

impl InputMethodError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Connect(_)
                | Self::Dispatch(_)
                | Self::Unavailable
                | Self::Transport(_)
                | Self::Timeout(_)
                | Self::PassThrough {
                    retryable: true,
                    ..
                }
        )
    }
}

fn source_error(error: InputMethodError) -> InputSourceError {
    InputSourceError {
        source: SOURCE_NAME,
        retryable: error.is_retryable(),
        message: error.to_string(),
    }
}

struct StateData {
    manager: Option<ZwpInputMethodManagerV2>,
    seat: Option<WlSeat>,
    input_method: Option<ZwpInputMethodV2>,
    keyboard: Option<ZwpInputMethodKeyboardGrabV2>,
    events: VecDeque<InputEvent>,
    keyboard_state: Option<State>,
    surrounding_text: Option<SurroundingText>,
    pending_sensitive: Option<bool>,
    /// Local XKB dead-key/Compose state. This is separate from compositor
    /// IME preedit, which zwp_input_method_v2 does not expose.
    composition_active: bool,
    local_compose: Option<LocalCompose>,
    commit_serial: u32,
    initial_roundtrip_done: bool,
    error: Option<InputMethodError>,
    /// Key events (Linux evdev numbering plus effective modifiers) from
    /// unsupported keys that must be passed through to a separate injector.
    pending_key_pass_through: VecDeque<PendingKeyPassThrough>,
    /// Safety ledger of physical keys whose virtual press may have been or
    /// may yet be sent. A key-up retires an entry only after its matching
    /// release is queued; teardown also inspects pending transitions because
    /// transport errors make delivery of the last event ambiguous.
    virtual_held_keys: Vec<u32>,
}

impl StateData {
    fn new() -> Self {
        Self {
            manager: None,
            seat: None,
            input_method: None,
            keyboard: None,
            events: VecDeque::new(),
            keyboard_state: None,
            surrounding_text: None,
            pending_sensitive: None,
            composition_active: false,
            local_compose: LocalCompose::new(),
            commit_serial: 0,
            initial_roundtrip_done: false,
            error: None,
            pending_key_pass_through: VecDeque::new(),
            virtual_held_keys: Vec::new(),
        }
    }

    fn set_content_type(
        &mut self,
        hint: WEnum<wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentHint>,
        purpose: WEnum<
            wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentPurpose,
        >,
    ) {
        self.pending_sensitive = Some(content_type_is_sensitive(hint, purpose));
    }

    fn finish_protocol_batch(&mut self) {
        self.commit_serial = self.commit_serial.wrapping_add(1);
        if let Some(sensitive) = self.pending_sensitive.take() {
            self.queue_event(InputEvent::FocusChanged { sensitive });
        }
    }

    fn activate(&mut self) {
        self.keyboard_state = None;
        self.surrounding_text = None;
        self.pending_sensitive = None;
        reset_local_composition(self);
        // Do not capture until the compositor reports the current content
        // purpose for this activation.
        self.queue_event(InputEvent::FocusChanged { sensitive: true });
    }

    fn deactivate(&mut self) {
        self.keyboard_state = None;
        self.surrounding_text = None;
        self.pending_sensitive = None;
        reset_local_composition(self);
        // Deactivation can race with already-queued keyboard events. Keep the
        // engine disabled until a new activation reports a safe content type.
        self.queue_event(deactivation_event());
    }

    fn queue_event(&mut self, event: InputEvent) {
        // Focus policy changes supersede every queued keyboard event. This
        // prevents stale text from being processed across activation,
        // deactivation, or a sensitive-field transition.
        if matches!(event, InputEvent::FocusChanged { .. }) {
            self.events.clear();
            // Preserve queued virtual transitions, then release every key
            // still held before the focus transition reaches the daemon.
            let pending_count = self.pending_key_pass_through.len();
            for _ in 0..pending_count {
                self.events.push_back(InputEvent::Reset);
            }
            let held = self
                .virtual_held_keys
                .iter()
                .rev()
                .copied()
                .collect::<Vec<_>>();
            self.virtual_held_keys.clear();
            for keycode in held {
                self.pending_key_pass_through
                    .push_back(PendingKeyPassThrough {
                        keycode,
                        modifiers: Modifiers::default(),
                        state: KeyEventState::Released,
                    });
                self.events.push_back(InputEvent::Reset);
            }
            self.events.push_back(event);
            return;
        }
        if self.events.len() >= MAX_QUEUED_EVENTS {
            // Never let compositor traffic grow memory without bound. A
            // boundary discards the partial trigger while preserving the
            // engine's normal non-sensitive capture policy.
            self.events.clear();
            self.error = Some(InputMethodError::PassThrough {
                message: format!(
                    "input-method event queue overflow (max {} events)",
                    MAX_QUEUED_EVENTS
                ),
                retryable: true,
            });
            self.events.push_back(InputEvent::Reset);
            return;
        }
        self.events.push_back(event);
    }

    fn queue_virtual_key_event(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
        key_state: KeyEventState,
    ) {
        let held_index = self
            .virtual_held_keys
            .iter()
            .position(|held| *held == keycode);
        let should_queue = match key_state {
            KeyEventState::Pressed if held_index.is_some() => false,
            KeyEventState::Pressed => {
                if self.virtual_held_keys.len() >= MAX_PENDING_KEY_PASS_THROUGH {
                    self.error = Some(InputMethodError::PassThrough {
                        message: format!(
                            "too many virtual keys held (max {})",
                            MAX_PENDING_KEY_PASS_THROUGH
                        ),
                        retryable: true,
                    });
                    self.queue_event(InputEvent::Reset);
                    return;
                }
                true
            }
            KeyEventState::Released => held_index.is_some(),
        };

        if should_queue && self.pending_key_pass_through.len() >= MAX_PENDING_KEY_PASS_THROUGH {
            self.error = Some(InputMethodError::PassThrough {
                message: format!(
                    "key pass-through queue overflow (max {} keys pending)",
                    MAX_PENDING_KEY_PASS_THROUGH
                ),
                retryable: true,
            });
            // Do not retire a physical key-up from our held set until its
            // virtual transition is queued. Error teardown will release it.
            self.queue_event(InputEvent::Reset);
            return;
        }

        if should_queue {
            match key_state {
                KeyEventState::Pressed => self.virtual_held_keys.push(keycode),
                KeyEventState::Released => {
                    if let Some(index) = held_index {
                        self.virtual_held_keys.remove(index);
                    }
                }
            }
            self.pending_key_pass_through
                .push_back(PendingKeyPassThrough {
                    keycode,
                    modifiers,
                    state: key_state,
                });
        }
        // Unsupported keys are always a matcher boundary, including repeat
        // notifications that are represented by the held virtual key.
        self.queue_event(InputEvent::Reset);
    }
}

fn key_is_composition(keyboard_state: Option<&State>, key: u32) -> bool {
    let Some(keycode) = key.checked_add(8) else {
        return false;
    };
    let Some(keysym) = keyboard_state.and_then(|state| state.key_get_one_sym(keycode)) else {
        return false;
    };
    let Some(name) = keysym_get_name(&keysym) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    name.starts_with("dead_") || name == "multi_key"
}

fn finish_local_composition(state: &mut StateData) {
    if state.composition_active {
        state.composition_active = false;
        state.queue_event(InputEvent::CompositionChanged { active: false });
    }
}

/// Drop an unfinished local dead-key/Compose sequence when its keyboard
/// focus is invalidated. Its buffered literal fallback belongs to the old
/// focus and must not be committed into whatever application focuses next.
fn reset_local_composition(state: &mut StateData) {
    state.local_compose = LocalCompose::new();
    finish_local_composition(state);
}

fn begin_local_composition(state: &mut StateData) {
    if !state.composition_active {
        state.composition_active = true;
        state.queue_event(InputEvent::CompositionChanged { active: true });
    }
}

fn apply_compose_update(state: &mut StateData, update: ComposeUpdate) -> ComposeUpdate {
    match update {
        ComposeUpdate::Pending => begin_local_composition(state),
        ComposeUpdate::Composed(_) | ComposeUpdate::Cancelled(_) => finish_local_composition(state),
        ComposeUpdate::Inactive => {}
    }
    update
}

/// Shared with `wayexpand-backend-evdev`; see `classify_keysym`.
pub fn key_chord(keyboard_state: &State, key: u32) -> Option<KeyChord> {
    let keycode = key.checked_add(8)?;
    let keysym = keyboard_state.key_get_one_sym(keycode)?;
    let key_name = keysym_get_name(&keysym)?;
    let key = match key_name.to_ascii_uppercase().as_str() {
        "RETURN" | "KP_ENTER" => "ENTER".to_string(),
        "BACKSPACE" => "BACKSPACE".to_string(),
        "ESCAPE" => "ESC".to_string(),
        "SPACE" => "SPACE".to_string(),
        "TAB" => "TAB".to_string(),
        name => name.to_string(),
    };
    Some(KeyChord {
        modifiers: active_modifiers(keyboard_state),
        key,
    })
}

fn active_modifiers(keyboard_state: &State) -> Modifiers {
    let effective = StateComponent::MODS_EFFECTIVE;
    let active = |names: &[&str]| {
        names.iter().any(|name| {
            keyboard_state
                .mod_name_is_active(*name, effective)
                .unwrap_or(false)
        })
    };
    Modifiers {
        ctrl: active(&["Control", "Ctrl"]),
        alt: active(&["Mod1", "Alt"]),
        shift: active(&["Shift"]),
        super_key: active(&["Mod4", "Super"]),
    }
}

/// Shared with `wayexpand-backend-evdev`; see `classify_keysym`.
pub fn is_modifier_keysym(raw_keysym: u32) -> bool {
    matches!(
        raw_keysym,
        xkeysym::key::Shift_L
            | xkeysym::key::Shift_R
            | xkeysym::key::Control_L
            | xkeysym::key::Control_R
            | xkeysym::key::Alt_L
            | xkeysym::key::Alt_R
            | xkeysym::key::Super_L
            | xkeysym::key::Super_R
            | xkeysym::key::Caps_Lock
            | xkeysym::key::Num_Lock
    )
}

/// Shared with `wayexpand-backend-evdev`, which calls this with raw evdev
/// keycodes and a `KeyState` it constructs from `EventSummary::Key`'s value
/// field (0 = released, 1 = pressed; repeats are not passed here).
pub fn key_action_and_update(
    keyboard_state: &mut State,
    key: u32,
    key_state: wl_keyboard::KeyState,
) -> Option<KeyAction> {
    let Some(keycode) = key.checked_add(8) else {
        return Some(KeyAction::Unsupported);
    };
    // XKB expects the keysym/text lookup before the key transition, then the
    // transition must be applied to keep modifiers, dead keys, and compose
    // state valid.
    let action = if key_state == wl_keyboard::KeyState::Pressed {
        key_action(keyboard_state, key)
    } else {
        None
    };
    let direction = match key_state {
        wl_keyboard::KeyState::Pressed => KeyDirection::Down,
        wl_keyboard::KeyState::Released => KeyDirection::Up,
        _ => return None,
    };
    keyboard_state.update_key(keycode, direction);
    action
}

/// Resolve a pressed key using the current XKB state without changing that
/// state. Evdev uses this for kernel repeat events, which repeat the text or
/// editing action of an already-held key without representing another key
/// transition.
pub fn key_action(keyboard_state: &State, key: u32) -> Option<KeyAction> {
    let keycode = key.checked_add(8)?;
    let raw_keysym = keyboard_state
        .key_get_one_sym(keycode)
        .map(|keysym| keysym.raw());
    let modifiers = active_modifiers(keyboard_state);
    match raw_keysym {
        None => Some(KeyAction::Unsupported),
        Some(raw_keysym) if is_modifier_keysym(raw_keysym) => Some(KeyAction::Ignore),
        Some(_) if modifiers.ctrl || modifiers.alt || modifiers.super_key => {
            Some(KeyAction::Unsupported)
        }
        Some(raw_keysym) if matches!(classify_keysym(raw_keysym), Some(InputEvent::Backspace)) => {
            Some(KeyAction::Delete)
        }
        Some(raw_keysym)
            if matches!(classify_keysym(raw_keysym), Some(InputEvent::Delimiter(_))) =>
        {
            Some(KeyAction::Commit(if raw_keysym == xkeysym::key::Tab {
                "\t"
            } else {
                "\n"
            }))
        }
        Some(raw_keysym) if raw_keysym == xkeysym::key::Escape => Some(KeyAction::Unsupported),
        Some(_) => keyboard_state
            .key_get_utf8(keycode)
            .filter(|text| !text.is_empty())
            .and_then(|text| String::from_utf8(text).ok())
            .map(KeyAction::Text)
            .or(Some(KeyAction::Unsupported)),
    }
}

fn key_is_modifier(keyboard_state: &State, key: u32) -> bool {
    key.checked_add(8)
        .and_then(|keycode| keyboard_state.key_get_one_sym(keycode))
        .is_some_and(|keysym| is_modifier_keysym(keysym.raw()))
}

fn forward_commit(state: &mut StateData, connection: &Connection, text: &str) {
    if let Some(input_method) = state.input_method.as_ref() {
        input_method.commit_string(text.to_owned());
        input_method.commit(state.commit_serial);
        match connection.flush() {
            Ok(()) => optimistic_commit(&mut state.surrounding_text, text),
            Err(error) => {
                state.error = Some(InputMethodError::Transport(error.to_string()));
            }
        }
    }
}

fn forward_delete(state: &mut StateData, connection: &Connection, before: u32, after: u32) {
    if let Some(input_method) = state.input_method.as_ref() {
        input_method.delete_surrounding_text(before, after);
        input_method.commit(state.commit_serial);
        if let Err(error) = connection.flush() {
            state.error = Some(InputMethodError::Transport(error.to_string()));
        }
    }
}

fn backspace_delete_lengths(surrounding: Option<&SurroundingText>) -> Option<(u32, u32)> {
    let surrounding = surrounding?;
    let cursor = usize::try_from(surrounding.cursor).ok()?;
    let anchor = usize::try_from(surrounding.anchor).ok()?;
    if cursor > surrounding.text.len()
        || anchor > surrounding.text.len()
        || !surrounding.text.is_char_boundary(cursor)
        || !surrounding.text.is_char_boundary(anchor)
    {
        return None;
    }
    if cursor != anchor {
        return if anchor < cursor {
            Some((u32::try_from(cursor - anchor).ok()?, 0))
        } else {
            Some((0, u32::try_from(anchor - cursor).ok()?))
        };
    }
    let previous = surrounding.text[..cursor]
        .grapheme_indices(true)
        .next_back();
    let start = previous.map_or(cursor, |(index, _)| index);
    Some((u32::try_from(cursor - start).ok()?, 0))
}

fn surrounding_has_selection(surrounding: Option<&SurroundingText>) -> bool {
    surrounding.is_some_and(|text| text.cursor != text.anchor)
}

fn matcher_event_for_deletion(before: u32, after: u32, selected: bool) -> InputEvent {
    if selected || (before == 0 && after == 0) {
        InputEvent::EndOfInput
    } else {
        InputEvent::Backspace
    }
}

fn matcher_event_for_unsupported_key() -> InputEvent {
    // Unsupported keys (Escape, arrows, F-keys, modifiers) are passed through
    // via the lifecycle-aware libei injector. A Reset boundary prevents a
    // partial trigger surviving a lost or dropped key event.
    InputEvent::Reset
}

fn pass_through_pending_keys(
    pending_keys: &mut VecDeque<PendingKeyPassThrough>,
    injector: Option<&mut dyn TextInjector>,
) -> Result<(), InputSourceError> {
    let Some(injector) = injector else {
        if let Some(pending) = pending_keys.front() {
            return Err(source_error(InputMethodError::PassThrough {
                message: format!(
                    "unsupported key {} was captured but no key pass-through injector is attached",
                    pending.keycode
                ),
                retryable: true,
            }));
        }
        return Ok(());
    };
    while let Some(pending) = pending_keys.front().copied() {
        injector
            .inject_key_event(pending.keycode, pending.modifiers, pending.state)
            .map_err(|error| {
                source_error(InputMethodError::PassThrough {
                    message: error.to_string(),
                    retryable: error.retryable,
                })
            })?;
        pending_keys.pop_front();
    }
    Ok(())
}

fn pass_through_pending_key(
    pending_keys: &mut VecDeque<PendingKeyPassThrough>,
    injector: Option<&mut dyn TextInjector>,
) -> Result<(), InputSourceError> {
    let Some(pending) = pending_keys.front().copied() else {
        return Ok(());
    };
    let Some(injector) = injector else {
        return Err(source_error(InputMethodError::PassThrough {
            message: format!(
                "unsupported key {} was captured but no key pass-through injector is attached",
                pending.keycode
            ),
            retryable: true,
        }));
    };
    injector
        .inject_key_event(pending.keycode, pending.modifiers, pending.state)
        .map_err(|error| {
            source_error(InputMethodError::PassThrough {
                message: error.to_string(),
                retryable: error.retryable,
            })
        })?;
    pending_keys.pop_front();
    Ok(())
}

fn release_virtual_keys_from_state(state: &mut StateData, injector: Option<&mut dyn TextInjector>) {
    let held = std::mem::take(&mut state.virtual_held_keys);
    if let Some(injector) = injector {
        let pending_flush_failed =
            pass_through_pending_keys(&mut state.pending_key_pass_through, Some(&mut *injector))
                .is_err();
        let mut release_keys: Vec<u32> = held.into_iter().rev().collect();
        if pending_flush_failed {
            // A failed transition has an ambiguous delivery outcome: the
            // compositor may have observed a press even though flush failed.
            // Keep its keycode until we've attempted a compensating release.
            for pending in state.pending_key_pass_through.iter().rev() {
                if !release_keys.contains(&pending.keycode) {
                    release_keys.push(pending.keycode);
                }
            }
        }
        for keycode in release_keys {
            let _ =
                injector.inject_key_event(keycode, Modifiers::default(), KeyEventState::Released);
        }
    }
    state.pending_key_pass_through.clear();
}

fn deactivation_event() -> InputEvent {
    // A deactivated input method must remain fail-closed until the next
    // activation has reported a non-sensitive content type.
    InputEvent::FocusChanged { sensitive: true }
}

fn decode_keymap(fd: std::os::fd::OwnedFd, size: u32) -> Result<Keymap, InputMethodError> {
    if size == 0 || size > MAX_KEYMAP_BYTES {
        return Err(InputMethodError::Keymap(format!(
            "invalid keymap size {size} bytes"
        )));
    }
    let mut file = File::from(fd);
    file.seek(SeekFrom::Start(0))
        .map_err(|error| InputMethodError::Keymap(error.to_string()))?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| InputMethodError::Keymap(error.to_string()))?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    let text =
        String::from_utf8(bytes).map_err(|error| InputMethodError::Keymap(error.to_string()))?;
    Keymap::new_from_string(
        Context::new(0).map_err(|error| InputMethodError::Keymap(error.to_string()))?,
        &text,
        KeymapFormat::TextV1,
        0,
    )
    .map_err(|error| InputMethodError::Keymap(error.to_string()))
}

trait WaylandEventTransport: Send {
    fn as_fd(&self) -> BorrowedFd<'_>;
    fn flush(&self) -> Result<(), InputMethodError>;
    fn dispatch(&mut self, state: &mut StateData) -> Result<(), InputMethodError>;
}

struct ConnectedWaylandTransport {
    connection: Connection,
    event_queue: EventQueue<StateData>,
}

impl WaylandEventTransport for ConnectedWaylandTransport {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.connection.as_fd()
    }

    fn flush(&self) -> Result<(), InputMethodError> {
        self.connection
            .flush()
            .map_err(|error| InputMethodError::Transport(error.to_string()))
    }

    fn dispatch(&mut self, state: &mut StateData) -> Result<(), InputMethodError> {
        self.event_queue
            .blocking_dispatch(state)
            .map(|_| ())
            .map_err(InputMethodError::Dispatch)
    }
}

pub struct InputMethodSource {
    transport: Box<dyn WaylandEventTransport>,
    state: StateData,
    key_pass_through: Option<Box<dyn TextInjector>>,
    /// Optional non-blocking eventfd that ends an idle wait early; see
    /// [`InputMethodSource::set_wake_fd`].
    wake: Option<std::sync::Arc<std::os::fd::OwnedFd>>,
}

impl InputMethodSource {
    /// Probe protocol availability without registering an input-method object
    /// or taking the compositor's keyboard grab.
    pub fn probe() -> Result<(), InputMethodError> {
        let connection = Connection::connect_to_env()?;
        let mut event_queue = connection.new_event_queue();
        let qh = event_queue.handle();
        let mut state = StateData::new();
        connection.display().get_registry(&qh, ());
        roundtrip_with_timeout(
            &connection,
            &mut event_queue,
            &mut state,
            INITIAL_ROUNDTRIP_TIMEOUT,
        )?;
        state.manager.ok_or(InputMethodError::MissingGlobal(
            "zwp_input_method_manager_v2",
        ))?;
        state
            .seat
            .ok_or(InputMethodError::MissingGlobal("wl_seat"))?;
        Ok(())
    }

    /// Connect an input-method session.
    ///
    /// The compositor may give this object an exclusive keyboard grab after
    /// activation. Printable text and common editing keys are forwarded via
    /// the input-method commit contract, and unsupported keys can be passed
    /// through via a separate lifecycle-aware injector (e.g. libei). The
    /// daemon attaches this injector before exposing the source to the
    /// compositor, so an active source always has a safe pass-through path.
    pub fn connect() -> Result<Self, InputMethodError> {
        let connection = Connection::connect_to_env()?;
        let mut event_queue = connection.new_event_queue();
        let qh = event_queue.handle();
        let mut state = StateData::new();
        connection.display().get_registry(&qh, ());
        roundtrip_with_timeout(
            &connection,
            &mut event_queue,
            &mut state,
            INITIAL_ROUNDTRIP_TIMEOUT,
        )?;
        let manager = state
            .manager
            .clone()
            .ok_or(InputMethodError::MissingGlobal(
                "zwp_input_method_manager_v2",
            ))?;
        let seat = state
            .seat
            .clone()
            .ok_or(InputMethodError::MissingGlobal("wl_seat"))?;
        state.input_method = Some(manager.get_input_method(&seat, &qh, ()));
        connection
            .flush()
            .map_err(|error| InputMethodError::Protocol(error.to_string()))?;
        Ok(Self {
            transport: Box::new(ConnectedWaylandTransport {
                connection,
                event_queue,
            }),
            state,
            key_pass_through: None,
            wake: None,
        })
    }

    /// Attach a separate injector for passing through unsupported keys
    /// (Escape, arrows, F-keys, modifiers, etc.) from the input-method-v2
    /// exclusive grab. The injector must preserve individual press and release
    /// events; the default trait implementation rejects releases safely.
    pub fn with_key_pass_through(mut self, injector: Box<dyn TextInjector>) -> Self {
        self.key_pass_through = Some(injector);
        self
    }

    /// Report the separately attached keyboard injector. The input-method
    /// text injector and the key pass-through injector are different protocol
    /// paths; status consumers must not infer keyboard capabilities from the
    /// former.
    pub fn key_pass_through_capabilities(&self) -> Option<InjectorCapabilities> {
        self.key_pass_through
            .as_ref()
            .map(|injector| injector.capabilities())
    }

    /// Return the negotiated mode of the separate keyboard injector for
    /// diagnostics. An input-method session otherwise has no text-injector
    /// mode to report because its text path is the Wayland protocol itself.
    pub fn key_pass_through_status_detail(&self) -> Option<&'static str> {
        self.key_pass_through
            .as_ref()
            .map(|injector| injector.status_detail())
            .filter(|detail| !detail.is_empty())
    }

    /// Best-effort cleanup for a lost input-method or pass-through transport.
    /// The input-method grab may disappear without delivering physical key-up
    /// events, so every key the virtual injector believes is held must be
    /// released before the injector is dropped or replaced.
    fn release_virtual_keys(&mut self) {
        let injector = self
            .key_pass_through
            .as_mut()
            .map(|injector| injector.as_mut() as &mut dyn TextInjector);
        release_virtual_keys_from_state(&mut self.state, injector);
    }

    /// Poll for one event without indefinitely blocking lifecycle handling in
    /// a daemon. This is intentionally an additive API; `InputSource::next_event`
    /// remains the blocking interface for simple consumers.
    /// Install a non-blocking eventfd that ends [`Self::next_event_timeout`]
    /// early when another thread has work for the caller. The source drains
    /// it when it fires.
    pub fn set_wake_fd(&mut self, wake: Option<std::sync::Arc<std::os::fd::OwnedFd>>) {
        self.wake = wake;
    }

    pub fn next_event_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<InputEvent>, InputSourceError> {
        if let Some(error) = self.state.error.take() {
            self.release_virtual_keys();
            return Err(source_error(error));
        }
        if let Some(event) = self.state.events.pop_front() {
            if matches!(event, InputEvent::Reset) {
                let injector = self
                    .key_pass_through
                    .as_mut()
                    .map(|injector| injector.as_mut() as &mut dyn TextInjector);
                let result =
                    pass_through_pending_key(&mut self.state.pending_key_pass_through, injector);
                if result.is_err() {
                    self.release_virtual_keys();
                }
                result?;
            }
            return Ok(Some(event));
        }
        if let Err(error) = self.transport.flush() {
            self.release_virtual_keys();
            return Err(source_error(error));
        }
        let Some((revents, woken)) = poll_connection_with_wake(
            self.transport.as_fd(),
            self.wake.as_deref().map(AsFd::as_fd),
            timeout,
        )
        .map_err(|error| source_error(InputMethodError::Transport(error.to_string())))?
        else {
            return Ok(None);
        };
        if woken && revents.is_empty() {
            // Only the wake descriptor fired: let the caller run.
            return Ok(None);
        }
        if connection_poll_failed(revents) {
            self.release_virtual_keys();
            return Err(source_error(InputMethodError::Transport(format!(
                "Wayland connection became unavailable ({revents:?})"
            ))));
        }
        if let Err(error) = self.transport.dispatch(&mut self.state) {
            self.release_virtual_keys();
            return Err(source_error(error));
        }
        if let Some(error) = self.state.error.take() {
            self.release_virtual_keys();
            return Err(source_error(error));
        }
        if let Some(event) = self.state.events.pop_front() {
            if matches!(event, InputEvent::Reset) {
                let injector = self
                    .key_pass_through
                    .as_mut()
                    .map(|injector| injector.as_mut() as &mut dyn TextInjector);
                let result =
                    pass_through_pending_key(&mut self.state.pending_key_pass_through, injector);
                if result.is_err() {
                    self.release_virtual_keys();
                }
                result?;
            }
            Ok(Some(event))
        } else {
            Ok(None)
        }
    }
}

impl Drop for InputMethodSource {
    fn drop(&mut self) {
        self.release_virtual_keys();
    }
}

fn roundtrip_with_timeout(
    connection: &Connection,
    event_queue: &mut EventQueue<StateData>,
    state: &mut StateData,
    timeout: Duration,
) -> Result<(), InputMethodError> {
    let qh = event_queue.handle();
    connection.display().sync(&qh, ());
    connection
        .flush()
        .map_err(|error| InputMethodError::Protocol(error.to_string()))?;
    let deadline = Instant::now() + timeout;

    while !state.initial_roundtrip_done {
        event_queue.dispatch_pending(state)?;
        if state.initial_roundtrip_done {
            break;
        }
        let Some(guard) = connection.prepare_read() else {
            continue;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(InputMethodError::Timeout(
                "Wayland registry roundtrip timed out".into(),
            ));
        }
        let timeout = rustix::event::Timespec {
            tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: remaining.subsec_nanos().into(),
        };
        let fd = guard.connection_fd();
        let mut fds = [rustix::event::PollFd::new(
            &fd,
            rustix::event::PollFlags::IN
                | rustix::event::PollFlags::ERR
                | rustix::event::PollFlags::HUP
                | rustix::event::PollFlags::NVAL,
        )];
        if rustix::event::poll(&mut fds, Some(&timeout))
            .map_err(|error| InputMethodError::Protocol(error.to_string()))?
            == 0
        {
            return Err(InputMethodError::Timeout(
                "Wayland registry roundtrip timed out".into(),
            ));
        }
        let revents = fds[0].revents();
        if connection_poll_failed(revents) {
            return Err(InputMethodError::Protocol(format!(
                "Wayland connection became unavailable ({revents:?})"
            )));
        }
        guard
            .read()
            .map_err(|error| InputMethodError::Protocol(error.to_string()))?;
    }
    Ok(())
}

impl TextInjector for InputMethodSource {
    // `move_cursor_left` is not overridden (falls back to the trait's
    // no-op default): `zwp_input_method_v2` offers only `commit_string`
    // and `delete_surrounding_text`, and every `commit_string` leaves the
    // cursor immediately after the text it just inserted with no protocol
    // request to move it back within already-committed text. Re-deleting
    // and re-committing the trailing text would just land the cursor at
    // the end again, so there is no sequence of requests in this protocol
    // that achieves `{{cursor}}` placement -- unlike the keyboard-level
    // backends (libei, wlroots), which can synthesize an actual Left key.
    fn name(&self) -> &'static str {
        SOURCE_NAME
    }

    fn capabilities(&self) -> InjectorCapabilities {
        InjectorCapabilities {
            insertion_mode: "input-method-v2 text",
            max_text_chars: 0,
            expected_throughput_chars_per_sec: None,
            atomic_replace: true,
            // `trigger_delete_length` refuses unless surrounding text ends
            // with the trigger, and delete + commit share one `done`.
            replacement_guarantee: wayexpand_core::ReplacementGuarantee::Atomic,
            full_unicode: true,
            cursor_reposition: false,
            key_passthrough: false,
        }
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        let bytes = trigger_delete_length(self.state.surrounding_text.as_ref(), trigger)?;
        let Some(input_method) = self.state.input_method.as_ref() else {
            return Err(InjectorError {
                backend: SOURCE_NAME,
                message: "input method is not active".into(),
                retryable: true,
            });
        };
        input_method.delete_surrounding_text(bytes, 0);
        input_method.commit(self.state.commit_serial);
        self.transport.flush().map_err(|error| InjectorError {
            backend: SOURCE_NAME,
            message: error.to_string(),
            retryable: true,
        })?;
        optimistic_replace(&mut self.state.surrounding_text, trigger, "")
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        validate_commit_text(text)?;
        if text.is_empty() {
            return Ok(());
        }
        let Some(input_method) = self.state.input_method.as_ref() else {
            return Err(InjectorError {
                backend: SOURCE_NAME,
                message: "input method is not active".into(),
                retryable: true,
            });
        };
        input_method.commit_string(text.to_owned());
        input_method.commit(self.state.commit_serial);
        self.transport.flush().map_err(|error| InjectorError {
            backend: SOURCE_NAME,
            message: error.to_string(),
            retryable: true,
        })?;
        optimistic_commit(&mut self.state.surrounding_text, text);
        Ok(())
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        validate_commit_text(text)?;
        let bytes = trigger_delete_length(self.state.surrounding_text.as_ref(), trigger)?;
        let Some(input_method) = self.state.input_method.as_ref() else {
            return Err(InjectorError {
                backend: SOURCE_NAME,
                message: "input method is not active".into(),
                retryable: true,
            });
        };
        input_method.delete_surrounding_text(bytes, 0);
        if !text.is_empty() {
            input_method.commit_string(text.to_owned());
        }
        input_method.commit(self.state.commit_serial);
        self.transport.flush().map_err(|error| InjectorError {
            backend: SOURCE_NAME,
            message: error.to_string(),
            retryable: true,
        })?;
        optimistic_replace(&mut self.state.surrounding_text, trigger, text)
    }
}

fn optimistic_commit(surrounding: &mut Option<SurroundingText>, text: &str) {
    let Some(surrounding) = surrounding.as_mut() else {
        return;
    };
    let Ok(cursor) = usize::try_from(surrounding.cursor) else {
        return;
    };
    let Ok(anchor) = usize::try_from(surrounding.anchor) else {
        return;
    };
    if cursor > surrounding.text.len()
        || anchor > surrounding.text.len()
        || !surrounding.text.is_char_boundary(cursor)
        || !surrounding.text.is_char_boundary(anchor)
    {
        return;
    }
    let start = cursor.min(anchor);
    let end = cursor.max(anchor);
    surrounding.text.replace_range(start..end, text);
    let new_cursor = start + text.len();
    let Ok(new_cursor) = u32::try_from(new_cursor) else {
        return;
    };
    surrounding.cursor = new_cursor;
    surrounding.anchor = new_cursor;
}

fn optimistic_replace(
    surrounding: &mut Option<SurroundingText>,
    trigger: &str,
    text: &str,
) -> Result<(), InjectorError> {
    let bytes = trigger_delete_length(surrounding.as_ref(), trigger)?;
    let Some(surrounding) = surrounding.as_mut() else {
        return Err(InjectorError {
            backend: SOURCE_NAME,
            message: "surrounding text is unavailable after replacement".into(),
            retryable: true,
        });
    };
    let cursor = usize::try_from(surrounding.cursor).map_err(|_| InjectorError {
        backend: SOURCE_NAME,
        message: "surrounding text cursor is invalid after replacement".into(),
        retryable: true,
    })?;
    let start = cursor
        .checked_sub(usize::try_from(bytes).unwrap_or(usize::MAX))
        .ok_or_else(|| InjectorError {
            backend: SOURCE_NAME,
            message: "replacement trigger exceeds surrounding text".into(),
            retryable: true,
        })?;
    surrounding.text.replace_range(start..cursor, text);
    let new_cursor = start + text.len();
    surrounding.cursor = u32::try_from(new_cursor).map_err(|_| InjectorError {
        backend: SOURCE_NAME,
        message: "replacement cursor exceeds protocol range".into(),
        retryable: false,
    })?;
    surrounding.anchor = surrounding.cursor;
    Ok(())
}

fn trigger_delete_length(
    surrounding: Option<&SurroundingText>,
    trigger: &str,
) -> Result<u32, InjectorError> {
    if !surrounding_ends_with_trigger(surrounding, trigger) {
        return Err(InjectorError {
            backend: SOURCE_NAME,
            message: "surrounding text no longer matches the expansion trigger".into(),
            // A fresh input-method session may receive current surrounding
            // text, so this is safe to recover by reconnecting. Crucially, no
            // delete or commit is sent before this validation succeeds.
            retryable: true,
        });
    }
    u32::try_from(trigger.len()).map_err(|_| InjectorError {
        backend: SOURCE_NAME,
        message: "trigger byte length exceeds protocol range".into(),
        retryable: false,
    })
}

fn surrounding_ends_with_trigger(surrounding: Option<&SurroundingText>, trigger: &str) -> bool {
    let Some(surrounding) = surrounding else {
        return false;
    };
    if surrounding.cursor != surrounding.anchor {
        return false;
    }
    let Ok(cursor) = usize::try_from(surrounding.cursor) else {
        return false;
    };
    if cursor > surrounding.text.len() || !surrounding.text.is_char_boundary(cursor) {
        return false;
    }
    let Some(start) = cursor.checked_sub(trigger.len()) else {
        return false;
    };
    surrounding.text.get(start..cursor) == Some(trigger)
}

fn validate_commit_text(text: &str) -> Result<(), InjectorError> {
    if text.len() > MAX_COMMIT_TEXT_BYTES {
        return Err(InjectorError {
            backend: SOURCE_NAME,
            message: format!(
                "replacement is {} bytes; input-method commit limit is {MAX_COMMIT_TEXT_BYTES}",
                text.len()
            ),
            retryable: false,
        });
    }
    if let Some(character) = text
        .chars()
        .find(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(InjectorError {
            backend: SOURCE_NAME,
            message: format!(
                "replacement contains unsupported control character U+{:04X}",
                character as u32
            ),
            retryable: false,
        });
    }
    Ok(())
}

impl InputSource for InputMethodSource {
    fn name(&self) -> &'static str {
        SOURCE_NAME
    }

    fn capabilities(&self) -> InputSourceCapabilities {
        InputSourceCapabilities {
            key_passthrough: self.key_pass_through.is_some(),
            ..InputSourceCapabilities::INPUT_METHOD_V2
        }
    }

    fn next_event(&mut self) -> Result<InputEvent, InputSourceError> {
        loop {
            if let Some(error) = self.state.error.take() {
                self.release_virtual_keys();
                return Err(source_error(error));
            }
            if let Some(event) = self.state.events.pop_front() {
                if matches!(event, InputEvent::Reset) {
                    let injector = self
                        .key_pass_through
                        .as_mut()
                        .map(|injector| injector.as_mut() as &mut dyn TextInjector);
                    let result = pass_through_pending_key(
                        &mut self.state.pending_key_pass_through,
                        injector,
                    );
                    if result.is_err() {
                        self.release_virtual_keys();
                    }
                    result?;
                }
                return Ok(event);
            }
            if let Err(error) = self.transport.dispatch(&mut self.state) {
                self.release_virtual_keys();
                return Err(source_error(error));
            }
        }
    }
}

#[cfg(test)]
mod tests;
