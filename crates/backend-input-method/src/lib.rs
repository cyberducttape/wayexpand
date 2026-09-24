//! Input-method-v2 input source.
//!
//! This backend is intentionally separate from the core and from the
//! wlroots virtual-keyboard injector. It receives key events only while the
//! compositor has activated the input method and grants its keyboard grab.
//! Printable text, Backspace, Return, and Tab are forwarded through the
//! input-method commit contract. Unsupported non-text keys and shortcut-like
//! modified keys require a separate key-event injector for pass-through; if
//! that injector is missing or fails, this source reports an error rather than
//! silently discarding keys.

use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Seek, SeekFrom},
    time::{Duration, Instant},
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
use wayexpand_core::{
    InjectorError, InputEvent, InputSource, InputSourceError, KeyChord, Modifiers, TextInjector,
};
use wayland_client::{
    protocol::{wl_callback, wl_keyboard, wl_registry, wl_seat::WlSeat},
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
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
const INITIAL_ROUNDTRIP_TIMEOUT: Duration = Duration::from_secs(5);

fn connection_poll_failed(flags: rustix::event::PollFlags) -> bool {
    flags.intersects(
        rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    )
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
    commit_serial: u32,
    initial_roundtrip_done: bool,
    error: Option<InputMethodError>,
    /// Key events (Linux evdev numbering plus effective modifiers) from
    /// unsupported keys that must be passed through to a separate injector.
    pending_key_pass_through: VecDeque<PendingKeyPassThrough>,
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
            commit_serial: 0,
            initial_roundtrip_done: false,
            error: None,
            pending_key_pass_through: VecDeque::new(),
        }
    }

    fn queue_event(&mut self, event: InputEvent) {
        // Focus policy changes supersede every queued keyboard event. This
        // prevents stale text from being processed across activation,
        // deactivation, or a sensitive-field transition.
        if matches!(event, InputEvent::FocusChanged { .. }) {
            self.events.clear();
            self.events.push_back(event);
            return;
        }
        if self.events.len() >= MAX_QUEUED_EVENTS {
            // Never let compositor traffic grow memory without bound. A
            // boundary discards the partial trigger while preserving the
            // engine's normal non-sensitive capture policy.
            self.events.clear();
            self.events.push_back(InputEvent::Reset);
            return;
        }
        self.events.push_back(event);
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for StateData {
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
        if interface == "zwp_input_method_manager_v2" && state.manager.is_none() {
            state.manager = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == "wl_seat" && state.seat.is_none() {
            state.seat = Some(registry.bind(name, version.min(7), qh, ()));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for StateData {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.initial_roundtrip_done = true;
        }
    }
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for StateData {
    fn event(
        _: &mut Self,
        _: &ZwpInputMethodManagerV2,
        _: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for StateData {
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

impl Dispatch<ZwpInputMethodV2, ()> for StateData {
    fn event(
        state: &mut Self,
        proxy: &ZwpInputMethodV2,
        event: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event,
        _: &(),
        connection: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::Activate => {
                // The compositor may activate without an intervening
                // Deactivate; release the outgoing grab object so it does not
                // leak on the compositor side.
                if let Some(previous) = state.keyboard.take() {
                    previous.release();
                    let _ = connection.flush();
                }
                state.keyboard = Some(proxy.grab_keyboard(qh, ()));
                state.surrounding_text = None;
                state.pending_sensitive = None;
                // Do not capture until the compositor has delivered the
                // current content purpose. This prevents an activation race
                // from briefly treating a password field as ordinary text.
                state.queue_event(InputEvent::FocusChanged { sensitive: true });
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::Deactivate => {
                if let Some(previous) = state.keyboard.take() {
                    previous.release();
                    let _ = connection.flush();
                }
                state.keyboard_state = None;
                state.surrounding_text = None;
                state.pending_sensitive = None;
                // Deactivation can race with already-queued keyboard events.
                // Keep the engine disabled until a new activation reports a
                // non-sensitive content type.
                state.queue_event(deactivation_event());
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::ContentType { hint, purpose } => {
                state.pending_sensitive = Some(content_type_is_sensitive(hint, purpose));
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::SurroundingText {
                text,
                cursor,
                anchor,
            } => {
                state.surrounding_text = Some(SurroundingText {
                    text,
                    cursor,
                    anchor,
                });
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::Done => {
                state.commit_serial = state.commit_serial.wrapping_add(1);
                if let Some(sensitive) = state.pending_sensitive.take() {
                    state.queue_event(InputEvent::FocusChanged { sensitive });
                }
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::Unavailable => {
                state.error = Some(InputMethodError::Unavailable);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, ()> for StateData {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodKeyboardGrabV2,
        event: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Deactivation can race with events already queued by the compositor.
        // Do not commit or delete anything from a stale exclusive grab.
        if state.keyboard.is_none() {
            return;
        }
        use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::Event;
        match event {
            Event::Keymap { format, fd, size } => {
                if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    state.error = Some(InputMethodError::Keymap(format!(
                        "unsupported keymap format {format:?}"
                    )));
                    return;
                }
                match decode_keymap(fd, size) {
                    Ok(keymap) => state.keyboard_state = Some(State::new(keymap)),
                    Err(error) => state.error = Some(error),
                }
            }
            Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(keyboard_state) = state.keyboard_state.as_mut() {
                    keyboard_state.update_mask(
                        mods_depressed,
                        mods_latched,
                        mods_locked,
                        0,
                        0,
                        group as usize,
                    );
                }
            }
            Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                if key_state == wl_keyboard::KeyState::Pressed {
                    if let Some(keyboard_state) = state.keyboard_state.as_ref() {
                        if let Some(chord) = key_chord(keyboard_state, key) {
                            state.queue_event(InputEvent::Key(chord));
                        }
                    }
                }
                let action = state
                    .keyboard_state
                    .as_mut()
                    .map_or(Some(KeyAction::Unsupported), |keyboard_state| {
                        key_action_and_update(keyboard_state, key, key_state)
                    });
                match action {
                    Some(KeyAction::Delete) => {
                        let selected = surrounding_has_selection(state.surrounding_text.as_ref());
                        if let Some((before, after)) =
                            backspace_delete_lengths(state.surrounding_text.as_ref())
                        {
                            forward_delete(state, connection, before, after);
                            state
                                .events
                                .push_back(matcher_event_for_deletion(before, after, selected));
                        } else {
                            state.error = Some(InputMethodError::Protocol(
                                "cannot safely forward Backspace without valid surrounding text"
                                    .into(),
                            ));
                        }
                    }
                    Some(KeyAction::Commit(text)) => {
                        forward_commit(state, connection, text);
                        state.queue_event(InputEvent::Delimiter(
                            text.chars().next().unwrap_or('\n'),
                        ));
                    }
                    Some(KeyAction::Text(text)) => {
                        forward_commit(state, connection, &text);
                        state.queue_event(InputEvent::Text(text));
                    }
                    Some(KeyAction::Ignore) => {}
                    Some(KeyAction::Unsupported) => {
                        if key_state == wl_keyboard::KeyState::Pressed {
                            let modifiers = state
                                .keyboard_state
                                .as_ref()
                                .map(active_modifiers)
                                .unwrap_or_default();
                            state
                                .pending_key_pass_through
                                .push_back(PendingKeyPassThrough {
                                    keycode: key,
                                    modifiers,
                                });
                        }
                        state.queue_event(matcher_event_for_unsupported_key());
                    }
                    None => {}
                }
            }
            Event::Key {
                state: WEnum::Unknown(_),
                ..
            } => {
                // An unrecognized key-state enum is a single malformed event,
                // not a reason to tear down the whole source: discard it the
                // same way an unsupported key is discarded.
                state.queue_event(matcher_event_for_unsupported_key());
            }
            _ => {}
        }
    }
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
    // The keyboard grab is exclusive, so the key cannot be passed through
    // safely. A boundary prevents a partial trigger surviving the lost event.
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
    while let Some(pending) = pending_keys.pop_front() {
        injector
            .inject_key_with_modifiers(pending.keycode, pending.modifiers)
            .map_err(|error| {
                source_error(InputMethodError::PassThrough {
                    message: error.to_string(),
                    retryable: error.retryable,
                })
            })?;
    }
    Ok(())
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

pub struct InputMethodSource {
    connection: Connection,
    event_queue: EventQueue<StateData>,
    state: StateData,
    key_pass_through: Option<Box<dyn TextInjector>>,
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
    /// the input-method commit contract, and unsupported keys can now be
    /// passed through via an optional separate injector (e.g., libei).
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
            connection,
            event_queue,
            state,
            key_pass_through: None,
        })
    }

    /// Attach an optional separate injector for passing through unsupported keys
    /// (Escape, arrows, F-keys, etc.) from the input-method-v2 exclusive grab.
    pub fn with_key_pass_through(mut self, injector: Box<dyn TextInjector>) -> Self {
        self.key_pass_through = Some(injector);
        self
    }

    /// Poll for one event without indefinitely blocking lifecycle handling in
    /// a daemon. This is intentionally an additive API; `InputSource::next_event`
    /// remains the blocking interface for simple consumers.
    pub fn next_event_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<InputEvent>, InputSourceError> {
        if let Some(error) = self.state.error.take() {
            return Err(source_error(error));
        }
        if let Some(event) = self.state.events.pop_front() {
            if matches!(event, InputEvent::Reset) {
                let injector = self
                    .key_pass_through
                    .as_mut()
                    .map(|injector| injector.as_mut() as &mut dyn TextInjector);
                pass_through_pending_keys(&mut self.state.pending_key_pass_through, injector)?;
            }
            return Ok(Some(event));
        }
        self.connection
            .flush()
            .map_err(|error| source_error(InputMethodError::Transport(error.to_string())))?;
        let timeout = rustix::event::Timespec {
            tv_sec: timeout.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: timeout.subsec_nanos().into(),
        };
        let mut fds = [rustix::event::PollFd::new(
            &self.connection,
            rustix::event::PollFlags::IN
                | rustix::event::PollFlags::ERR
                | rustix::event::PollFlags::HUP
                | rustix::event::PollFlags::NVAL,
        )];
        if rustix::event::poll(&mut fds, Some(&timeout))
            .map_err(|error| source_error(InputMethodError::Transport(error.to_string())))?
            == 0
        {
            return Ok(None);
        }
        let revents = fds[0].revents();
        if connection_poll_failed(revents) {
            return Err(source_error(InputMethodError::Transport(format!(
                "Wayland connection became unavailable ({revents:?})"
            ))));
        }
        self.event_queue
            .blocking_dispatch(&mut self.state)
            .map_err(|error| source_error(InputMethodError::Dispatch(error)))?;
        if let Some(error) = self.state.error.take() {
            return Err(source_error(error));
        }
        if let Some(event) = self.state.events.pop_front() {
            if matches!(event, InputEvent::Reset) {
                let injector = self
                    .key_pass_through
                    .as_mut()
                    .map(|injector| injector.as_mut() as &mut dyn TextInjector);
                pass_through_pending_keys(&mut self.state.pending_key_pass_through, injector)?;
            }
            Ok(Some(event))
        } else {
            Ok(None)
        }
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
        self.connection.flush().map_err(|error| InjectorError {
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
        self.connection.flush().map_err(|error| InjectorError {
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
        self.connection.flush().map_err(|error| InjectorError {
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

    fn next_event(&mut self) -> Result<InputEvent, InputSourceError> {
        loop {
            if let Some(error) = self.state.error.take() {
                return Err(source_error(error));
            }
            if let Some(event) = self.state.events.pop_front() {
                if matches!(event, InputEvent::Reset) {
                    let injector = self
                        .key_pass_through
                        .as_mut()
                        .map(|injector| injector.as_mut() as &mut dyn TextInjector);
                    pass_through_pending_keys(&mut self.state.pending_key_pass_through, injector)?;
                }
                return Ok(event);
            }
            self.event_queue
                .blocking_dispatch(&mut self.state)
                .map_err(|error| source_error(InputMethodError::Dispatch(error)))?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xkbcommon_rs::xkb_keymap::RuleNames;

    struct RecordingInjector {
        calls: Vec<(u32, Modifiers)>,
        fail: bool,
    }

    fn default_state() -> State {
        let keymap = Keymap::new_from_names(Context::new(0).unwrap(), None, 0).unwrap();
        State::new(keymap)
    }

    fn keymap_state(layout: &str, variant: &str) -> Option<State> {
        let names = RuleNames::new("", "", layout, variant, "");
        Keymap::new_from_names(Context::new(0).unwrap(), Some(names), 0)
            .ok()
            .map(State::new)
    }

    impl TextInjector for RecordingInjector {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn erase(&mut self, _: &str) -> Result<(), InjectorError> {
            Ok(())
        }

        fn insert(&mut self, _: &str) -> Result<(), InjectorError> {
            Ok(())
        }

        fn inject_key_with_modifiers(
            &mut self,
            keycode: u32,
            modifiers: Modifiers,
        ) -> Result<(), InjectorError> {
            if self.fail {
                return Err(InjectorError {
                    backend: self.name(),
                    message: "synthetic failure".into(),
                    retryable: true,
                });
            }
            self.calls.push((keycode, modifiers));
            Ok(())
        }
    }

    #[test]
    fn special_keysyms_become_engine_events() {
        assert_eq!(
            classify_keysym(xkeysym::key::BackSpace),
            Some(InputEvent::Backspace)
        );
        assert_eq!(
            classify_keysym(xkeysym::key::Return),
            Some(InputEvent::Delimiter('\n'))
        );
        assert_eq!(
            classify_keysym(xkeysym::key::KP_Enter),
            Some(InputEvent::Delimiter('\n'))
        );
        assert_eq!(
            classify_keysym(xkeysym::key::Tab),
            Some(InputEvent::Delimiter('\t'))
        );
        assert_eq!(classify_keysym(xkeysym::key::Escape), None);
    }

    #[test]
    fn xkb_state_transition_applies_modifiers_before_next_key() {
        let mut state = default_state();

        assert!(matches!(
            key_action_and_update(&mut state, 42, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Ignore)
        ));
        assert!(matches!(
            key_action_and_update(&mut state, 30, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Text(text)) if text == "A"
        ));
        assert!(key_action_and_update(&mut state, 30, wl_keyboard::KeyState::Released).is_none());
        assert!(key_action_and_update(&mut state, 42, wl_keyboard::KeyState::Released).is_none());
    }

    #[test]
    fn ctrl_shortcut_is_passed_through_instead_of_committed_as_text() {
        let mut state = default_state();

        assert!(matches!(
            key_action_and_update(&mut state, 29, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Ignore)
        ));
        assert_eq!(
            active_modifiers(&state),
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            }
        );
        assert!(matches!(
            key_action_and_update(&mut state, 46, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Unsupported)
        ));
    }

    #[test]
    fn common_control_shortcuts_are_passed_through_before_utf8_conversion() {
        for (name, key) in [("Ctrl+C", 46), ("Ctrl+V", 47), ("Ctrl+Z", 44)] {
            let mut state = default_state();
            assert!(matches!(
                key_action_and_update(&mut state, 29, wl_keyboard::KeyState::Pressed),
                Some(KeyAction::Ignore)
            ));
            assert!(
                matches!(
                    key_action_and_update(&mut state, key, wl_keyboard::KeyState::Pressed),
                    Some(KeyAction::Unsupported)
                ),
                "{name} must pass through instead of committing control text"
            );
        }
    }

    #[test]
    fn ctrl_shift_shortcuts_are_passed_through_with_modifiers_preserved() {
        let mut state = default_state();
        assert!(matches!(
            key_action_and_update(&mut state, 29, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Ignore)
        ));
        assert!(matches!(
            key_action_and_update(&mut state, 42, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Ignore)
        ));
        assert_eq!(
            active_modifiers(&state),
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            }
        );
        assert!(matches!(
            key_action_and_update(&mut state, 20, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Unsupported)
        ));
    }

    #[test]
    fn alt_and_super_shortcuts_are_passed_through_before_utf8_conversion() {
        for (name, modifier, key) in [("Alt+F4", 56, 62), ("Super+L", 125, 38)] {
            let mut state = default_state();
            assert!(matches!(
                key_action_and_update(&mut state, modifier, wl_keyboard::KeyState::Pressed),
                Some(KeyAction::Ignore)
            ));
            assert!(
                matches!(
                    key_action_and_update(&mut state, key, wl_keyboard::KeyState::Pressed),
                    Some(KeyAction::Unsupported)
                ),
                "{name} must pass through instead of becoming text"
            );
        }
    }

    #[test]
    fn caps_lock_and_shift_remain_text_producing_modifiers() {
        let mut shifted = default_state();
        assert!(matches!(
            key_action_and_update(&mut shifted, 42, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Ignore)
        ));
        assert!(matches!(
            key_action_and_update(&mut shifted, 30, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Text(text)) if text == "A"
        ));

        let mut caps = default_state();
        assert!(matches!(
            key_action_and_update(&mut caps, 58, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Ignore)
        ));
        assert!(matches!(
            key_action_and_update(&mut caps, 30, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Text(text)) if text == "A"
        ));
    }

    #[test]
    fn altgr_text_is_not_treated_as_shortcut_alt_when_layout_supports_it() {
        let Some(mut state) = keymap_state("de", "") else {
            return;
        };
        let _ = key_action_and_update(&mut state, 100, wl_keyboard::KeyState::Pressed);
        let action = key_action_and_update(&mut state, 16, wl_keyboard::KeyState::Pressed);
        if let Some(KeyAction::Text(text)) = action {
            // On systems with complete xkeyboard-config, AltGr+Q produces @
            assert_eq!(text, "@");
        } else {
            // Some minimal CI images lack complete xkeyboard-config data or
            // map right Alt differently. The required invariant is that AltGr
            // must not be classified as shortcut Alt by `active_modifiers`.
            assert!(!active_modifiers(&state).alt);
        }
    }

    #[test]
    fn pending_pass_through_preserves_modifiers() {
        let mut pending = VecDeque::from([PendingKeyPassThrough {
            keycode: 105,
            modifiers: Modifiers {
                alt: true,
                shift: true,
                ..Modifiers::default()
            },
        }]);
        let mut injector = RecordingInjector {
            calls: Vec::new(),
            fail: false,
        };

        pass_through_pending_keys(&mut pending, Some(&mut injector)).unwrap();

        assert!(pending.is_empty());
        assert_eq!(
            injector.calls,
            vec![(
                105,
                Modifiers {
                    alt: true,
                    shift: true,
                    ..Modifiers::default()
                }
            )]
        );
    }

    #[test]
    fn missing_pass_through_injector_is_reported_not_silent() {
        let mut pending = VecDeque::from([PendingKeyPassThrough {
            keycode: 1,
            modifiers: Modifiers::default(),
        }]);

        let error = pass_through_pending_keys(&mut pending, None).unwrap_err();

        assert!(error.retryable);
        assert!(error.message.contains("no key pass-through injector"));
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn pass_through_injector_failure_is_reported_not_silent() {
        let mut pending = VecDeque::from([PendingKeyPassThrough {
            keycode: 1,
            modifiers: Modifiers::default(),
        }]);
        let mut injector = RecordingInjector {
            calls: Vec::new(),
            fail: true,
        };

        let error = pass_through_pending_keys(&mut pending, Some(&mut injector)).unwrap_err();

        assert!(error.retryable);
        assert!(error.message.contains("synthetic failure"));
    }

    #[test]
    fn overflowing_protocol_keycode_fails_closed() {
        let mut state = default_state();
        assert!(matches!(
            key_action_and_update(&mut state, u32::MAX, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Unsupported)
        ));
    }

    #[test]
    fn unrelated_keysym_is_left_for_utf8_conversion() {
        assert_eq!(classify_keysym('a' as u32), None);
    }

    #[test]
    fn password_and_unknown_content_purposes_are_sensitive() {
        use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
            ContentHint, ContentPurpose,
        };

        assert!(content_type_is_sensitive(
            WEnum::Value(ContentHint::None),
            WEnum::Value(ContentPurpose::Password),
        ));
        assert!(content_type_is_sensitive(
            WEnum::Value(ContentHint::None),
            WEnum::Unknown(255),
        ));
        assert!(content_type_is_sensitive(
            WEnum::Value(ContentHint::HiddenText),
            WEnum::Value(ContentPurpose::Normal),
        ));
        assert!(content_type_is_sensitive(
            WEnum::Value(ContentHint::SensitiveData),
            WEnum::Value(ContentPurpose::Normal),
        ));
        assert!(!content_type_is_sensitive(
            WEnum::Value(ContentHint::None),
            WEnum::Value(ContentPurpose::Normal),
        ));
    }

    #[test]
    fn input_method_commit_size_is_checked_before_injection() {
        assert!(validate_commit_text(&"a".repeat(MAX_COMMIT_TEXT_BYTES)).is_ok());
        assert!(validate_commit_text(&"a".repeat(MAX_COMMIT_TEXT_BYTES + 1)).is_err());
    }

    #[test]
    fn input_method_control_characters_are_rejected_before_injection() {
        assert!(validate_commit_text("\u{0001}").is_err());
        assert!(validate_commit_text("safe\ntext\r\t").is_ok());
    }

    #[test]
    fn connection_poll_errors_are_terminal() {
        use rustix::event::PollFlags;

        assert!(connection_poll_failed(PollFlags::ERR));
        assert!(connection_poll_failed(PollFlags::HUP));
        assert!(connection_poll_failed(PollFlags::NVAL));
        assert!(!connection_poll_failed(PollFlags::IN));
    }

    #[test]
    fn transport_failures_are_retryable_but_protocol_failures_are_not() {
        assert!(InputMethodError::Unavailable.is_retryable());
        assert!(InputMethodError::Transport("socket closed".into()).is_retryable());
        assert!(InputMethodError::Timeout("startup".into()).is_retryable());
        assert!(!InputMethodError::Protocol("unsupported key".into()).is_retryable());
        assert!(!InputMethodError::Keymap("invalid".into()).is_retryable());
    }

    #[test]
    fn backspace_uses_utf8_byte_length() {
        let surrounding = SurroundingText {
            text: "café".into(),
            cursor: 5,
            anchor: 5,
        };
        assert_eq!(backspace_delete_lengths(Some(&surrounding)), Some((2, 0)));
    }

    #[test]
    fn backspace_deletes_whole_grapheme_cluster() {
        // "e" (1 byte) + combining acute accent U+0301 (2 bytes) is a single
        // extended grapheme cluster; Backspace must remove both codepoints,
        // not just the trailing combining mark.
        let surrounding = SurroundingText {
            text: "e\u{0301}".into(),
            cursor: 3,
            anchor: 3,
        };
        assert_eq!(backspace_delete_lengths(Some(&surrounding)), Some((3, 0)));
    }

    #[test]
    fn backspace_deletes_selection_or_fails_closed() {
        let selected = SurroundingText {
            text: "hello".into(),
            cursor: 5,
            anchor: 2,
        };
        assert_eq!(backspace_delete_lengths(Some(&selected)), Some((3, 0)));
        assert_eq!(backspace_delete_lengths(None), None);
    }

    #[test]
    fn selection_backspace_is_a_boundary_for_matching() {
        let selected = SurroundingText {
            text: "hello".into(),
            cursor: 5,
            anchor: 2,
        };
        let collapsed = SurroundingText {
            text: "hello".into(),
            cursor: 5,
            anchor: 5,
        };
        assert!(surrounding_has_selection(Some(&selected)));
        assert!(!surrounding_has_selection(Some(&collapsed)));
        assert!(!surrounding_has_selection(None));
    }

    #[test]
    fn empty_surrounding_text_does_not_emit_matcher_backspace() {
        let empty = SurroundingText {
            text: String::new(),
            cursor: 0,
            anchor: 0,
        };
        assert_eq!(backspace_delete_lengths(Some(&empty)), Some((0, 0)));
        assert!(!surrounding_has_selection(Some(&empty)));
        assert_eq!(
            matcher_event_for_deletion(0, 0, false),
            InputEvent::EndOfInput
        );
        assert_eq!(
            matcher_event_for_deletion(2, 0, false),
            InputEvent::Backspace
        );
    }

    #[test]
    fn unsupported_key_clears_pending_matching_state() {
        assert_eq!(matcher_event_for_unsupported_key(), InputEvent::Reset);
    }

    #[test]
    fn deactivation_disables_matching() {
        assert_eq!(
            deactivation_event(),
            InputEvent::FocusChanged { sensitive: true }
        );
    }

    #[test]
    fn replacement_context_must_match_exact_utf8_trigger() {
        let surrounding = SurroundingText {
            text: "café:x".into(),
            cursor: 7,
            anchor: 7,
        };
        assert!(surrounding_ends_with_trigger(Some(&surrounding), ":x"));
        assert!(!surrounding_ends_with_trigger(Some(&surrounding), ":y"));
        assert!(!surrounding_ends_with_trigger(None, ":x"));
    }

    #[test]
    fn forwarded_text_updates_context_before_immediate_replacement() {
        let mut surrounding = Some(SurroundingText {
            text: ":".into(),
            cursor: 1,
            anchor: 1,
        });

        optimistic_commit(&mut surrounding, "x");

        assert!(surrounding_ends_with_trigger(surrounding.as_ref(), ":x"));
        optimistic_replace(&mut surrounding, ":x", "expanded").unwrap();
        assert_eq!(surrounding.as_ref().unwrap().text, "expanded");
    }

    #[test]
    fn replacement_context_rejects_selection_and_invalid_cursor() {
        let selected = SurroundingText {
            text: ":x".into(),
            cursor: 2,
            anchor: 0,
        };
        assert!(!surrounding_ends_with_trigger(Some(&selected), ":x"));
        let invalid = SurroundingText {
            text: ":x".into(),
            cursor: 1,
            anchor: 1,
        };
        assert!(!surrounding_ends_with_trigger(Some(&invalid), ":x"));
    }

    #[test]
    fn protocol_event_queue_is_bounded_and_fails_closed() {
        let mut state = StateData::new();
        for _ in 0..MAX_QUEUED_EVENTS {
            state.queue_event(InputEvent::Text("x".into()));
        }
        state.queue_event(InputEvent::Text("overflow".into()));
        assert_eq!(state.events.len(), 1);
        assert_eq!(state.events.front(), Some(&InputEvent::Reset));
    }

    #[test]
    fn focus_policy_events_supersede_queued_keyboard_events() {
        let mut state = StateData::new();
        state.queue_event(InputEvent::Text("stale".into()));
        state.queue_event(InputEvent::FocusChanged { sensitive: true });
        assert_eq!(
            state.events.as_slices().0,
            &[InputEvent::FocusChanged { sensitive: true }]
        );
    }
}
