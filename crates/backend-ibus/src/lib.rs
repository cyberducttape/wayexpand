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
    CompletionNotifier, Config, ExpansionEngine, ExpansionResult, InjectorCapabilities, InputEvent,
    InputSourceCapabilities, OrganizationPolicy, PendingExpansionDispatch,
};

// IBus' public C API defines IBUS_RELEASE_MASK as (1 << 30). The ibus-rs
// crate is not used because it adds a mandatory libdbus system dependency.
const IBUS_RELEASE_MASK: u32 = 1 << 30;

/// Organization-policy identity of this backend. IBus is governed separately
/// from the native input-method-v2 route: their surrounding-text, sensitive-
/// field, and replacement guarantees differ, so `allowed_backends` must name
/// `ibus` explicitly to permit it.
pub const IBUS_BACKEND_NAME: &str = "ibus";

/// `IBUS_CAP_SURROUNDING_TEXT`: the client can report surrounding text and
/// honour `DeleteSurroundingText`.
pub const IBUS_CAP_SURROUNDING_TEXT: u32 = 1 << 5;

// IBusInputPurpose / IBusInputHints values from ibustypes.h.
const IBUS_INPUT_PURPOSE_PASSWORD: u32 = 8;
const IBUS_INPUT_PURPOSE_PIN: u32 = 9;
const IBUS_INPUT_PURPOSE_LAST_KNOWN: u32 = 13; // IBUS_INPUT_PURPOSE_DATETIME
const IBUS_INPUT_HINT_PRIVATE: u32 = 1 << 11;
const IBUS_INPUT_HINT_HIDDEN_TEXT: u32 = 1 << 12;

/// Whether an IBus content type must disable capture. Password and PIN
/// purposes, the private and hidden-text hints, and any purpose newer than
/// this build knows are all treated as sensitive: unknown means sensitive.
pub fn content_type_is_sensitive(purpose: u32, hints: u32) -> bool {
    matches!(
        purpose,
        IBUS_INPUT_PURPOSE_PASSWORD | IBUS_INPUT_PURPOSE_PIN
    ) || purpose > IBUS_INPUT_PURPOSE_LAST_KNOWN
        || hints & (IBUS_INPUT_HINT_PRIVATE | IBUS_INPUT_HINT_HIDDEN_TEXT) != 0
}

/// Output guarantees of the IBus route. Delete and commit are two separate
/// D-Bus signals with an observable failure boundary between them, so the
/// replacement is not atomic.
pub fn injector_capabilities() -> InjectorCapabilities {
    InjectorCapabilities {
        insertion_mode: "ibus commit-text",
        max_text_chars: 0,
        expected_throughput_chars_per_sec: None,
        atomic_replace: false,
        // The engine refuses unless IBus surrounding text confirms the
        // trigger before the cursor.
        replacement_guarantee: wayexpand_core::ReplacementGuarantee::VerifiedSurroundingText,
        full_unicode: true,
        cursor_reposition: false,
        key_passthrough: true,
    }
}

/// Capture guarantees of the IBus route. Sensitive-field focus is reported
/// through IBus content types and fails closed until one is received.
pub fn source_capabilities() -> InputSourceCapabilities {
    InputSourceCapabilities {
        sensitive_focus: true,
        exclusive_capture: true,
        reliable_key_state: false,
        key_passthrough: true,
        composition_aware: false,
        local_compose_aware: false,
        layout_aware: true,
    }
}

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

/// The client's text around the cursor as last reported through
/// `SetSurroundingText`, advanced locally by the edits this engine emits.
/// Positions are in Unicode scalar values, as IBus reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SurroundingText {
    text: Vec<char>,
    cursor: usize,
    anchor: usize,
}

impl SurroundingText {
    /// Whether `expected` is exactly the text before a collapsed cursor.
    fn ends_with_at_cursor(&self, expected: &str) -> bool {
        if self.cursor != self.anchor || self.cursor > self.text.len() {
            return false;
        }
        let expected: Vec<char> = expected.chars().collect();
        self.text[..self.cursor].ends_with(&expected)
    }

    /// Apply an emitted action. Returns false when the model can no longer
    /// describe the client, so the caller drops it.
    fn apply(&mut self, action: &IbusAction) -> bool {
        if self.cursor != self.anchor || self.cursor > self.text.len() {
            return false;
        }
        match action {
            IbusAction::DeleteSurroundingText { nchars } => {
                let count = *nchars as usize;
                let Some(start) = self.cursor.checked_sub(count) else {
                    return false;
                };
                self.text.drain(start..self.cursor);
                self.cursor = start;
            }
            IbusAction::CommitText(text) => {
                let inserted: Vec<char> = text.chars().collect();
                let count = inserted.len();
                self.text.splice(self.cursor..self.cursor, inserted);
                self.cursor += count;
            }
        }
        self.anchor = self.cursor;
        true
    }
}

/// Core-backed IBus engine state.
pub struct IbusEngineAdapter {
    engine: ExpansionEngine,
    enabled: bool,
    policy: OrganizationPolicy,
    client_capabilities: u32,
    surrounding: Option<SurroundingText>,
    /// Set when safe-mode policy requires guarantees this route lacks
    /// (for example `require_atomic_replace`). Expansion is then disabled.
    capability_block: Option<String>,
}

impl IbusEngineAdapter {
    pub fn new(engine: ExpansionEngine) -> Self {
        Self::build(engine, OrganizationPolicy::default())
    }

    pub fn with_policy(
        mut engine: ExpansionEngine,
        policy: OrganizationPolicy,
    ) -> Result<Self, wayexpand_core::ConfigError> {
        engine.apply_administrator_policy(&policy)?;
        Ok(Self::build(engine, policy))
    }

    fn build(mut engine: ExpansionEngine, policy: OrganizationPolicy) -> Self {
        apply_policy_to_engine(&mut engine, &policy);
        // Keep IBus key handling non-blocking for broker actions. Direct
        // programs are still rejected by the backend-specific gate below.
        let _ = engine.enable_async_commands();
        // A new engine has no content type yet: capture stays off until the
        // client reports a known non-sensitive field.
        engine.process(InputEvent::FocusChanged { sensitive: true });
        let capability_block = Self::check_capabilities(&policy);
        Self {
            engine,
            enabled: true,
            policy,
            client_capabilities: 0,
            surrounding: None,
            capability_block,
        }
    }

    /// Verify policy requirements against the real IBus capability profile,
    /// as the daemon does for its negotiated source and injector.
    fn check_capabilities(policy: &OrganizationPolicy) -> Option<String> {
        let violation = policy
            .capability_violation_for_source(injector_capabilities(), source_capabilities())?;
        policy_blocks(policy, Some(violation.clone()), "route capability check")
            .then_some(violation)
    }

    /// Why safe-mode policy has disabled expansion for this route, if it has.
    pub fn capability_block(&self) -> Option<&str> {
        self.capability_block.as_deref()
    }

    /// Register a wakeup for asynchronous command completions; see
    /// [`ExpansionEngine::set_completion_notifier`].
    pub fn set_completion_notifier(&mut self, notifier: Option<CompletionNotifier>) {
        self.engine.set_completion_notifier(notifier);
    }

    /// Apply an IBus `SetContentType`. This is the only way capture is
    /// enabled for a field.
    pub fn set_content_type(&mut self, purpose: u32, hints: u32) {
        self.engine.process(InputEvent::FocusChanged {
            sensitive: content_type_is_sensitive(purpose, hints),
        });
    }

    /// Apply an IBus `SetCapabilities`.
    pub fn set_capabilities(&mut self, capabilities: u32) {
        self.client_capabilities = capabilities;
        if capabilities & IBUS_CAP_SURROUNDING_TEXT == 0 {
            self.surrounding = None;
        }
    }

    /// Apply an IBus `SetSurroundingText`. `cursor` and `anchor` are in
    /// Unicode scalar values.
    pub fn set_surrounding_text(&mut self, text: &str, cursor: u32, anchor: u32) {
        let text: Vec<char> = text.chars().collect();
        let (cursor, anchor) = (cursor as usize, anchor as usize);
        self.surrounding = (self.client_capabilities & IBUS_CAP_SURROUNDING_TEXT != 0
            && cursor <= text.len()
            && anchor <= text.len())
        .then_some(SurroundingText {
            text,
            cursor,
            anchor,
        });
    }

    /// Forget the surrounding-text model; replacements are refused until the
    /// client reports its text again.
    pub fn clear_surrounding_text(&mut self) {
        self.surrounding = None;
    }

    /// Whether the client's reported text agrees that `delivered` sits
    /// immediately before a collapsed cursor. Nothing destructive may be
    /// emitted unless it does.
    fn surrounding_confirms(&self, delivered: &str) -> bool {
        self.client_capabilities & IBUS_CAP_SURROUNDING_TEXT != 0
            && self
                .surrounding
                .as_ref()
                .is_some_and(|surrounding| surrounding.ends_with_at_cursor(delivered))
    }

    /// Advance the local surrounding-text model over emitted actions.
    fn record_emitted(&mut self, actions: &[IbusAction]) {
        if let Some(surrounding) = self.surrounding.as_mut() {
            if !actions.iter().all(|action| surrounding.apply(action)) {
                self.surrounding = None;
            }
        }
    }

    pub fn engine(&self) -> &ExpansionEngine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut ExpansionEngine {
        &mut self.engine
    }

    /// Drain completed asynchronous commands and return their actions. Each
    /// result is recorded as applied as soon as its actions are produced;
    /// hosts that emit actions should use [`Self::drain_completed_commands_with`].
    pub fn drain_completed_commands(&mut self) -> Vec<IbusAction> {
        let mut actions = Vec::new();
        let _ = self.drain_completed_commands_with(|batch| {
            actions.extend_from_slice(batch);
            Ok::<(), std::convert::Infallible>(())
        });
        actions
    }

    /// Drain completed asynchronous commands, emitting each replacement
    /// through `emit` and recording it as applied only after `emit`
    /// succeeds. On an emit failure the engine is reset (the client may hold
    /// a partial edit) and the error is returned.
    pub fn drain_completed_commands_with<E>(
        &mut self,
        mut emit: impl FnMut(&[IbusAction]) -> Result<(), E>,
    ) -> Result<usize, E> {
        let mut emitted = 0;
        for result in self.engine.drain_completed_commands() {
            if !self.enabled || self.capability_block.is_some() {
                self.engine.restore_deferred_match(&result.matched_text);
                continue;
            }
            let violation = self.policy.expansion_policy_violation(
                result.insert.len(),
                result.command_backed,
                IBUS_BACKEND_NAME,
            );
            if policy_blocks(&self.policy, violation, "completed expansion") {
                self.engine.restore_deferred_match(&result.matched_text);
                continue;
            }
            // The key that queued the command was committed at queue time,
            // so the whole trigger (and any delimiter) is in the document.
            let (delivered, actions) = replacement_actions(&result, true);
            if !self.surrounding_confirms(&delivered) {
                warn!("IBus surrounding text no longer matches the trigger; replacement discarded");
                self.engine.restore_deferred_match(&result.matched_text);
                self.reset();
                continue;
            }
            if let Err(error) = emit(&actions) {
                self.reset();
                return Err(error);
            }
            self.engine.commit_applied_expansion(&result);
            self.record_emitted(&actions);
            emitted += 1;
        }
        Ok(emitted)
    }

    pub fn replace_config(&mut self, config: Config) -> Result<(), wayexpand_core::ConfigError> {
        let mut engine = ExpansionEngine::new(config)?;
        engine.apply_administrator_policy(&self.policy)?;
        engine.set_user_paused(self.engine.is_user_paused());
        engine.set_sensitive_focus(self.engine.is_sensitive_focus());
        engine.set_current_window(self.engine.current_window().cloned());
        engine.set_commands_disabled(self.engine.commands_disabled());
        engine.set_direct_commands_disabled(self.engine.direct_commands_disabled());
        engine.set_title_matching_disabled(self.engine.title_matching_disabled());
        engine.set_reinsert_terminators(self.engine.reinserts_terminators());
        engine.set_composition_active(self.engine.is_composition_active());
        engine.set_completion_notifier(self.engine.completion_notifier());
        engine.set_clipboard_reader(self.engine.clipboard_reader());
        if self.engine.async_commands_enabled() && !engine.enable_async_commands() {
            warn!(
                "IBus asynchronous workers could not restart after configuration reload; command-backed actions are unavailable"
            );
        }
        apply_policy_to_engine(&mut engine, &self.policy);
        self.engine = engine;
        Ok(())
    }

    /// A context gained focus. ibus-daemon follows FocusIn with Enable,
    /// SetCapabilities and SetContentType, so capture stays off until that
    /// content type arrives; a refocus never reuses an older field's type.
    pub fn focus_in(&mut self) {
        self.enabled = true;
        self.surrounding = None;
        self.engine
            .process(InputEvent::FocusChanged { sensitive: true });
    }

    pub fn focus_out(&mut self) {
        self.enabled = false;
        self.surrounding = None;
        self.engine
            .process(InputEvent::FocusChanged { sensitive: true });
    }

    /// IBus `Reset`: the cursor may have moved, so the trigger buffer and the
    /// surrounding-text model are dropped. The field's content type is
    /// unchanged by a reset and is kept.
    pub fn reset(&mut self) {
        self.surrounding = None;
        self.engine.process(InputEvent::Reset);
    }

    /// Process an IBus key press. `keyval` is an XKB keysym, as specified by
    /// `org.freedesktop.IBus.Engine.ProcessKeyEvent`.
    pub fn process_key_event(&mut self, keyval: u32, keycode: u32, state: u32) -> IbusKeyResult {
        let result = self.process_key_event_inner(keyval, keycode, state);
        if state & IBUS_RELEASE_MASK == 0 {
            if result.handled {
                self.record_emitted(&result.actions);
            } else {
                // The client applies this key itself; the model cannot know
                // its effect until the client reports surrounding text again.
                self.surrounding = None;
            }
        }
        result
    }

    fn process_key_event_inner(&mut self, keyval: u32, _keycode: u32, state: u32) -> IbusKeyResult {
        // IBus delivers both press and release events through this method.
        // Releases carry IBUS_RELEASE_MASK and must not be interpreted as a
        // second printable character or delimiter.
        if state & IBUS_RELEASE_MASK != 0 {
            return IbusKeyResult::default();
        }

        if !self.enabled || self.capability_block.is_some() {
            return IbusKeyResult::default();
        }

        // Pre-flight policy check: prevents side effects (e.g., command execution)
        // before policy approval. Post-execution checks happen after engine.process().
        if policy_blocks(
            &self.policy,
            wayexpand_core::pre_flight_check(&self.policy),
            "pre-flight check",
        ) {
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
                if command.action.is_none()
                    && policy_blocks(
                        &self.policy,
                        self.policy.command_path_violation(&command.program),
                        "command path",
                    )
                {
                    self.engine
                        .restore_deferred_match(&pending_result.matched_text);
                    policy_blocked = true;
                    continue;
                }
            }

            // Check policy BEFORE executing commands
            let violation = self.policy.expansion_policy_violation(
                pending_result.template_text.len(),
                has_command,
                IBUS_BACKEND_NAME,
            );
            if policy_blocks(&self.policy, violation, "expansion (pre-execution)") {
                self.engine
                    .restore_deferred_match(&pending_result.matched_text);
                policy_blocked = true;
                continue;
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
                    // The current key has not reached the client. Refuse the
                    // replacement unless the client's own text confirms the
                    // rest of the trigger sits right before the cursor.
                    let (delivered, replacement) = replacement_actions(&result, false);
                    if !self.surrounding_confirms(&delivered) {
                        warn!(
                            "IBus surrounding text does not confirm the trigger; replacement refused"
                        );
                        self.engine.restore_deferred_match(&result.matched_text);
                        policy_blocked = true;
                        continue;
                    }
                    actions.extend(replacement);
                    // The returned IBus actions are the output transaction
                    // for this synchronous/static result; the service resets
                    // the engine if emitting them fails.
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

/// Plan a replacement: the text already in the client document that must be
/// deleted, and the delete/commit actions that replace it.
///
/// `key_delivered` says whether the key that completed the match has reached
/// the client. A synchronous match consumes that key, so an immediate
/// trigger's final character was never delivered and a boundary match's
/// delimiter is carried by the replacement instead. An asynchronous match
/// committed the key when the command was queued, so the whole trigger and
/// any delimiter are in the document.
fn replacement_actions(result: &ExpansionResult, key_delivered: bool) -> (String, Vec<IbusAction>) {
    let mut delivered = result.matched_text.clone();
    match (key_delivered, result.reinsert_after) {
        (true, Some(delimiter)) => delivered.push(delimiter),
        (false, None) => {
            delivered.pop();
        }
        _ => {}
    }
    let mut actions = Vec::with_capacity(2);
    let delete_chars = delivered.chars().count();
    if delete_chars > 0 {
        actions.push(IbusAction::DeleteSurroundingText {
            nchars: delete_chars as u32,
        });
    }
    let mut replacement = result.insert.clone();
    if let Some(character) = result.reinsert_after {
        replacement.push(character);
    }
    actions.push(IbusAction::CommitText(replacement));
    (delivered, actions)
}

/// Enforce an organization-policy violation: safe mode logs an error and
/// blocks (returns true); audit mode logs a warning and lets the operation
/// proceed. `what` names the checked operation in the log line.
fn policy_blocks(policy: &OrganizationPolicy, violation: Option<String>, what: &str) -> bool {
    let Some(violation) = violation else {
        return false;
    };
    if policy.safe_mode {
        error!(
            audit_prefix = %policy.audit_prefix,
            violation = %violation,
            "IBus {what} blocked by organization policy"
        );
        true
    } else {
        warn!(
            audit_prefix = %policy.audit_prefix,
            violation = %violation,
            "IBus {what} violates organization policy; audit mode permits it"
        );
        false
    }
}

fn apply_policy_to_engine(engine: &mut ExpansionEngine, policy: &OrganizationPolicy) {
    let enforcement = policy.effective_enforcement_policy();
    // IBus runs outside the hardened wayexpand.service boundary. Direct
    // executable commands remain disabled here, but managed Action Broker
    // requests cross a separate authenticated Unix-socket boundary and are
    // allowed to fail closed if that broker is unavailable.
    engine.set_direct_commands_disabled(true);
    engine.set_title_matching_disabled(enforcement.disable_title_matching);
}

/// Convert the XKB keysym that IBus supplies to the character it types.
/// Layout, dead-key and Compose resolution has already happened before IBus
/// receives the event. Conversion follows xkbcommon's `xkb_keysym_to_utf32`
/// (via `xkeysym`), so legacy keysyms such as `Greek_alpha` or `Cyrillic_a`
/// and keypad digits resolve like explicit Unicode keysyms. Control
/// characters other than tab and newline are not text.
fn keysym_to_char(keysym: u32) -> Option<char> {
    match keysym {
        xkeysym::key::Tab | xkeysym::key::KP_Tab => Some('\t'),
        xkeysym::key::Return | xkeysym::key::KP_Enter => Some('\n'),
        _ => xkeysym::Keysym::new(keysym)
            .key_char()
            .filter(|character| !character.is_control()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayexpand_core::{Config, ExpansionEngine, OrganizationPolicy};

    /// Put an adapter in the state ibus-daemon leaves it in for an ordinary
    /// text field: focused, surrounding text supported, free-form content.
    fn focused(mut adapter: IbusEngineAdapter) -> IbusEngineAdapter {
        adapter.focus_in();
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        adapter.set_content_type(0, 0);
        adapter
    }

    /// A minimal IBus client. Like ibus-gtk it reports surrounding text before
    /// every key press, inserts keys the engine does not handle, and applies
    /// the engine's delete/commit actions to its own document.
    #[derive(Default)]
    struct Client {
        text: Vec<char>,
        cursor: usize,
    }

    impl Client {
        fn key(
            &mut self,
            adapter: &mut IbusEngineAdapter,
            keyval: u32,
            state: u32,
        ) -> IbusKeyResult {
            let text: String = self.text.iter().collect();
            adapter.set_surrounding_text(&text, self.cursor as u32, self.cursor as u32);
            let result = adapter.process_key_event(keyval, 0, state);
            if state == 0 && !result.handled {
                if keyval == xkeysym::key::BackSpace {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                        self.text.remove(self.cursor);
                    }
                } else if let Some(character) = keysym_to_char(keyval) {
                    self.text.insert(self.cursor, character);
                    self.cursor += 1;
                }
            }
            self.apply(&result.actions);
            result
        }

        fn type_text(&mut self, adapter: &mut IbusEngineAdapter, text: &str) -> Vec<IbusAction> {
            text.chars()
                .flat_map(|character| self.key(adapter, character as u32, 0).actions)
                .collect()
        }

        fn apply(&mut self, actions: &[IbusAction]) {
            for action in actions {
                match action {
                    IbusAction::DeleteSurroundingText { nchars } => {
                        let start = self.cursor - *nchars as usize;
                        self.text.drain(start..self.cursor);
                        self.cursor = start;
                    }
                    IbusAction::CommitText(text) => {
                        for character in text.chars() {
                            self.text.insert(self.cursor, character);
                            self.cursor += 1;
                        }
                    }
                }
            }
        }

        fn text(&self) -> String {
            self.text.iter().collect()
        }
    }

    fn adapter() -> IbusEngineAdapter {
        let config: Config = toml::from_str(
            r#"[[expansion]]
trigger = ":sig"
replacement = "signature"
"#,
        )
        .unwrap();
        focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ))
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
            r#"[settings]
undo_chord = "Ctrl+Z"

[[expansion]]
trigger = ":sig"
replacement = "signature"
match_mode = "word-boundary"
"#,
        )
        .unwrap();
        focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ))
    }

    fn policy_adapter(policy: OrganizationPolicy) -> IbusEngineAdapter {
        let config: Config = toml::from_str(
            r#"[[expansion]]
trigger = ":sig"
replacement = "signature"
"#,
        )
        .unwrap();
        focused(
            IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap(),
        )
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
    fn ibus_disables_commands_even_in_audit_mode() {
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
        let mut adapter =
            IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap();
        assert!(adapter.engine().direct_commands_disabled());

        let mut key_actions = Vec::new();
        for character in ":cmd".chars() {
            key_actions.extend(adapter.process_key_event(character as u32, 0, 0).actions);
        }
        assert!(key_actions.iter().all(|action| matches!(
            action,
            IbusAction::CommitText(text) if text.chars().count() == 1
        )));
        assert!(adapter.drain_completed_commands().is_empty());
    }

    #[test]
    fn ibus_keeps_managed_actions_enabled_behind_the_broker_boundary() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":action"
            replacement = ""
            [expansion.command]
            action = "cluster-status"
            timeout_ms = 3000
            "#,
        )
        .unwrap();
        let adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());
        assert!(adapter.engine().direct_commands_disabled());
        assert!(!adapter.engine().commands_disabled());
    }

    #[cfg(unix)]
    #[test]
    fn command_backed_expansions_are_not_started_by_ibus() {
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
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));
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

        assert!(adapter.drain_completed_commands().is_empty());
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

        actions.extend(adapter.drain_completed_commands());
        thread::sleep(Duration::from_millis(20));
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, IbusAction::CommitText(text) if text.len() > 1)),
            "command output must not be committed by IBus"
        );
        assert!(
            !marker.exists(),
            "IBus must not spawn command-backed expansions"
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

        for character in ":large".chars() {
            adapter.process_key_event(character as u32, 0, 0);
        }
        let actions = adapter.process_key_event(' ' as u32, 0, 0).actions;
        assert!(actions.iter().all(|action| !matches!(
            action,
            IbusAction::CommitText(text) if text.len() > 1
        )));
        assert!(adapter.drain_completed_commands().is_empty());
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
        let mut client = Client::default();
        client.type_text(&mut adapter, "x");
        let actions = client.type_text(&mut adapter, ":sig");
        // The final `g` is consumed and never reaches the client, so only
        // `:si` is deleted. Deleting four would also remove the user's `x`.
        assert_eq!(
            &actions[actions.len() - 2..],
            &[
                IbusAction::DeleteSurroundingText { nchars: 3 },
                IbusAction::CommitText("signature".into())
            ]
        );
        assert_eq!(client.text(), "xsignature");
    }

    #[test]
    fn unicode_keysym_forms_are_committed_without_loss() {
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(
                Config::parse(
                    r#"[[expansion]]
trigger = ":東京😀"
replacement = "世界 🌍"
"#,
                )
                .unwrap(),
            )
            .unwrap(),
        ));

        // IBus uses the X11 Unicode keysym form for code points outside the
        // legacy Latin-1 range: 0x01000000 + the scalar value.
        let unicode_keysym = |character: char| 0x0100_0000 + character as u32;
        let mut client = Client::default();
        let mut actions = Vec::new();
        for character in ":東京😀".chars() {
            let keyval = if character.is_ascii() {
                character as u32
            } else {
                unicode_keysym(character)
            };
            actions.extend(client.key(&mut adapter, keyval, 0).actions);
        }

        assert_eq!(
            actions,
            vec![
                IbusAction::CommitText("東".into()),
                IbusAction::CommitText("京".into()),
                IbusAction::DeleteSurroundingText { nchars: 3 },
                IbusAction::CommitText("世界 🌍".into()),
            ]
        );
        assert_eq!(client.text(), "世界 🌍");
    }

    #[test]
    fn invalid_unicode_keysym_is_not_committed() {
        let mut adapter = adapter();
        let result = adapter.process_key_event(0x0100_0000 + 0x11_0000, 0, 0);
        assert_eq!(result, IbusKeyResult::default());
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
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));
        Client::default().type_text(&mut adapter, ":sig");

        let undo = adapter
            .engine()
            .prepare_undo(&wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap())
            .expect("a successfully emitted IBus expansion should be undoable");
        assert_eq!(undo.matched_text, "signature");
        assert_eq!(undo.insert, ":sig");
    }

    #[test]
    fn delimiter_is_reinserted_with_the_replacement() {
        let mut adapter = boundary_adapter();
        let mut client = Client::default();
        let actions = client.type_text(&mut adapter, ":sig ");
        assert_eq!(
            &actions[actions.len() - 2..],
            &[
                IbusAction::DeleteSurroundingText { nchars: 4 },
                IbusAction::CommitText("signature ".into())
            ]
        );
        assert_eq!(client.text(), "signature ");
        let undo = adapter
            .engine()
            .prepare_undo(&wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap())
            .expect("a boundary expansion should be undoable");
        assert_eq!(undo.matched_text, "signature ");
        assert_eq!(undo.insert, ":sig ");
    }

    #[cfg(unix)]
    #[test]
    fn command_backed_word_boundary_is_forwarded_without_execution() {
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
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));

        let typed_actions = Client::default().type_text(&mut adapter, ":sig ");
        assert!(!typed_actions
            .iter()
            .any(|action| matches!(action, IbusAction::DeleteSurroundingText { .. })));
        assert!(!typed_actions.iter().any(|action| matches!(
            action,
            IbusAction::CommitText(text) if text.contains("signature")
        )));
        assert!(adapter.drain_completed_commands().is_empty());
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
        let mut client = Client::default();

        client.type_text(&mut adapter, ":si");
        assert_eq!(
            client.key(&mut adapter, 'i' as u32, IBUS_RELEASE_MASK),
            IbusKeyResult::default()
        );

        let result = client.key(&mut adapter, 'g' as u32, 0);
        assert_eq!(
            result.actions,
            vec![
                IbusAction::DeleteSurroundingText { nchars: 3 },
                IbusAction::CommitText("signature".into())
            ]
        );
        assert_eq!(client.text(), "signature");
    }

    #[test]
    fn modifier_key_release_does_not_reset_valid_buffer() {
        const CONTROL_MASK: u32 = 1 << 2;
        let mut adapter = adapter();
        let mut client = Client::default();

        client.type_text(&mut adapter, ":si");
        assert_eq!(
            client.key(
                &mut adapter,
                xkeysym::key::Control_L,
                CONTROL_MASK | IBUS_RELEASE_MASK
            ),
            IbusKeyResult::default()
        );

        let result = client.key(&mut adapter, 'g' as u32, 0);
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
        let mut client = Client::default();
        client.type_text(&mut altgr_adapter, ":sig");
        assert_eq!(
            client
                .key(&mut altgr_adapter, ' ' as u32, MOD5_MASK)
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
        let mut first_client = Client::default();

        first_client.type_text(&mut first, ":si");

        // A second input context must not inherit the first context's partial
        // trigger. Its ordinary key is committed unchanged.
        let isolated = Client::default().key(&mut second, 'g' as u32, 0);
        assert_eq!(isolated.actions, vec![IbusAction::CommitText("g".into())]);

        let completed = first_client.key(&mut first, 'g' as u32, 0);
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
        let mut client = Client::default();
        let actions = client.type_text(&mut adapter, ":sig");
        assert_eq!(actions.last(), Some(&IbusAction::CommitText("g".into())));
        assert_eq!(client.text(), ":sig");
    }

    #[test]
    fn audit_organization_policy_allows_but_reports_disallowed_ibus_expansion() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: false,
            allowed_backends: vec!["libei".into()],
            ..Default::default()
        });
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), "signature");
    }

    #[test]
    fn input_method_v2_allowance_does_not_permit_ibus() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["input-method-v2".into()],
            ..Default::default()
        });
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), ":sig");

        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["input-method-v2".into(), "ibus".into()],
            ..Default::default()
        });
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), "signature");
    }

    #[test]
    fn safe_mode_atomic_replace_requirement_disables_ibus() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            require_atomic_replace: true,
            ..Default::default()
        });
        assert!(adapter.capability_block().is_some());
        let result = adapter.process_key_event('a' as u32, 0, 0);
        assert_eq!(result, IbusKeyResult::default());

        // Audit mode reports the gap but keeps the route usable.
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: false,
            require_atomic_replace: true,
            ..Default::default()
        });
        assert!(adapter.capability_block().is_none());
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), "signature");
    }

    #[test]
    fn sensitive_focus_requirement_is_met_by_content_types() {
        let adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            require_sensitive_focus: true,
            ..Default::default()
        });
        assert!(adapter.capability_block().is_none());
    }

    #[test]
    fn ibus_route_does_not_claim_atomic_replacement() {
        assert!(!injector_capabilities().atomic_replace);
        assert_eq!(
            injector_capabilities().replacement_guarantee,
            wayexpand_core::ReplacementGuarantee::VerifiedSurroundingText
        );
    }

    fn unfocused_adapter() -> IbusEngineAdapter {
        let config: Config =
            toml::from_str("[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\n")
                .unwrap();
        IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap())
    }

    #[test]
    fn capture_is_off_until_a_content_type_arrives() {
        let mut adapter = unfocused_adapter();
        assert!(adapter.engine().is_sensitive_focus());
        adapter.focus_in();
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        // A key that arrives before SetContentType passes through untouched.
        assert_eq!(
            adapter.process_key_event('a' as u32, 0, 0),
            IbusKeyResult::default()
        );
        adapter.set_content_type(0, 0);
        assert!(!adapter.engine().is_sensitive_focus());
        assert!(adapter.process_key_event('a' as u32, 0, 0).handled);
    }

    #[test]
    fn sensitive_content_types_and_hints_disable_capture() {
        const PASSWORD: u32 = 8;
        const PIN: u32 = 9;
        const TERMINAL: u32 = 10;
        const DATETIME: u32 = 13;
        const UNKNOWN: u32 = 14;
        const PRIVATE: u32 = 1 << 11;
        const HIDDEN_TEXT: u32 = 1 << 12;
        for (purpose, hints, sensitive) in [
            (0, 0, false),
            (TERMINAL, 0, false),
            (DATETIME, 0, false),
            (PASSWORD, 0, true),
            (PIN, 0, true),
            (UNKNOWN, 0, true),
            (u32::MAX, 0, true),
            (0, PRIVATE, true),
            (0, HIDDEN_TEXT, true),
            (0, 1 << 0, false),
        ] {
            assert_eq!(
                content_type_is_sensitive(purpose, hints),
                sensitive,
                "purpose {purpose} hints {hints:#x}"
            );
            let mut adapter = adapter();
            adapter.set_content_type(purpose, hints);
            assert_eq!(adapter.engine().is_sensitive_focus(), sensitive);
            if sensitive {
                assert_eq!(
                    adapter.process_key_event('a' as u32, 0, 0),
                    IbusKeyResult::default()
                );
            }
        }
    }

    #[test]
    fn refocus_without_a_new_content_type_stays_sensitive() {
        let mut adapter = adapter();
        assert!(!adapter.engine().is_sensitive_focus());
        adapter.focus_out();
        adapter.focus_in();
        assert!(adapter.engine().is_sensitive_focus());
        assert_eq!(
            adapter.process_key_event('a' as u32, 0, 0),
            IbusKeyResult::default()
        );
    }

    #[test]
    fn reset_keeps_the_field_content_type() {
        let mut adapter = adapter();
        adapter.set_content_type(8, 0);
        adapter.reset();
        assert!(adapter.engine().is_sensitive_focus());
        adapter.set_content_type(0, 0);
        adapter.reset();
        assert!(!adapter.engine().is_sensitive_focus());
    }

    #[test]
    fn moved_cursor_without_reset_refuses_replacement() {
        let mut adapter = adapter();
        let mut client = Client::default();
        client.type_text(&mut adapter, ":s");
        // The user clicks elsewhere; this client sends no Reset.
        client.cursor = 0;
        let actions = client.type_text(&mut adapter, "ig");
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, IbusAction::DeleteSurroundingText { .. })),
            "{actions:?}"
        );
        assert_eq!(client.text(), "ig:s");
    }

    #[test]
    fn active_selection_refuses_replacement() {
        let mut adapter = adapter();
        let mut client = Client::default();
        client.type_text(&mut adapter, ":si");
        let text = client.text();
        adapter.set_surrounding_text(&text, 3, 1);
        let result = adapter.process_key_event('g' as u32, 0, 0);
        assert_eq!(result.actions, vec![IbusAction::CommitText("g".into())]);
    }

    #[test]
    fn client_without_surrounding_text_gets_no_replacement() {
        let mut adapter = adapter();
        adapter.set_capabilities(0);
        let mut client = Client::default();
        let actions = client.type_text(&mut adapter, ":sig");
        assert!(!actions
            .iter()
            .any(|action| matches!(action, IbusAction::DeleteSurroundingText { .. })));
        assert_eq!(client.text(), ":sig");
    }

    #[test]
    fn stale_surrounding_text_refuses_replacement() {
        let mut adapter = adapter();
        let mut client = Client::default();
        client.type_text(&mut adapter, ":si");
        // The client reports text that no longer ends with the trigger.
        adapter.set_surrounding_text("hello", 5, 5);
        let result = adapter.process_key_event('g' as u32, 0, 0);
        assert_eq!(result.actions, vec![IbusAction::CommitText("g".into())]);
    }

    #[test]
    fn legacy_and_keypad_keysyms_convert_to_text() {
        for (keysym, expected) in [
            (0x07e1, Some('α')), // Greek_alpha
            (0x06c1, Some('а')), // Cyrillic_a
            (0x05c7, Some('ا')), // Arabic_alef
            (0x0ce0, Some('א')), // hebrew_aleph
            (xkeysym::key::KP_1, Some('1')),
            (xkeysym::key::KP_Space, Some(' ')),
            (xkeysym::key::Return, Some('\n')),
            (xkeysym::key::Tab, Some('\t')),
            (xkeysym::key::dead_acute, None),
            (xkeysym::key::Multi_key, None),
            (xkeysym::key::Escape, None),
            (xkeysym::key::Delete, None),
            (xkeysym::key::BackSpace, None),
        ] {
            assert_eq!(keysym_to_char(keysym), expected, "keysym {keysym:#x}");
        }
    }

    #[test]
    fn greek_trigger_typed_with_legacy_keysyms_expands() {
        let config: Config =
            toml::from_str("[[expansion]]\ntrigger = \";αβ\"\nreplacement = \"alpha beta\"\n")
                .unwrap();
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));
        let mut client = Client::default();
        client.key(&mut adapter, ';' as u32, 0);
        client.key(&mut adapter, 0x07e1, 0); // Greek_alpha
        client.key(&mut adapter, 0x07e2, 0); // Greek_beta
        assert_eq!(client.text(), "alpha beta");
    }

    #[test]
    fn async_emit_failure_resets_without_recording_the_expansion() {
        let mut adapter = adapter();
        let mut calls = 0;
        let result = adapter.drain_completed_commands_with(|_| {
            calls += 1;
            Err::<(), _>("unreachable")
        });
        // Nothing completed, so nothing is emitted.
        assert_eq!(result, Ok(0));
        assert_eq!(calls, 0);
    }
}
