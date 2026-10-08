//! Expansion event processing: policy-checked dispatch, evdev key-release gating, and applying results through the injector.

use crate::*;

pub(crate) fn evdev_release_is_safe<E: std::fmt::Display>(release: Result<(), E>) -> bool {
    match release {
        Ok(()) => true,
        Err(error) => {
            warn!(
                %error,
                "waiting for key release failed; dropping expansion because safe injection cannot be verified"
            );
            false
        }
    }
}

pub(crate) fn process_event(
    engine: &mut ExpansionEngine,
    event: InputEvent,
    mut injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    if let InputEvent::Key(chord) = event.clone() {
        // Invalidate any pending asynchronous expansions before processing
        // the hotkey. This also updates undo validity: undo is only preserved
        // for the undo chord itself; all other keys invalidate it.
        let _ = engine.process(event);

        for action in engine.process_key(&chord) {
            // Apply the organization policy only to configured hotkey
            // actions. Ordinary evdev key events must not be treated as
            // hotkey violations, and undo is handled independently below.
            if let Err(violation) = policy::check_hotkey_allowed(policy) {
                policy::log_violation(policy, &violation);
                if policy.safe_mode {
                    continue;
                }
            }
            if action.command.action.is_none() {
                if let Some(violation) = policy.command_path_violation(&action.command.program) {
                    policy::log_violation(policy, &violation);
                    if policy.command_path_is_blocked(&action.command.program) {
                        continue;
                    }
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
                    let outcome = latency::apply(backend, &result);
                    if !outcome.is_applied() {
                        // Only a transaction known not to have started may
                        // retain the undo record for a retry. After a
                        // possibly partial one, replaying the undo would
                        // erase text the user never typed, so drop it.
                        if !matches!(
                            outcome,
                            wayexpand_core::TransactionOutcome::NotApplied { .. }
                        ) {
                            engine.commit_undo(&result);
                        }
                        return Err(Box::new(EventError {
                            result,
                            source: outcome,
                        }));
                    }
                    if let wayexpand_core::TransactionOutcome::AppliedWithCursorPositionFailure {
                        ref source,
                    } = outcome
                    {
                        warn!(%source, "undo applied but cursor repositioning failed");
                    }
                    engine.commit_undo(&result);
                    info!("expansion undone");
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
pub(crate) fn apply_pending_results(
    engine: &mut ExpansionEngine,
    pending: Vec<wayexpand_core::PendingExpansionResult>,
    injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
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
pub(crate) fn dispatch_pending_results(
    engine: &mut ExpansionEngine,
    pending: Vec<wayexpand_core::PendingExpansionResult>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> Vec<ExpansionResult> {
    let mut results = Vec::new();
    for pending_result in pending {
        let has_command = pending_result.command.is_some();

        if let Some(command) = &pending_result.command {
            if command.action.is_none() {
                if let Some(violation) = policy.command_path_violation(&command.program) {
                    policy::log_violation(policy, &violation);
                    if policy.command_path_is_blocked(&command.program) {
                        engine.restore_deferred_match(&pending_result.matched_text);
                        continue;
                    }
                }
            }
        }

        // Check policy BEFORE executing commands
        if policy::check_and_log_expansion_violations(
            policy,
            pending_result.template_text.len(),
            has_command,
            active_backend,
        ) {
            // In safe_mode, block the expansion
            engine.restore_deferred_match(&pending_result.matched_text);
            continue;
        }

        // Policy approved: command-backed expansions go to the bounded worker;
        // static replacements and cache hits are ready immediately.
        let enforcement_policy = policy.effective_enforcement_policy();
        let result = match engine
            .dispatch_pending_with_policy(pending_result, enforcement_policy.max_replacement_size)
        {
            Ok(wayexpand_core::PendingExpansionDispatch::Ready(result)) => result,
            Ok(wayexpand_core::PendingExpansionDispatch::Queued) => continue,
            Err(error) => {
                warn!(%error, "expansion could not be queued or completed");
                continue;
            }
        };
        results.push(result);
    }
    results
}

/// Apply evdev safety gating to expansion results before injection.
/// Ensures physical key-up event is processed and no competing input arrived.
/// Only applies when evdev source is available and in use.
pub(crate) struct EvdevGatingOutcome {
    pub(crate) results: Vec<ExpansionResult>,
    pub(crate) follow_up: Vec<InputEvent>,
    pub(crate) abandoned: Vec<ExpansionResult>,
}

pub(crate) fn apply_evdev_gating(
    mut results: Vec<ExpansionResult>,
    evdev: &mut Option<EvdevSource>,
) -> EvdevGatingOutcome {
    let mut follow_up = Vec::new();
    if let Some(source) = evdev.as_mut() {
        // Wait for physical key release before injecting synthetic input.
        // Injecting while trigger key is held can make synthetic input appear
        // as auto-repeat or cancel the physical release.
        if !evdev_release_is_safe(source.wait_for_key_release(KEY_RELEASE_TIMEOUT)) {
            follow_up = source.take_pending_events();
            return EvdevGatingOutcome {
                results: Vec::new(),
                follow_up,
                abandoned: results,
            };
        }

        // Check if any input arrived during key release wait.
        let input_quiet = match source.wait_for_input_quiet(EVDEV_QUIET_TIMEOUT) {
            Ok(quiet) => quiet,
            Err(error) => {
                warn!(%error, "evdev quiet-period check failed; abandoning expansion");
                // Preserve anything captured before the polling error so the
                // matcher sees the same stream as the focused application.
                follow_up = source.take_pending_events();
                return EvdevGatingOutcome {
                    results: Vec::new(),
                    follow_up,
                    abandoned: results,
                };
            }
        };

        // If other input arrived, handle delimiter preservation or drop expansion
        if !input_quiet {
            follow_up = source.take_pending_events();
            if follow_up.len() == 1 {
                if let InputEvent::Delimiter(character) = &follow_up[0] {
                    // A single delimiter belongs after the complete batch of
                    // results, not after every result in it. Absorb it into
                    // the final erase/reinsert transaction so it is removed
                    // and restored exactly once.
                    absorb_evdev_delimiter(&mut results, *character);
                    // The delimiter is represented in the adjusted result and
                    // must not be replayed a second time through the matcher.
                    follow_up.clear();
                } else {
                    // Other input arrived: don't inject (avoid cursor misplacement)
                    warn!(
                        "input arrived while waiting for key release; dropping expansion to avoid cursor misplacement"
                    );
                    let abandoned = results.clone();
                    results.clear();
                    return EvdevGatingOutcome {
                        results,
                        follow_up,
                        abandoned,
                    };
                }
            } else if follow_up.len() > 1 {
                // Multiple inputs arrived: don't inject
                warn!(
                    "multiple inputs arrived while waiting for key release; dropping expansion to avoid cursor misplacement"
                );
                let abandoned = results.clone();
                results.clear();
                return EvdevGatingOutcome {
                    results,
                    follow_up,
                    abandoned,
                };
            }
        }
    }
    EvdevGatingOutcome {
        results,
        follow_up,
        abandoned: Vec::new(),
    }
}

pub(crate) fn absorb_evdev_delimiter(results: &mut [ExpansionResult], character: char) {
    if let Some(result) = results.last_mut() {
        // The current delimiter may already be represented separately in
        // `reinsert_after`. Once another delimiter arrives while evdev is
        // waiting for a safe injection point, fold that first delimiter into
        // the transaction before appending the follow-up character. This
        // preserves the exact order already present in the application.
        if let Some(first) = result.reinsert_after.take() {
            result.fold_typed_suffix(first);
        }
        result.fold_typed_suffix(character);
    }
}

/// Replay input captured during evdev gating through the matcher. The focused
/// application receives these events through non-exclusive capture already;
/// this keeps the engine's buffer and boundary state in sync without applying
/// a second replacement for the same physical input.
pub(crate) fn replay_evdev_follow_up(
    engine: &mut ExpansionEngine,
    follow_up: Vec<InputEvent>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    for event in follow_up {
        process_event(engine, event, None, policy, active_backend)?;
    }
    Ok(())
}

pub(crate) fn restore_abandoned_results(
    engine: &mut ExpansionEngine,
    results: Vec<ExpansionResult>,
) {
    for result in results {
        engine.restore_deferred_result(&result);
    }
}

pub(crate) fn apply_results(
    engine: &mut ExpansionEngine,
    results: Vec<ExpansionResult>,
    mut injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    for result in results {
        // Check if expansion violates policy and log if needed
        // Use explicit provenance instead of heuristic: command_backed is set by engine
        if policy::check_and_log_expansion_violations(
            policy,
            result.insert.len(),
            result.command_backed,
            active_backend,
        ) {
            // In safe_mode, block the expansion
            engine.restore_deferred_result(&result);
            continue;
        }

        if let Some(backend) = injector.as_deref_mut() {
            let inject_result = latency::apply(backend, &result);

            match inject_result {
                wayexpand_core::TransactionOutcome::Applied => {}
                wayexpand_core::TransactionOutcome::AppliedWithCursorPositionFailure {
                    ref source,
                } => {
                    warn!(%source, "expansion applied but cursor repositioning failed");
                }
                wayexpand_core::TransactionOutcome::NotApplied { .. } => {
                    engine.restore_deferred_result(&result);
                    return Err(Box::new(EventError {
                        result,
                        source: inject_result,
                    }));
                }
                wayexpand_core::TransactionOutcome::UnknownPartialFailure { .. } => {
                    warn!("expansion transaction may have partially applied; refusing automatic retry");
                    return Err(Box::new(EventError {
                        result,
                        source: inject_result,
                    }));
                }
            }
            engine.commit_applied_expansion(&result);
            info!(
                trigger_chars = result.trigger.chars().count(),
                insert_bytes = result.insert.len(),
                "expansion injected"
            );
        } else {
            engine.restore_deferred_result(&result);
            info!(
                trigger_chars = result.trigger.chars().count(),
                matched_chars = result.matched_text.chars().count(),
                insert_bytes = result.insert.len(),
                "expansion matched"
            );
        }
    }
    Ok(())
}
