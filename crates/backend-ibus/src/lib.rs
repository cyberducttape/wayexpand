//! Protocol-neutral IBus engine adapter.
//!
//! IBus owns the key event loop and asks an engine whether each key was
//! handled.  This adapter deliberately contains no D-Bus code: a small host
//! (the native IBus service or a test harness) translates [`IbusAction`]s to
//! IBus signals. Keeping that boundary explicit makes the expansion behavior
//! testable and prevents D-Bus threading details from entering the matcher.

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use tracing::{error, warn};
use wayexpand_core::{
    Config, ExpansionEngine, ExpansionResult, InputEvent, OrganizationPolicy,
    PendingExpansionDispatch,
};

// IBus' public C API defines IBUS_RELEASE_MASK as (1 << 30). The ibus-rs
// crate is not used because it adds a mandatory libdbus system dependency.
const IBUS_RELEASE_MASK: u32 = 1 << 30;
const IBUS_BACKEND_NAME: &str = "input-method-v2";

/// Return whether the installed IBus component can be discovered by setup.
/// This is intentionally an installation/provisioning probe, not an
/// end-to-end typing guarantee.
pub fn engine_available() -> bool {
    if !executable_in_path("ibus") || !executable_in_path("wayexpand-ibus") {
        return false;
    }

    if component_file_present_in(&ibus_component_directories()) {
        return true;
    }

    ibus_registry_contains_engine()
}

fn ibus_registry_contains_engine() -> bool {
    let Ok(mut child) = Command::new("ibus")
        .args(["list-engine"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map(|output| {
                        output.status.success()
                            && String::from_utf8_lossy(&output.stdout)
                                .lines()
                                .any(|line| line.contains("wayexpand"))
                    })
                    .unwrap_or(false);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn executable_in_path(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|directory| {
        let candidate = directory.join(name);
        fs::metadata(candidate)
            .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

fn component_file_present_in(directories: &[PathBuf]) -> bool {
    directories
        .iter()
        .map(|directory| directory.join("wayexpand.xml"))
        .any(|path| path.is_file())
}

fn ibus_component_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        directories.push(PathBuf::from(data_home).join("ibus/component"));
    } else if let Some(home) = env::var_os("HOME") {
        directories.push(PathBuf::from(home).join(".local/share/ibus/component"));
    }
    directories.push(PathBuf::from("/usr/local/share/ibus/component"));
    directories.push(PathBuf::from("/usr/share/ibus/component"));
    directories
}

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
    pub fn new(mut engine: ExpansionEngine) -> Self {
        let policy = OrganizationPolicy::default();
        apply_policy_to_engine(&mut engine, &policy);
        Self {
            engine,
            enabled: true,
            policy,
        }
    }

    pub fn with_policy(
        mut engine: ExpansionEngine,
        policy: OrganizationPolicy,
    ) -> Result<Self, wayexpand_core::ConfigError> {
        engine.apply_administrator_policy(&policy)?;
        apply_policy_to_engine(&mut engine, &policy);
        Ok(Self {
            engine,
            enabled: true,
            policy,
        })
    }

    pub fn engine(&self) -> &ExpansionEngine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut ExpansionEngine {
        &mut self.engine
    }

    pub fn drain_completed_commands(&mut self) -> Vec<IbusAction> {
        self.engine
            .drain_completed_commands()
            .into_iter()
            .filter_map(|result| {
                if let Some(violation) = self.policy.expansion_policy_violation(
                    result.insert.len(),
                    result.command_backed,
                    IBUS_BACKEND_NAME,
                ) {
                    if self.policy.safe_mode {
                        self.engine.restore_deferred_match(&result.matched_text);
                        error!(
                            audit_prefix = %self.policy.audit_prefix,
                            violation = %violation,
                            "IBus completed expansion blocked by organization policy"
                        );
                        return None;
                    }
                    warn!(
                        audit_prefix = %self.policy.audit_prefix,
                        violation = %violation,
                        "IBus completed expansion violates organization policy; audit mode permits it"
                    );
                }
                let actions = expansion_actions(&result, true);
                // IBus has no injection acknowledgement. The protocol action
                // batch is the adapter's commit point; the D-Bus service
                // resets the engine if emitting it fails.
                self.engine.commit_applied_expansion(&result);
                Some(actions)
            })
            .flatten()
            .collect()
    }

    pub fn replace_config(&mut self, config: Config) -> Result<(), wayexpand_core::ConfigError> {
        let mut engine = ExpansionEngine::new(config)?;
        engine.apply_administrator_policy(&self.policy)?;
        engine.set_user_paused(self.engine.is_user_paused());
        engine.set_sensitive_focus(self.engine.is_sensitive_focus());
        engine.set_current_window(self.engine.current_window().cloned());
        engine.set_commands_disabled(self.engine.commands_disabled());
        engine.set_title_matching_disabled(self.engine.title_matching_disabled());
        engine.set_reinsert_terminators(self.engine.reinserts_terminators());
        if self.engine.async_commands_enabled() && !engine.enable_async_commands() {
            warn!(
                "IBus asynchronous workers could not restart after configuration reload; command-backed actions are unavailable"
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

        // Pre-flight policy check: prevents side effects (e.g., command execution)
        // before policy approval. Post-execution checks happen after engine.process().
        if let Some(violation) = wayexpand_core::pre_flight_check(&self.policy) {
            if self.policy.safe_mode {
                error!(
                    audit_prefix = %self.policy.audit_prefix,
                    violation = %violation,
                    "IBus blocked by pre-flight policy check (prevents execution)"
                );
                return IbusKeyResult::default();
            }
            warn!(
                audit_prefix = %self.policy.audit_prefix,
                violation = %violation,
                "IBus pre-flight check violation in audit mode (execution allowed)"
            );
        }
        // When the core has disabled capture (password fields or an explicit
        // pause), do not consume or commit anything on the client's behalf.
        // The toolkit must receive the original key unchanged.
        if self.engine.is_sensitive_focus() || self.engine.is_user_paused() {
            return IbusKeyResult::default();
        }

        // IBus modifier flags use the same low bits as X11. Do not consume
        // shortcuts or navigation keys: clearing the matcher state is safer
        // than allowing a trigger to span an unrelated command. Mod4 is the
        // normal X11 representation of Super, while IBUS_SUPER_MASK is a
        // separate virtual modifier used by some clients. Mod5 is commonly
        // AltGr, so it is intentionally not treated as a shortcut mask: IBus
        // has already resolved the keyval and the resulting text must remain
        // usable on AltGr layouts.
        const CONTROL_MASK: u32 = 1 << 2;
        const ALT_MASK: u32 = 1 << 3;
        const MOD3_MASK: u32 = 1 << 5;
        const MOD4_MASK: u32 = 1 << 6;
        const IBUS_SUPER_MASK: u32 = 1 << 26;
        if state & (CONTROL_MASK | ALT_MASK | MOD3_MASK | MOD4_MASK | IBUS_SUPER_MASK) != 0 {
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
        // Deferred execution: policy check BEFORE command execution
        // This prevents side effects from occurring before approval.
        let pending = self.engine.process_deferred(event);
        let mut actions = Vec::new();
        let mut policy_blocked = false;
        for pending_result in pending {
            let has_command = pending_result.command.is_some();

            if let Some(command) = &pending_result.command {
                if let Some(violation) = self.policy.command_path_violation(&command.program) {
                    if self.policy.command_path_is_blocked(&command.program) {
                        self.engine
                            .restore_deferred_match(&pending_result.matched_text);
                        error!(
                            audit_prefix = %self.policy.audit_prefix,
                            violation = %violation,
                            "IBus command blocked by organization path policy"
                        );
                        policy_blocked = true;
                        continue;
                    }
                    warn!(
                        audit_prefix = %self.policy.audit_prefix,
                        violation = %violation,
                        "IBus command violates organization path policy; audit mode permits it"
                    );
                }
            }

            // Check policy BEFORE executing commands
            if let Some(violation) = self.policy.expansion_policy_violation(
                pending_result.template_text.len(),
                has_command,
                IBUS_BACKEND_NAME,
            ) {
                if self.policy.safe_mode {
                    self.engine
                        .restore_deferred_match(&pending_result.matched_text);
                    error!(
                        audit_prefix = %self.policy.audit_prefix,
                        violation = %violation,
                        "IBus expansion blocked by organization policy (pre-execution)"
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

            // Policy-approved commands are queued so ProcessKeyEvent never
            // waits for command timeout. Static matches and cache hits commit
            // synchronously because they require no child process.
            let enforcement_policy = self.policy.effective_enforcement_policy();
            let dispatch = match self.engine.dispatch_pending_with_policy(
                pending_result,
                enforcement_policy.max_replacement_size,
            ) {
                Ok(dispatch) => dispatch,
                Err(e) => {
                    warn!("IBus expansion could not be queued or completed: {}", e);
                    continue;
                }
            };

            match dispatch {
                PendingExpansionDispatch::Ready(result) => {
                    actions.extend(expansion_actions(&result, false));
                    // The returned IBus actions are the successful output
                    // transaction for this synchronous/static result.
                    self.engine.commit_applied_expansion(&result);
                }
                PendingExpansionDispatch::Queued => {
                    // The current key has not reached the client yet. Commit
                    // it now so the eventual surrounding-text deletion covers
                    // the complete trigger and preserves key order.
                    actions.push(IbusAction::CommitText(character.to_string()));
                }
            }
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

fn expansion_actions(
    result: &ExpansionResult,
    terminator_already_delivered: bool,
) -> Vec<IbusAction> {
    // Static boundary matches arrive before IBus delivers the delimiter, so
    // the deletion covers only the trigger. Async command matches are drained
    // later, after the delimiter was committed when the command was queued;
    // include that already-delivered character in the same delete/commit
    // transaction or the deletion removes the trigger's final character plus
    // the delimiter and leaves a leading fragment behind.
    let mut delete_chars = result.matched_text.chars().count();
    if terminator_already_delivered && result.reinsert_after.is_some() {
        delete_chars = delete_chars.saturating_add(1);
    }
    let mut actions = vec![IbusAction::DeleteSurroundingText {
        nchars: delete_chars as u32,
    }];
    let mut replacement = result.insert.clone();
    if let Some(character) = result.reinsert_after {
        replacement.push(character);
    }
    actions.push(IbusAction::CommitText(replacement));
    actions
}

fn apply_policy_to_engine(engine: &mut ExpansionEngine, policy: &OrganizationPolicy) {
    let enforcement = policy.effective_enforcement_policy();
    engine.set_commands_disabled(enforcement.disable_commands);
    engine.set_title_matching_disabled(enforcement.disable_title_matching);
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

    #[test]
    fn component_file_detection_is_independent_of_ibus_registry_refresh() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-ibus-component-{}", std::process::id()));
        let component_dir = root.join("ibus/component");
        std::fs::create_dir_all(&component_dir).unwrap();
        assert!(!component_file_present_in(std::slice::from_ref(
            &component_dir
        )));
        std::fs::write(component_dir.join("wayexpand.xml"), "<component/>").unwrap();
        assert!(component_file_present_in(std::slice::from_ref(
            &component_dir
        )));
        std::fs::remove_dir_all(root).unwrap();
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
        IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap()
    }

    #[test]
    fn administrator_absolute_command_policy_rejects_relative_programs() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":cmd"
            replacement = ""
            [expansion.command]
            program = "printf"
            args = ["ok"]
            "#,
        )
        .unwrap();
        let engine = ExpansionEngine::new(config).unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };

        assert!(matches!(
            IbusEngineAdapter::with_policy(engine, policy),
            Err(wayexpand_core::ConfigError::InvalidCommand { .. })
        ));
    }

    #[test]
    fn audit_absolute_command_policy_allows_relative_programs() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":cmd"
            replacement = ""
            [expansion.command]
            program = "printf"
            args = ["audit-ok"]
            "#,
        )
        .unwrap();
        let policy = OrganizationPolicy {
            safe_mode: false,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };
        let mut engine = ExpansionEngine::new(config).unwrap();
        assert!(engine.enable_async_commands());
        let mut adapter = IbusEngineAdapter::with_policy(engine, policy).unwrap();

        let mut key_actions = Vec::new();
        for character in ":cmd".chars() {
            key_actions.extend(adapter.process_key_event(character as u32, 0, 0).actions);
        }
        assert!(key_actions.contains(&IbusAction::CommitText("d".into())));
        let deadline = Instant::now() + Duration::from_secs(1);
        let completed = loop {
            let actions = adapter.drain_completed_commands();
            if !actions.is_empty() {
                break actions;
            }
            assert!(Instant::now() < deadline, "IBus command did not complete");
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(
            completed,
            vec![
                IbusAction::DeleteSurroundingText { nchars: 4 },
                IbusAction::CommitText("audit-ok".into())
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn command_execution_does_not_block_ibus_key_processing() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":slow"
            replacement = ""
            [expansion.command]
            program = "/bin/sh"
            args = ["-c", "sleep 0.4; printf done"]
            timeout_ms = 1000
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        assert!(engine.enable_async_commands());
        let mut adapter = IbusEngineAdapter::new(engine);
        let started = Instant::now();
        let mut key_actions = Vec::new();
        for character in ":slow".chars() {
            key_actions.extend(adapter.process_key_event(character as u32, 0, 0).actions);
        }
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "IBus key processing waited for the child process"
        );
        assert!(key_actions.contains(&IbusAction::CommitText("w".into())));

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let actions = adapter.drain_completed_commands();
            if !actions.is_empty() {
                assert_eq!(
                    actions,
                    vec![
                        IbusAction::DeleteSurroundingText { nchars: 5 },
                        IbusAction::CommitText("done".into())
                    ]
                );
                break;
            }
            assert!(Instant::now() < deadline, "IBus command did not complete");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(unix)]
    #[test]
    fn deferred_command_output_over_organization_limit_is_not_committed() {
        let marker = std::env::temp_dir().join(format!(
            "wayexpand-ibus-policy-output-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let command = format!(
            "printf completed > '{}'; yes x | head -c 257",
            marker.display()
        );
        let config = Config::parse(&format!(
            r#"
            [[expansion]]
            trigger = ":large"
            replacement = ""
            [expansion.command]
            program = "/bin/sh"
            args = ["-c", "{command}"]
            timeout_ms = 1000
            "#
        ))
        .unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            max_replacement_size: 256,
            ..OrganizationPolicy::default()
        };
        let mut adapter =
            IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap();
        assert!(adapter.engine_mut().enable_async_commands());

        let mut actions = Vec::new();
        for character in ":large".chars() {
            actions.extend(adapter.process_key_event(character as u32, 0, 0).actions);
        }

        let deadline = Instant::now() + Duration::from_secs(1);
        while !marker.exists() {
            assert!(Instant::now() < deadline, "the subprocess did not complete");
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(20));
        actions.extend(adapter.drain_completed_commands());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, IbusAction::CommitText(text) if text.len() > 1)),
            "oversized command output must not be committed"
        );
        let _ = std::fs::remove_file(marker);
    }

    #[cfg(unix)]
    #[test]
    fn audit_mode_logs_but_commits_async_output_over_organization_limit() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":large"
            replacement = ""
            match_mode = "word-boundary"
            [expansion.command]
            program = "/bin/sh"
            args = ["-c", "yes x | head -c 257"]
            timeout_ms = 1000
            "#,
        )
        .unwrap();
        let policy = OrganizationPolicy {
            safe_mode: false,
            max_replacement_size: 256,
            ..OrganizationPolicy::default()
        };
        let mut adapter =
            IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap();
        assert!(adapter.engine_mut().enable_async_commands());

        for character in ":large".chars() {
            adapter.process_key_event(character as u32, 0, 0);
        }
        let queued = adapter.process_key_event(' ' as u32, 0, 0);
        assert!(queued.actions.contains(&IbusAction::CommitText(" ".into())));

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let actions = adapter.drain_completed_commands();
            if !actions.is_empty() {
                assert_eq!(
                    actions.first(),
                    Some(&IbusAction::DeleteSurroundingText { nchars: 7 })
                );
                assert!(matches!(
                    actions.get(1),
                    Some(IbusAction::CommitText(text)) if text.len() == 258
                ));
                break;
            }
            assert!(Instant::now() < deadline, "the subprocess did not complete");
            thread::sleep(Duration::from_millis(5));
        }
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
    fn synchronous_ibus_expansion_commits_undo_after_actions_are_created() {
        let config: Config = toml::from_str(
            r#"
            [settings]
            undo_chord = "Ctrl+Z"

            [[expansion]]
            trigger = ":sig"
            replacement = "signature"
            "#,
        )
        .unwrap();
        let mut adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());
        for character in ":sig".chars() {
            adapter.process_key_event(character as u32, 0, 0);
        }

        let undo = adapter
            .engine()
            .prepare_undo(&wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap())
            .expect("a successfully emitted IBus expansion should be undoable");
        assert_eq!(undo.matched_text, "signature");
        assert_eq!(undo.insert, ":sig");
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

    #[cfg(unix)]
    #[test]
    fn async_word_boundary_command_replaces_delivered_delimiter_transactionally() {
        let config: Config = toml::from_str(
            r#"[settings]
undo_chord = "Ctrl+Z"

[[expansion]]
trigger = ":sig"
replacement = ""
match_mode = "word-boundary"
[expansion.command]
program = "/bin/sh"
args = ["-c", "printf signature"]
timeout_ms = 1000
"#,
        )
        .unwrap();
        let mut adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());
        assert!(adapter.engine_mut().enable_async_commands());

        let mut typed_actions = Vec::new();
        for character in ":sig ".chars() {
            typed_actions.extend(adapter.process_key_event(character as u32, 0, 0).actions);
        }
        assert_eq!(
            typed_actions.last(),
            Some(&IbusAction::CommitText(" ".into()))
        );

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let actions = adapter.drain_completed_commands();
            if !actions.is_empty() {
                assert_eq!(
                    actions,
                    vec![
                        IbusAction::DeleteSurroundingText { nchars: 5 },
                        IbusAction::CommitText("signature ".into()),
                    ]
                );
                let undo = adapter
                    .engine()
                    .prepare_undo(&wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap())
                    .expect("completed IBus command should be undoable");
                assert_eq!(undo.matched_text, "signature");
                assert_eq!(undo.insert, ":sig");
                break;
            }
            assert!(Instant::now() < deadline, "IBus command did not complete");
            thread::sleep(Duration::from_millis(5));
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
    fn super_modifiers_do_not_consume_printable_shortcuts() {
        const MOD4_MASK: u32 = 1 << 6;
        const IBUS_SUPER_MASK: u32 = 1 << 26;

        for state in [MOD4_MASK, IBUS_SUPER_MASK] {
            let mut adapter = adapter();
            let result = adapter.process_key_event('a' as u32, 0, state);
            assert_eq!(result, IbusKeyResult::default());
        }
    }

    #[test]
    fn modifier_levels_do_not_consume_boundary_shortcuts() {
        const MOD3_MASK: u32 = 1 << 5;
        const MOD4_MASK: u32 = 1 << 6;
        const MOD5_MASK: u32 = 1 << 7;

        for state in [MOD3_MASK, MOD4_MASK] {
            let mut adapter = boundary_adapter();
            for character in ":sig".chars() {
                adapter.process_key_event(character as u32, 0, 0);
            }
            assert_eq!(
                adapter.process_key_event(' ' as u32, 0, state),
                IbusKeyResult::default()
            );
            assert_eq!(
                adapter.process_key_event('g' as u32, 0, 0).actions,
                vec![IbusAction::CommitText("g".into())]
            );
        }

        let mut altgr_adapter = boundary_adapter();
        for character in ":sig".chars() {
            altgr_adapter.process_key_event(character as u32, 0, 0);
        }
        assert_eq!(
            altgr_adapter
                .process_key_event(' ' as u32, 0, MOD5_MASK)
                .actions,
            vec![
                IbusAction::DeleteSurroundingText { nchars: 4 },
                IbusAction::CommitText("signature ".into())
            ]
        );
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
