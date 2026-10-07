use super::*;

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

pub(super) fn active_modifiers(keyboard_state: &State) -> Modifiers {
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

pub(super) fn key_is_modifier(keyboard_state: &State, key: u32) -> bool {
    key.checked_add(8)
        .and_then(|keycode| keyboard_state.key_get_one_sym(keycode))
        .is_some_and(|keysym| is_modifier_keysym(keysym.raw()))
}
