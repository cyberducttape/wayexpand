//! Protocol-neutral IBus engine adapter.
//!
//! IBus owns the key event loop and asks an engine whether each key was
//! handled.  This adapter deliberately contains no D-Bus code: a small host
//! (the native IBus service or a test harness) translates [`IbusAction`]s to
//! IBus signals. Keeping that boundary explicit makes the expansion behavior
//! testable and prevents D-Bus threading details from entering the matcher.

use wayexpand_core::{Config, ExpansionEngine, InputEvent};

// IBus' public C API defines IBUS_RELEASE_MASK as (1 << 30). The ibus-rs
// crate is not used because it adds a mandatory libdbus system dependency.
const IBUS_RELEASE_MASK: u32 = 1 << 30;

mod service;

pub use service::{run_service, IbusServiceError};

/// Output operations an IBus host must emit on the engine's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IbusAction {
    /// Delete `nchars` Unicode scalar values before the insertion cursor.
    DeleteSurroundingText { nchars: u32 },
    /// Commit text at the application cursor.
    CommitText(String),
}

/// Result of processing one IBus key press.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IbusKeyResult {
    /// Whether the engine consumed the key. If false, the IBus host must let
    /// the client process the original key event.
    pub handled: bool,
    pub actions: Vec<IbusAction>,
}

/// Core-backed IBus engine state.
pub struct IbusEngineAdapter {
    engine: ExpansionEngine,
    enabled: bool,
}

impl IbusEngineAdapter {
    pub fn new(engine: ExpansionEngine) -> Self {
        Self {
            engine,
            enabled: true,
        }
    }

    pub fn engine(&self) -> &ExpansionEngine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut ExpansionEngine {
        &mut self.engine
    }

    pub fn replace_config(&mut self, config: Config) -> Result<(), wayexpand_core::ConfigError> {
        let mut engine = ExpansionEngine::new(config)?;
        engine.set_user_paused(self.engine.is_user_paused());
        engine.set_sensitive_focus(self.engine.is_sensitive_focus());
        engine.set_current_window(self.engine.current_window().cloned());
        engine.set_commands_disabled(self.engine.commands_disabled());
        engine.set_title_matching_disabled(self.engine.title_matching_disabled());
        engine.set_reinsert_terminators(self.engine.reinserts_terminators());
        if self.engine.async_commands_enabled() {
            engine.enable_async_commands();
        }
        self.engine = engine;
        Ok(())
    }

    pub fn focus_in(&mut self) {
        self.enabled = true;
        self.engine.process(InputEvent::Reset);
    }

    pub fn focus_out(&mut self) {
        self.enabled = false;
        self.engine.process(InputEvent::Reset);
    }

    pub fn reset(&mut self) {
        self.engine.process(InputEvent::Reset);
    }

    /// Process an IBus key press. `keyval` is an XKB keysym, as specified by
    /// `org.freedesktop.IBus.Engine.ProcessKeyEvent`.
    pub fn process_key_event(&mut self, keyval: u32, _keycode: u32, state: u32) -> IbusKeyResult {
        // IBus delivers both press and release events through this method.
        // Releases carry IBUS_RELEASE_MASK and must not be interpreted as a
        // second printable character or delimiter.
        // IBus' public C API defines IBUS_RELEASE_MASK as (1 << 30).
        // Keep this named at the protocol boundary; the ibus-rs crate cannot
        // be used here without adding a mandatory libdbus system dependency.
        const IBUS_RELEASE_MASK: u32 = 1 << 30;
        if state & IBUS_RELEASE_MASK != 0 {
            return IbusKeyResult::default();
        }

        if !self.enabled {
            return IbusKeyResult::default();
        }
        // When the core has disabled capture (password fields or an explicit
        // pause), do not consume or commit anything on the client's behalf.
        // The toolkit must receive the original key unchanged.
        if self.engine.is_sensitive_focus() || self.engine.is_user_paused() {
            return IbusKeyResult::default();
        }

        // IBus modifier flags use the same low bits as X11. Do not consume
        // shortcuts or navigation keys: clearing the matcher state is safer
        // than allowing a trigger to span an unrelated command.
        const CONTROL_MASK: u32 = 1 << 2;
        const ALT_MASK: u32 = 1 << 3;
        const SUPER_MASK: u32 = 1 << 26;
        if state & (CONTROL_MASK | ALT_MASK | SUPER_MASK) != 0 {
            self.engine.process(InputEvent::Reset);
            return IbusKeyResult::default();
        }

        if keyval == xkeysym::key::BackSpace {
            self.engine.process(InputEvent::Backspace);
            return IbusKeyResult::default();
        }

        let Some(character) = keysym_to_char(keyval) else {
            self.engine.process(InputEvent::Reset);
            return IbusKeyResult::default();
        };

        let delimiter =
            character.is_whitespace() || !character.is_alphanumeric() && character != '_';
        let event = if delimiter {
            InputEvent::Delimiter(character)
        } else {
            InputEvent::Text(character.to_string())
        };
        let results = self.engine.process(event);
        let mut actions = Vec::new();
        for result in results {
            actions.push(IbusAction::DeleteSurroundingText {
                // IBus invokes the engine before forwarding the key to the
                // client. The delimiter is not in the client's surrounding
                // text yet; delete only the trigger and commit the delimiter
                // together with the replacement.
                nchars: result.matched_text.chars().count() as u32,
            });
            let mut replacement = result.insert;
            if let Some(character) = result.reinsert_after {
                replacement.push(character);
            }
            actions.push(IbusAction::CommitText(replacement));
        }

        if actions.is_empty() {
            // IBus engines must commit ordinary text themselves once they
            // claim the key; this avoids duplicate delivery to the client.
            if !delimiter {
                actions.push(IbusAction::CommitText(character.to_string()));
                return IbusKeyResult {
                    handled: true,
                    actions,
                };
            }
            return IbusKeyResult {
                handled: false,
                actions,
            };
        }
        IbusKeyResult {
            handled: true,
            actions,
        }
    }
}

/// Convert the printable XKB keysyms that IBus supplies to Unicode.
/// Keysyms in the Unicode range are intentionally handled without a keymap;
/// layout/dead-key composition has already happened before IBus receives the
/// event from the toolkit.
fn keysym_to_char(keysym: u32) -> Option<char> {
    match keysym {
        xkeysym::key::space => Some(' '),
        xkeysym::key::Tab => Some('\t'),
        xkeysym::key::Return | xkeysym::key::KP_Enter => Some('\n'),
        0x0100_0000..=0x0110_ffff => char::from_u32(keysym - 0x0100_0000),
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(keysym),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayexpand_core::{Config, ExpansionEngine};

    fn adapter() -> IbusEngineAdapter {
        let config: Config = toml::from_str(
            r#"[[expansion]]
trigger = ":sig"
replacement = "signature"
"#,
        )
        .unwrap();
        IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap())
    }

    fn boundary_adapter() -> IbusEngineAdapter {
        let config: Config = toml::from_str(
            r#"[[expansion]]
trigger = ":sig"
replacement = "signature"
match_mode = "word-boundary"
"#,
        )
        .unwrap();
        IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap())
    }

    #[test]
    fn ordinary_text_is_committed_immediately() {
        let mut adapter = adapter();
        let result = adapter.process_key_event('a' as u32, 0, 0);
        assert!(result.handled);
        assert_eq!(result.actions, vec![IbusAction::CommitText("a".into())]);
    }

    #[test]
    fn trigger_is_deleted_and_replacement_committed() {
        let mut adapter = adapter();
        for character in ":sig".chars() {
            let result = adapter.process_key_event(character as u32, 0, 0);
            if character == 'g' {
                assert_eq!(
                    result.actions,
                    vec![
                        IbusAction::DeleteSurroundingText { nchars: 4 },
                        IbusAction::CommitText("signature".into())
                    ]
                );
            }
        }
    }

    #[test]
    fn delimiter_is_reinserted_atomically() {
        let mut adapter = boundary_adapter();
        for character in ":sig ".chars() {
            let result = adapter.process_key_event(character as u32, 0, 0);
            if character == ' ' {
                assert_eq!(
                    result.actions,
                    vec![
                        IbusAction::DeleteSurroundingText { nchars: 4 },
                        IbusAction::CommitText("signature ".into())
                    ]
                );
            }
        }
    }

    #[test]
    fn printable_key_release_is_not_committed() {
        let mut adapter = adapter();

        assert_eq!(
            adapter.process_key_event('a' as u32, 0, IBUS_RELEASE_MASK),
            IbusKeyResult::default()
        );
    }

    #[test]
    fn trigger_key_release_does_not_advance_matcher() {
        let mut adapter = adapter();

        for character in ":si".chars() {
            adapter.process_key_event(character as u32, 0, 0);
        }
        assert_eq!(
            adapter.process_key_event('i' as u32, 0, IBUS_RELEASE_MASK),
            IbusKeyResult::default()
        );

        let result = adapter.process_key_event('g' as u32, 0, 0);
        assert_eq!(
            result.actions,
            vec![
                IbusAction::DeleteSurroundingText { nchars: 4 },
                IbusAction::CommitText("signature".into())
            ]
        );
    }

    #[test]
    fn modifier_key_release_does_not_reset_valid_buffer() {
        const CONTROL_MASK: u32 = 1 << 2;
        let mut adapter = adapter();

        for character in ":si".chars() {
            adapter.process_key_event(character as u32, 0, 0);
        }
        assert_eq!(
            adapter
                .process_key_event(xkeysym::key::Control_L, 0, CONTROL_MASK | IBUS_RELEASE_MASK,),
            IbusKeyResult::default()
        );

        let result = adapter.process_key_event('g' as u32, 0, 0);
        assert!(result
            .actions
            .contains(&IbusAction::CommitText("signature".into())));
    }
}
