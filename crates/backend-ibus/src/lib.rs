//! Protocol-neutral IBus engine adapter.
//!
//! IBus owns the key event loop and asks an engine whether each key was
//! handled.  This adapter deliberately contains no D-Bus code: a small host
//! (the native IBus service or a test harness) translates [`IbusAction`]s to
//! IBus signals. Keeping that boundary explicit makes the expansion behavior
//! testable and prevents D-Bus threading details from entering the matcher.

use tracing::{error, warn};
use wayexpand_core::{Config, ExpansionEngine, InputEvent, OrganizationPolicy};

// IBus' public C API defines IBUS_RELEASE_MASK as (1 << 30). The ibus-rs
// crate is not used because it adds a mandatory libdbus system dependency.
const IBUS_RELEASE_MASK: u32 = 1 << 30;
const IBUS_BACKEND_NAME: &str = "input-method-v2";

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
    policy: OrganizationPolicy,
}

impl IbusEngineAdapter {
    pub fn new(engine: ExpansionEngine) -> Self {
        Self::with_policy(engine, OrganizationPolicy::default())
    }

    pub fn with_policy(mut engine: ExpansionEngine, policy: OrganizationPolicy) -> Self {
        apply_policy_to_engine(&mut engine, &policy);
        Self {
            engine,
            enabled: true,
            policy,
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
        if self.engine.async_commands_enabled() && !engine.enable_async_commands() {
            warn!(
                "IBus asynchronous workers could not restart after configuration reload; using synchronous fallback"
            );
        }
        apply_policy_to_engine(&mut engine, &self.policy);
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
        let mut policy_blocked = false;
        for result in results {
            if let Some(violation) = self.policy.expansion_policy_violation(
                result.insert.len(),
                result.command_backed,
                IBUS_BACKEND_NAME,
            ) {
                if self.policy.safe_mode {
                    error!(
                        audit_prefix = %self.policy.audit_prefix,
                        violation = %violation,
                        "IBus expansion blocked by organization policy"
                    );
                    policy_blocked = true;
                    continue;
                }
                warn!(
                    audit_prefix = %self.policy.audit_prefix,
                    violation = %violation,
                    "IBus expansion violates organization policy; audit mode permits it"
                );
            }
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

        if policy_blocked {
            // The trigger characters have already been committed as ordinary
            // IBus text. Do not leave a partial matcher after a blocked
            // replacement; the current key is handled by the normal fallback
            // below and the next trigger starts from a clean boundary.
            self.engine.process(InputEvent::Reset);
            actions.clear();
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

fn apply_policy_to_engine(engine: &mut ExpansionEngine, policy: &OrganizationPolicy) {
    engine.set_commands_disabled(policy.safe_mode && policy.disable_commands);
    engine.set_title_matching_disabled(policy.disable_title_matching);
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
    use wayexpand_core::{Config, ExpansionEngine, OrganizationPolicy};

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

    fn policy_adapter(policy: OrganizationPolicy) -> IbusEngineAdapter {
        let config: Config = toml::from_str(
            r#"[[expansion]]
trigger = ":sig"
replacement = "signature"
"#,
        )
        .unwrap();
        IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy)
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

    #[test]
    fn engine_instances_do_not_share_matcher_state() {
        let mut first = adapter();
        let mut second = adapter();

        for character in ":si".chars() {
            first.process_key_event(character as u32, 0, 0);
        }

        // A second input context must not inherit the first context's partial
        // trigger. Its ordinary key is committed unchanged.
        let isolated = second.process_key_event('g' as u32, 0, 0);
        assert_eq!(isolated.actions, vec![IbusAction::CommitText("g".into())]);

        let completed = first.process_key_event('g' as u32, 0, 0);
        assert!(completed
            .actions
            .contains(&IbusAction::CommitText("signature".into())));
    }

    #[test]
    fn safe_organization_policy_blocks_disallowed_ibus_expansion() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["libei".into()],
            ..Default::default()
        });
        for character in ":sig".chars() {
            let result = adapter.process_key_event(character as u32, 0, 0);
            if character == 'g' {
                assert_eq!(result.actions, vec![IbusAction::CommitText("g".into())]);
            }
        }
    }

    #[test]
    fn audit_organization_policy_allows_but_reports_disallowed_ibus_expansion() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: false,
            allowed_backends: vec!["libei".into()],
            ..Default::default()
        });
        for character in ":sig".chars() {
            let result = adapter.process_key_event(character as u32, 0, 0);
            if character == 'g' {
                assert!(result
                    .actions
                    .contains(&IbusAction::CommitText("signature".into())));
            }
        }
    }
}
