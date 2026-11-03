//! XKB layout refresh and raw key to matcher-event translation.

use super::*;
use wayexpand_backend_input_method::{key_action, key_action_and_update, key_chord, KeyAction};
use wayland_client::protocol::wl_keyboard::KeyState;
use xkbcommon_rs::{xkb_keymap::RuleNames, Context, Keymap, State};

impl EvdevSource {
    /// Translates one raw evdev event into zero or more `InputEvent`s,
    /// pushed directly onto `self.pending`. A key press can yield both a
    /// hotkey chord and an ordinary matcher event (mirrors how
    /// `backend-input-method` reports both from the same key press).
    #[cfg(test)]
    pub(super) fn translate(&mut self, event: evdev::InputEvent) {
        self.translate_for_device(0, event);
    }

    pub(super) fn translate_for_device(&mut self, device_index: usize, event: evdev::InputEvent) {
        let evdev::EventSummary::Key(_, key_code, value) = event.destructure() else {
            return;
        };
        let keycode = u32::from(key_code.code());
        if value == 2 {
            if !self.refresh_layout() {
                return;
            }
            let action = key_action(&self.state, keycode);
            self.queue_action(action);
            return;
        }
        let key_state = match value {
            0 => KeyState::Released,
            1 => KeyState::Pressed,
            _ => return,
        };
        let layout_available = self.refresh_layout();
        if self.pressed.len() <= device_index {
            self.pressed.resize_with(device_index + 1, HashSet::new);
        }
        match key_state {
            KeyState::Pressed => {
                self.pressed[device_index].insert(keycode);
            }
            _ => {
                self.pressed[device_index].remove(&keycode);
            }
        }
        // Keep physical key ownership for output-interruption safety even
        // when no trustworthy layout is available. Do not infer characters,
        // delimiters, or hotkeys from a startup-only keymap.
        if !layout_available {
            if key_state == KeyState::Pressed {
                self.queue_event(InputEvent::Reset);
            }
            return;
        }
        if key_state == KeyState::Pressed {
            if let Some(chord) = key_chord(&self.state, keycode) {
                self.queue_event(InputEvent::Key(chord));
            }
        }
        let action = key_action_and_update(&mut self.state, keycode, key_state);
        self.queue_action(action);
    }

    pub(super) fn refresh_layout(&mut self) -> bool {
        let Some(snapshot) = self.layout_provider.snapshot() else {
            if self.layout_available {
                self.layout_available = false;
                self.layout_generation = None;
                self.queue_event(InputEvent::Reset);
            }
            return false;
        };
        if self.layout_generation == Some(snapshot.generation) {
            return true;
        }
        let context = match Context::new(0) {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(%error, "cannot initialize compositor keyboard layout; disabling evdev matching");
                self.layout_available = false;
                self.queue_event(InputEvent::Reset);
                return false;
            }
        };
        let keymap = match Keymap::new_from_names(
            context,
            Some(RuleNames {
                rules: snapshot.rules,
                model: snapshot.model,
                layout: snapshot.layout,
                variant: snapshot.variant,
                options: snapshot.options,
            }),
            0,
        ) {
            Ok(keymap) => keymap,
            Err(error) => {
                tracing::warn!(%error, "cannot load compositor keyboard layout; disabling evdev matching");
                self.layout_available = false;
                self.queue_event(InputEvent::Reset);
                return false;
            }
        };
        let locked_modifiers = self
            .state
            .serialize_mods(xkbcommon_rs::xkb_state::StateComponent::MODS_LOCKED);
        self.state = State::new(keymap);
        self.state
            .update_mask(0, 0, locked_modifiers, 0, 0, snapshot.group as usize);
        let mut held_keys = self
            .pressed
            .iter()
            .flat_map(|keys| keys.iter().copied())
            .collect::<Vec<_>>();
        held_keys.sort_unstable();
        for keycode in held_keys {
            if let Some(xkb_keycode) = keycode.checked_add(8) {
                self.state.update_key(xkb_keycode, KeyDirection::Down);
            }
        }
        self.layout_generation = Some(snapshot.generation);
        self.layout_available = true;
        // A layout switch invalidates partially composed characters and any
        // trigger assembled under the previous keymap.
        self.queue_event(InputEvent::Reset);
        true
    }

    fn queue_action(&mut self, action: Option<KeyAction>) {
        let translated = match action {
            Some(KeyAction::Delete) => Some(InputEvent::Backspace),
            // Commit carries "\n"/"\t" for the app, which already received
            // the real key natively; the matcher only needs the boundary.
            Some(KeyAction::Commit(text)) => text.chars().next().map(InputEvent::Delimiter),
            Some(KeyAction::Character(character)) => Some(InputEvent::TextChar(character)),
            Some(KeyAction::Text(text)) => Some(InputEvent::Text(text)),
            Some(KeyAction::Ignore) | None => None,
            Some(KeyAction::Unsupported) => Some(InputEvent::Reset),
        };
        if let Some(event) = translated {
            self.queue_event(event);
        }
    }
}
