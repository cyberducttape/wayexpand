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
    injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    let mut pending_results = Vec::new();
    process_event_with_pending_buffer(
        engine,
        event,
        injector,
        policy,
        active_backend,
        &mut pending_results,
    )
}

/// Process an event while reusing caller-owned storage for deferred match
/// results. The vector is drained before dispatch returns, retaining its
/// capacity without allowing jobs to borrow from it.
pub(crate) fn process_event_with_pending_buffer(
    engine: &mut ExpansionEngine,
    event: InputEvent,
    mut injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
    pending_results: &mut Vec<wayexpand_core::PendingExpansionResult>,
) -> std::result::Result<(), Box<EventError>> {
    if let InputEvent::Key(chord) = event {
        pending_results.clear();
        // Undo is a transaction too: do not consume the undo record until
        // there is an injector and the replacement has been applied. This
        // preserves retryability across backend reconnects and failures.
        if let Some(result) = process_key_event(engine, chord, injector.is_some(), policy) {
            if let Some(backend) = injector.as_deref_mut() {
                let outcome = latency::apply(backend, &result);
                return finish_undo(engine, result, outcome);
            }
        }
        return Ok(());
    }
    // Check policy before executing deferred commands.
    latency::measure_matcher(|| engine.process_deferred_into(event, pending_results));
    apply_pending_results_with_pending_buffer(
        engine,
        pending_results,
        injector,
        policy,
        active_backend,
    )
}

/// Match a key event and queue its hotkey actions. Returns the undo
/// transaction to apply, prepared only when an injector is available.
pub(crate) fn process_key_event(
    engine: &mut ExpansionEngine,
    chord: wayexpand_core::KeyChord,
    injector_available: bool,
    policy: &wayexpand_core::OrganizationPolicy,
) -> Option<ExpansionResult> {
    {
        // Invalidate any pending asynchronous expansions before processing
        // the hotkey. This also updates undo validity: undo is only preserved
        // for the undo chord itself; all other keys invalidate it.
        let _ = latency::measure_matcher(|| engine.process(InputEvent::Key(chord.clone())));

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
    }
    injector_available
        .then(|| engine.prepare_undo(&chord))
        .flatten()
}

/// Record the outcome of an undo transaction in the engine.
pub(crate) fn finish_undo(
    engine: &mut ExpansionEngine,
    result: ExpansionResult,
    outcome: wayexpand_core::TransactionOutcome,
) -> std::result::Result<(), Box<EventError>> {
    if !outcome.is_applied() {
        // Only a transaction known not to have started may retain the undo
        // record for a retry. After a possibly partial one, replaying the
        // undo would erase text the user never typed, so drop it.
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
    if let wayexpand_core::TransactionOutcome::AppliedWithCursorPositionFailure { ref source } =
        outcome
    {
        warn!(%source, "undo applied but cursor repositioning failed");
    }
    engine.commit_undo(&result);
    info!("expansion undone");
    Ok(())
}

pub(crate) fn apply_pending_results_with_pending_buffer(
    engine: &mut ExpansionEngine,
    pending: &mut Vec<wayexpand_core::PendingExpansionResult>,
    injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    let results =
        dispatch_pending_results_with_pending_buffer(engine, pending, policy, active_backend);
    apply_results(engine, results, injector, policy, active_backend)
}

/// Apply policy and dispatch deferred expansions without injecting ready results.
///
/// Keeping this stage separate lets evdev perform its physical key-release and
/// input-quiet checks before injection, while still guaranteeing that command
/// expansions are dispatched through the bounded asynchronous worker.
#[cfg(test)]
pub(crate) fn dispatch_pending_results(
    engine: &mut ExpansionEngine,
    pending: Vec<wayexpand_core::PendingExpansionResult>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> Vec<ExpansionResult> {
    let mut pending = pending;
    dispatch_pending_results_with_pending_buffer(engine, &mut pending, policy, active_backend)
}

pub(crate) fn dispatch_pending_results_with_pending_buffer(
    engine: &mut ExpansionEngine,
    pending: &mut Vec<wayexpand_core::PendingExpansionResult>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> Vec<ExpansionResult> {
    let mut results = Vec::new();
    for pending_result in pending.drain(..) {
        let has_command = pending_result.command.is_some();

        if let Some(command) = &pending_result.command {
            if command.action.is_none() {
                if let Some(violation) = policy.command_path_violation(&command.program) {
                    policy::log_violation(policy, &violation);
                    if policy.command_path_is_blocked(&command.program) {
                        failure_trace::record_blocked_expansion(
                            active_backend,
                            "blocked by command policy",
                        );
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
            failure_trace::record_blocked_expansion(
                active_backend,
                "blocked by organization policy",
            );
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
                if has_command {
                    failure_trace::record_command_failure(active_backend, error.kind());
                } else {
                    failure_trace::record_preparation_failure(active_backend, &error);
                }
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
    active_backend: &str,
) -> EvdevGatingOutcome {
    let mut follow_up = Vec::new();
    if let Some(source) = evdev.as_mut() {
        // Wait for physical key release before injecting synthetic input.
        // Injecting while trigger key is held can make synthetic input appear
        // as auto-repeat or cancel the physical release.
        if !evdev_release_is_safe(source.wait_for_key_release(KEY_RELEASE_TIMEOUT)) {
            record_abandoned_expansions(
                active_backend,
                results.len(),
                "key release was not confirmed before output",
            );
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
                record_abandoned_expansions(
                    active_backend,
                    results.len(),
                    "input quiet check failed",
                );
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
                    record_abandoned_expansions(
                        active_backend,
                        results.len(),
                        "input arrived before output",
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
                record_abandoned_expansions(
                    active_backend,
                    results.len(),
                    "multiple inputs arrived before output",
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

fn record_abandoned_expansions(backend: &str, count: usize, reason: &'static str) {
    for _ in 0..count {
        failure_trace::record_blocked_expansion(backend, reason);
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

/// Match a non-key evdev event, gate its results on the physical key state,
/// and inject them. Failures are returned, not propagated past the caller,
/// which must restore its injector before classifying them for recovery.
#[cfg(test)]
pub(crate) fn apply_deferred_evdev_event(
    engine: &mut ExpansionEngine,
    event: InputEvent,
    backend: &mut dyn TextInjector,
    evdev: &mut Option<EvdevSource>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    let mut pending_results = Vec::new();
    apply_deferred_evdev_event_with_pending_buffer(
        engine,
        event,
        backend,
        evdev,
        policy,
        active_backend,
        &mut pending_results,
    )
}

pub(crate) fn apply_deferred_evdev_event_with_pending_buffer(
    engine: &mut ExpansionEngine,
    event: InputEvent,
    backend: &mut dyn TextInjector,
    evdev: &mut Option<EvdevSource>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
    pending_results: &mut Vec<wayexpand_core::PendingExpansionResult>,
) -> std::result::Result<(), Box<EventError>> {
    engine.process_deferred_into(event, pending_results);
    let results = dispatch_pending_results_with_pending_buffer(
        engine,
        pending_results,
        policy,
        active_backend,
    );
    if results.is_empty() {
        return Ok(());
    }
    let gating = apply_evdev_gating(results, evdev, active_backend);
    restore_abandoned_results(engine, gating.abandoned);
    apply_results(
        engine,
        gating.results,
        Some(backend),
        policy,
        active_backend,
    )?;
    replay_evdev_follow_up_with_pending_buffer(
        engine,
        gating.follow_up,
        policy,
        active_backend,
        pending_results,
    )
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
    let mut pending_results = Vec::new();
    replay_evdev_follow_up_with_pending_buffer(
        engine,
        follow_up,
        policy,
        active_backend,
        &mut pending_results,
    )
}

pub(crate) fn replay_evdev_follow_up_with_pending_buffer(
    engine: &mut ExpansionEngine,
    follow_up: Vec<InputEvent>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
    pending_results: &mut Vec<wayexpand_core::PendingExpansionResult>,
) -> std::result::Result<(), Box<EventError>> {
    for event in follow_up {
        process_event_with_pending_buffer(
            engine,
            event,
            None,
            policy,
            active_backend,
            pending_results,
        )?;
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

/// Apply organization policy to a ready expansion. A blocked expansion is
/// returned to the matcher and must not be output.
pub(crate) fn expansion_allowed(
    engine: &mut ExpansionEngine,
    result: &ExpansionResult,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> bool {
    // Use explicit provenance instead of heuristic: command_backed is set by engine
    if policy::check_and_log_expansion_violations(
        policy,
        result.insert.len(),
        result.command_backed,
        active_backend,
    ) {
        // In safe_mode, block the expansion
        failure_trace::record_blocked_expansion(active_backend, "blocked by organization policy");
        engine.restore_deferred_result(result);
        return false;
    }
    true
}

/// Record an expansion transaction outcome. Only a known-not-applied result
/// is restored for retry. Uncertain results discard the deferred reservation
/// so the engine cannot replay a trigger that may already have been replaced.
pub(crate) fn finish_expansion(
    engine: &mut ExpansionEngine,
    result: ExpansionResult,
    outcome: wayexpand_core::TransactionOutcome,
) -> std::result::Result<(), Box<EventError>> {
    match outcome {
        wayexpand_core::TransactionOutcome::Applied => {}
        wayexpand_core::TransactionOutcome::AppliedWithCursorPositionFailure { ref source } => {
            warn!(%source, "expansion applied but cursor repositioning failed");
        }
        wayexpand_core::TransactionOutcome::NotApplied { .. } => {
            engine.restore_deferred_result(&result);
            return Err(Box::new(EventError {
                result,
                source: outcome,
            }));
        }
        wayexpand_core::TransactionOutcome::UnknownApplicationOutcome { .. } => {
            engine.discard_deferred_result(&result);
            warn!("expansion replacement outcome is unknown; refusing automatic retry");
            return Err(Box::new(EventError {
                result,
                source: outcome,
            }));
        }
        wayexpand_core::TransactionOutcome::UnknownPartialFailure { .. } => {
            engine.discard_deferred_result(&result);
            warn!("expansion transaction may have partially applied; refusing automatic retry");
            return Err(Box::new(EventError {
                result,
                source: outcome,
            }));
        }
    }
    engine.commit_applied_expansion(&result);
    info!(
        trigger_chars = result.trigger.chars().count(),
        insert_bytes = result.insert.len(),
        "expansion injected"
    );
    Ok(())
}

pub(crate) fn apply_results(
    engine: &mut ExpansionEngine,
    results: Vec<ExpansionResult>,
    mut injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    for result in results {
        if !expansion_allowed(engine, &result, policy, active_backend) {
            continue;
        }

        if let Some(backend) = injector.as_deref_mut() {
            let outcome = latency::apply(backend, &result);
            finish_expansion(engine, result, outcome)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn deferred_engine_and_result() -> (ExpansionEngine, ExpansionResult) {
        let config = wayexpand_core::Config::parse(
            r#"
            [[expansion]]
            trigger = ":x"
            replacement = "replacement"
        "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let pending = engine
            .process_deferred(InputEvent::Text(":x".into()))
            .pop()
            .expect("trigger should be reserved");
        let result = match engine.dispatch_pending_with_policy(pending, 0).unwrap() {
            wayexpand_core::PendingExpansionDispatch::Ready(result) => result,
            wayexpand_core::PendingExpansionDispatch::Queued => {
                panic!("static replacement must be ready")
            }
        };
        (engine, result)
    }

    fn failure() -> wayexpand_core::InjectorError {
        wayexpand_core::InjectorError {
            backend: "test",
            message: "output acknowledgement lost".into(),
            retryable: true,
        }
    }

    #[test]
    fn known_not_applied_result_restores_the_deferred_trigger_for_retry() {
        let (mut engine, result) = deferred_engine_and_result();
        let failed = finish_expansion(
            &mut engine,
            result,
            wayexpand_core::TransactionOutcome::NotApplied { source: failure() },
        );

        assert!(failed.is_err());
        assert_eq!(
            engine.process_deferred(InputEvent::Text(" ".into())).len(),
            1
        );
    }

    #[test]
    fn uncertain_results_discard_the_deferred_trigger_instead_of_replaying_it() {
        for outcome in [
            wayexpand_core::TransactionOutcome::UnknownApplicationOutcome { source: failure() },
            wayexpand_core::TransactionOutcome::UnknownPartialFailure { source: failure() },
        ] {
            let (mut engine, result) = deferred_engine_and_result();

            let error = finish_expansion(&mut engine, result, outcome).unwrap_err();

            assert!(error.retryable());
            assert!(engine
                .process_deferred(InputEvent::Text(" ".into()))
                .is_empty());
        }
    }

    #[test]
    fn organization_policy_blocks_are_recorded_without_expansion_content() {
        let config = wayexpand_core::Config::parse(
            r#"
            [[expansion]]
            trigger = ":x"
            replacement = "private replacement text"
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let pending = engine.process_deferred(InputEvent::Text(":x".into()));
        let policy = wayexpand_core::OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["allowed-route".into()],
            ..Default::default()
        };

        let results = dispatch_pending_results(&mut engine, pending, &policy, "trace-test-route");

        assert!(results.is_empty());
        let outcomes: serde_json::Value =
            serde_json::from_str(&crate::failure_trace::recent_outcomes_json()).unwrap();
        let refusal = outcomes
            .as_array()
            .unwrap()
            .iter()
            .find(|trace| {
                trace["backend"] == "trace-test-route"
                    && trace["completion"] == "blocked by organization policy"
            })
            .expect("the blocked matched expansion should be visible in diagnostics");
        assert_eq!(refusal["trigger_recognized"], "yes");
        assert_eq!(refusal["focus_admission"], "not reached");
        assert_eq!(refusal["erase_acknowledged"], "not attempted");
        assert_eq!(refusal["insertion_attempted"], false);
        let refusal_json = serde_json::to_string(refusal).unwrap();
        assert!(!refusal_json.contains(":x"));
        assert!(!refusal_json.contains("private replacement text"));
    }
}
