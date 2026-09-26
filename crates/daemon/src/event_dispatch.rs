//! Event processing and dispatching logic for the expansion daemon.
//!
//! Handles the core event-to-result pipeline:
//! 1. Receive input events (text, keys, delimiters)
//! 2. Process through the expansion engine
//! 3. Apply organization policy
//! 4. Inject results to output backend

use std::sync::atomic::AtomicBool;
use tracing::warn;
use wayexpand_core::{ExpansionEngine, ExpansionResult, InputEvent, KeyChord, OrganizationPolicy, TextInjector};

#[derive(Debug)]
pub struct EventError {
    pub result: ExpansionResult,
    pub source: wayexpand_core::ExpansionError,
}

impl EventError {
    pub fn retryable(&self) -> bool {
        matches!(
            &self.source,
            wayexpand_core::ExpansionError::Injection(error) if error.retryable
        )
    }

    pub fn expansion_rejected(&self) -> bool {
        matches!(
            &self.source,
            wayexpand_core::ExpansionError::Injection(error)
                if error.kind() == wayexpand_core::InjectorErrorKind::ExpansionRejected
        )
    }
}

impl std::fmt::Display for EventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "expansion for {:?} ({} bytes) failed",
            self.result.trigger,
            self.result.insert.len()
        )
    }
}

impl std::error::Error for EventError {}

/// Process an input event through the engine and apply policy.
/// For key events, handles hotkeys and undo separately from text expansion.
pub fn process_event(
    engine: &mut ExpansionEngine,
    event: InputEvent,
    mut injector: Option<&mut dyn TextInjector>,
    policy: &OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    if let InputEvent::Key(chord) = event.clone() {
        // Invalidate any pending asynchronous expansions before processing
        // the hotkey. This also updates undo validity: undo is only preserved
        // for the undo chord itself; all other keys invalidate it.
        engine.process(event);

        for action in engine.process_key(&chord) {
            // Apply the organization policy only to configured hotkey
            // actions. Ordinary evdev key events must not be treated as
            // hotkey violations, and undo is handled independently below.
            if let Err(violation) = crate::policy::check_hotkey_allowed(policy) {
                crate::policy::log_violation(policy, &violation);
                if policy.safe_mode {
                    continue;
                }
            }
            if let Some(violation) = policy.command_path_violation(&action.command.program) {
                crate::policy::log_violation(policy, &violation);
                if policy.command_path_is_blocked(&action.command.program) {
                    continue;
                }
            }
            if let Err(error) = engine.queue_hotkey(&action) {
                warn!(
                    chord = %action.chord,
                    %error,
                    "hotkey action was not queued"
                );
            }
        }
        // Undo is a transaction too: do not consume the undo record until
        // there is an injector and the replacement has been applied. This
        // preserves retryability across backend reconnects and failures.
        if injector.is_some() {
            if let Some(result) = engine.prepare_undo(&chord) {
                if let Some(backend) = injector.as_deref_mut() {
                    if let Err(source) = ExpansionEngine::apply(backend, &result) {
                        return Err(Box::new(EventError { result, source }));
                    }
                    engine.commit_undo(&result);
                    tracing::info!("expansion undone");
                }
            }
        }
        return Ok(());
    }
    // Check policy before executing deferred commands.
    let pending = engine.process_deferred(event);
    apply_pending_results(engine, pending, injector, policy, active_backend)
}

/// Apply deferred expansion results after policy pre-approval.
/// Checks policy before executing commands, preventing irreversible side effects.
fn apply_pending_results(
    engine: &mut ExpansionEngine,
    pending: Vec<wayexpand_core::PendingExpansionResult>,
    injector: Option<&mut dyn TextInjector>,
    policy: &OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    let results = dispatch_pending_results(engine, pending, policy, active_backend);
    apply_results(engine, results, injector, policy, active_backend)
}

/// Apply policy and dispatch deferred expansions without injecting ready results.
///
/// Keeping this stage separate lets evdev perform its physical key-release and
/// input-quiet checks before injection, while still guaranteeing that command
/// expansions are dispatched through the bounded asynchronous worker.
pub fn dispatch_pending_results(
    engine: &mut ExpansionEngine,
    pending: Vec<wayexpand_core::PendingExpansionResult>,
    policy: &OrganizationPolicy,
    active_backend: &str,
) -> Vec<ExpansionResult> {
    let mut results = Vec::new();
    for pending_result in pending {
        let has_command = pending_result.command.is_some();

        if let Some(command) = &pending_result.command {
            if let Some(violation) = policy.command_path_violation(&command.program) {
                crate::policy::log_violation(policy, &violation);
                if policy.command_path_is_blocked(&command.program) {
                    engine.restore_deferred_match(&pending_result.matched_text);
                    continue;
                }
            }
        }

        // If the expansion has an async command, it must be enqueued through
        // the bounded worker. If the worker is unavailable (startup failure) and
        // policy enforcement is active, the entire expansion is discarded.
        let command_policy_applies = has_command
            && (crate::policy::commands_enforced(policy) || policy.safe_mode);
        if has_command && command_policy_applies {
            if let Some(command) = pending_result.command.as_ref() {
                match engine.queue_command(&pending_result, command) {
                    Ok(()) => {
                        results.push(pending_result.into());
                    }
                    Err(error) => {
                        warn!(
                            %error,
                            trigger = %pending_result.trigger,
                            "command could not be queued; expansion discarded"
                        );
                        engine.restore_deferred_match(&pending_result.matched_text);
                    }
                }
            }
        } else {
            results.push(pending_result.into());
        }
    }
    results
}

/// Apply expansion results: erase trigger, inject replacement, handle undo.
pub fn apply_results(
    engine: &mut ExpansionEngine,
    results: Vec<ExpansionResult>,
    mut injector: Option<&mut dyn TextInjector>,
    policy: &OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    for result in results {
        if let Some(backend) = injector.as_deref_mut() {
            let inject_result = ExpansionEngine::apply(backend, &result);

            if let Err(source) = inject_result {
                return Err(Box::new(EventError { result, source }));
            }

            engine.register_expansion(result);
        }
    }
    Ok(())
}
