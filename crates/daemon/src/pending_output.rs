//! Output transactions in flight on the serialized output worker.
//!
//! On the serialized route (evdev capture with libei output) a transaction
//! can take seconds: a paced backend types each character, and a wedged
//! portal only fails at the acknowledgement deadline. Waiting for it inside
//! the reactor stalled stop, status, and `explain` for that long. Instead the
//! transaction is submitted to the worker and kept here as pending state.
//!
//! While a transaction is pending the reactor services only work that cannot
//! change what is typed or how its outcome is recorded: stop, status
//! publication, and `explain`. Input, window changes, pause, reload, command
//! completions, and inserts wait until the outcome is known, exactly as they
//! did while the reactor blocked. That ordering matters: for example, an undo
//! record must not be attached to a window that gained focus while the
//! replacement was still being typed.

use std::collections::VecDeque;

use wayexpand_core::InjectorError;

use crate::events::{
    dispatch_pending_results_with_pending_buffer, expansion_allowed,
    finish_expansion_with_retry_allowed, finish_undo_with_retry_allowed, process_key_event,
};
#[cfg(test)]
use crate::output_loop::spawn_async_injector;
use crate::output_loop::SubmittedTransaction;
use crate::source_steps::Step;
use crate::*;

/// Physical keys must be checked frequently while libei is pacing a
/// non-atomic replacement. The normal maintenance interval (250 ms) is long
/// enough for many simulated characters to be delivered before cancellation.
/// Keep this short and use it only while such a transaction is active.
const NON_ATOMIC_INPUT_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Integrity state for one submitted output transaction. Physical input is a
/// monotonic invalidation: once observed, even a later `Applied` worker result
/// is uncertain because the last synthetic event may have raced with that
/// input. The reactor drains input before resolving a ready completion, so
/// both completion/input orderings pass through this same gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OutputIntegrityGate {
    physical_input_interrupted: bool,
    resolved_invalidate_context: Option<bool>,
}

impl OutputIntegrityGate {
    pub(crate) fn interrupt_for_physical_input(&mut self) {
        if self.resolved_invalidate_context.is_none() {
            self.physical_input_interrupted = true;
        }
    }

    fn was_interrupted(self) -> bool {
        self.physical_input_interrupted
    }

    pub(crate) fn resolve(&mut self, invalidate_context: bool) -> bool {
        if let Some(resolved) = self.resolved_invalidate_context {
            return resolved;
        }
        let resolved = self.physical_input_interrupted || invalidate_context;
        self.resolved_invalidate_context = Some(resolved);
        resolved
    }
}

/// Where a batch of output came from; failures are handled as on the
/// synchronous path for that source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputOrigin {
    /// Typed input: a failure skips the evdev follow-up replay.
    Typed,
    /// A completed command-backed expansion.
    Command,
    /// A picker or `wayexpand insert` request; failures are never fatal.
    Insert,
}

/// What the reactor may do with queued work after resolving one output job.
/// Keep this decision pure so adversarial sequence tests exercise the same
/// branch that controls the live follow-up path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputContinuation {
    Quarantine,
    AdvanceBatch,
    ReplayFollowUp,
}

pub(crate) fn output_continuation(
    origin: OutputOrigin,
    invalidate_context: bool,
    completed: bool,
) -> OutputContinuation {
    if invalidate_context || (!completed && origin == OutputOrigin::Typed) {
        OutputContinuation::Quarantine
    } else if completed {
        OutputContinuation::AdvanceBatch
    } else {
        OutputContinuation::ReplayFollowUp
    }
}

/// Whether the backend's terminal result proves that the matcher context can
/// no longer be trusted. A clean pre-output refusal normally permits retry,
/// but a refusal specifically caused by focus loss must drop stale context.
pub(crate) fn outcome_invalidates_context(outcome: &wayexpand_core::TransactionOutcome) -> bool {
    match outcome {
        wayexpand_core::TransactionOutcome::UnknownApplicationOutcome { .. }
        | wayexpand_core::TransactionOutcome::UnknownPartialFailure { .. }
        | wayexpand_core::TransactionOutcome::AppliedWithCursorPositionFailure { .. } => true,
        wayexpand_core::TransactionOutcome::NotApplied { source } => {
            source.invalidates_target_context()
        }
        wayexpand_core::TransactionOutcome::Applied => false,
    }
}

pub(crate) enum OutputJob {
    Expansion(ExpansionResult),
    Undo(ExpansionResult),
}

impl OutputJob {
    fn split_payload(self) -> (Self, ExpansionResult) {
        fn metadata(result: &ExpansionResult) -> ExpansionResult {
            ExpansionResult {
                snippet_id: Arc::clone(&result.snippet_id),
                trigger: Arc::clone(&result.trigger),
                matched_text: result.matched_text.clone(),
                insert: String::new().into(),
                cursor_offset: result.cursor_offset,
                reinsert_after: result.reinsert_after,
                command_backed: result.command_backed,
                undoable: result.undoable,
                folded_suffix_chars: result.folded_suffix_chars,
            }
        }

        match self {
            Self::Expansion(result) => {
                let pending = metadata(&result);
                (Self::Expansion(pending), result)
            }
            Self::Undo(result) => {
                let pending = metadata(&result);
                (Self::Undo(pending), result)
            }
        }
    }

    fn with_result(self, result: ExpansionResult) -> Self {
        match self {
            Self::Expansion(_) => Self::Expansion(result),
            Self::Undo(_) => Self::Undo(result),
        }
    }
}

pub(crate) struct PendingOutput {
    origin: OutputOrigin,
    in_flight: OutputJob,
    transaction: SubmittedTransaction,
    queued: VecDeque<OutputJob>,
    follow_up: Vec<InputEvent>,
    integrity: OutputIntegrityGate,
    interrupted_at: Option<Instant>,
}

impl Daemon {
    /// Whether output goes through the serialized worker as pending
    /// transactions rather than blocking calls.
    pub(crate) fn serialized_output(&self) -> bool {
        self.injector.is_some()
            && self
                .output_worker
                .as_ref()
                .is_some_and(|worker| worker.submitter().is_some())
    }

    /// Start a batch of output on the serialized worker. Policy-blocked
    /// expansions are returned to the matcher first. `follow_up` is input
    /// captured during evdev gating; it is replayed through the matcher only
    /// after the batch has been output, as on the synchronous path.
    pub(crate) fn start_output(
        &mut self,
        origin: OutputOrigin,
        jobs: Vec<OutputJob>,
        follow_up: Vec<InputEvent>,
    ) -> Result<()> {
        debug_assert!(self.pending_output.is_none());
        let mut allowed = VecDeque::with_capacity(jobs.len());
        for job in jobs {
            let permitted = match &job {
                OutputJob::Expansion(result) => expansion_allowed(
                    &mut self.config.engine,
                    result,
                    &self.policy,
                    self.active_backend,
                ),
                OutputJob::Undo(_) => true,
            };
            if permitted {
                if matches!(&job, OutputJob::Expansion(_)) {
                    failure_trace::note_expansion_attempt();
                }
                allowed.push_back(job);
            }
        }
        self.advance_output(origin, allowed, follow_up)
    }

    /// Submit the next job, or finish the batch when none remain.
    fn advance_output(
        &mut self,
        origin: OutputOrigin,
        mut queued: VecDeque<OutputJob>,
        follow_up: Vec<InputEvent>,
    ) -> Result<()> {
        while let Some(job) = queued.pop_front() {
            let (pending_job, result) = job.split_payload();
            let submitted = match self
                .output_worker
                .as_ref()
                .and_then(OutputWorkerState::submitter)
            {
                Some(submitter) => submitter.submit(result),
                None => Err(Box::new((
                    InjectorError {
                        backend: self.active_backend,
                        message: "serialized output worker is no longer available".into(),
                        retryable: true,
                    },
                    result,
                ))),
            };
            match submitted {
                Ok(transaction) => {
                    self.pending_output = Some(PendingOutput {
                        origin,
                        in_flight: pending_job,
                        transaction,
                        queued,
                        follow_up,
                        integrity: OutputIntegrityGate::default(),
                        interrupted_at: None,
                    });
                    return Ok(());
                }
                // The worker never received the transaction, so nothing was
                // typed: report it as not applied.
                Err(error) => {
                    let (source, result) = *error;
                    let outcome = wayexpand_core::TransactionOutcome::NotApplied { source };
                    if !self.complete_job(origin, pending_job.with_result(result), outcome, true)? {
                        return self.end_failed_batch(origin, follow_up);
                    }
                }
            }
        }
        self.replay_follow_up(follow_up)
    }

    /// Record a job's outcome. Returns false when it failed; the failure has
    /// been handled and the rest of the batch must not be output.
    fn complete_job(
        &mut self,
        origin: OutputOrigin,
        job: OutputJob,
        outcome: wayexpand_core::TransactionOutcome,
        retry_allowed: bool,
    ) -> Result<bool> {
        let finished = match job {
            OutputJob::Expansion(result) => finish_expansion_with_retry_allowed(
                &mut self.config.engine,
                result,
                outcome,
                retry_allowed,
            ),
            OutputJob::Undo(result) => finish_undo_with_retry_allowed(
                &mut self.config.engine,
                result,
                outcome,
                retry_allowed,
            ),
        };
        let Err(error) = finished else {
            return Ok(true);
        };
        match origin {
            OutputOrigin::Typed | OutputOrigin::Command => {
                self.recover_from_output_error(*error)?
            }
            OutputOrigin::Insert => self.recover_from_insert_error(&error),
        }
        Ok(false)
    }

    /// After a failure the synchronous typed path returned before replaying
    /// its follow-up; command completions and inserts still replayed it.
    fn end_failed_batch(&mut self, origin: OutputOrigin, follow_up: Vec<InputEvent>) -> Result<()> {
        match output_continuation(origin, false, false) {
            OutputContinuation::Quarantine => {
                self.quarantine_evdev_input();
                Ok(())
            }
            OutputContinuation::ReplayFollowUp => self.replay_follow_up(follow_up),
            OutputContinuation::AdvanceBatch => unreachable!("failed batch cannot advance"),
        }
    }

    fn replay_follow_up(&mut self, follow_up: Vec<InputEvent>) -> Result<()> {
        if let Err(error) = replay_evdev_follow_up(
            &mut self.config.engine,
            follow_up,
            &self.policy,
            self.active_backend,
        ) {
            self.recover_from_output_error(*error)?;
        }
        self.quarantine_evdev_input();
        Ok(())
    }

    /// One reactor turn while a transaction is pending: honor stop, keep
    /// status and `explain` live, and resolve the transaction when its
    /// outcome arrives or its deadline passes.
    pub(crate) fn service_pending_output(&mut self) -> Result<Step> {
        if self
            .control
            .stop_requested
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(Step::Stop);
        }
        self.cancel_non_atomic_output_on_new_input(Duration::ZERO);
        let metrics = self.config.engine.command_metrics();
        self.publish_status(metrics);
        let Some(pending) = self.pending_output.as_ref() else {
            return Ok(Step::Next);
        };
        // Outcomes reported by the worker were counted when it applied them;
        // only the ones synthesized here still need counting.
        let (completed_result, mut outcome, trace_id) = match pending.transaction.outcome.try_recv()
        {
            Ok((result, outcome, trace_id)) => (Some(result), outcome, Some(trace_id)),
            Err(mpsc::TryRecvError::Disconnected) => {
                let outcome = wayexpand_core::TransactionOutcome::UnknownPartialFailure {
                    source: InjectorError {
                        backend: self.active_backend,
                        message: "serialized output worker stopped before acknowledging the \
                                  transaction"
                            .into(),
                        retryable: true,
                    },
                };
                output_health::note_outcome(&outcome);
                (None, outcome, None)
            }
            Err(mpsc::TryRecvError::Empty) => {
                let remaining = pending
                    .transaction
                    .deadline
                    .saturating_duration_since(Instant::now());
                if !remaining.is_zero() {
                    if self.watches_non_atomic_output_for_input() {
                        let wait = remaining.min(NON_ATOMIC_INPUT_POLL_INTERVAL);
                        self.cancel_non_atomic_output_on_new_input(wait);
                    } else {
                        let wait = remaining.min(input_loop::IDLE_MAINTENANCE_INTERVAL);
                        self.waker.wait(wait);
                    }
                    return Ok(Step::Next);
                }
                let source = self
                    .output_worker
                    .as_ref()
                    .and_then(OutputWorkerState::submitter)
                    .map(output_loop::TransactionSubmitter::abandon_after_timeout)
                    .unwrap_or_else(|| InjectorError {
                        backend: self.active_backend,
                        message: "output backend did not acknowledge the transaction in time"
                            .into(),
                        retryable: true,
                    });
                let outcome = wayexpand_core::TransactionOutcome::UnknownPartialFailure { source };
                output_health::note_outcome(&outcome);
                (None, outcome, None)
            }
        };
        // Drain once more after observing worker completion. Physical input
        // may have arrived between the reactor's initial poll and the worker
        // acknowledgement; it still invalidates an emitted-but-unverified
        // result and must not be replayed into the target.
        self.cancel_non_atomic_output_on_new_input(Duration::ZERO);
        let physical_input_interrupted = self
            .pending_output
            .as_ref()
            .is_some_and(|pending| pending.integrity.was_interrupted());
        if physical_input_interrupted {
            outcome = match outcome {
                wayexpand_core::TransactionOutcome::Applied
                | wayexpand_core::TransactionOutcome::AppliedWithCursorPositionFailure { .. } => {
                    if let Some(trace_id) = trace_id.as_deref() {
                        failure_trace::mark_partial_after_input(trace_id, self.active_backend);
                    }
                    // The output worker may have completed its last send just
                    // before the reactor observed the physical key. The key
                    // can still have landed before that final synthetic key.
                    let outcome = wayexpand_core::TransactionOutcome::UnknownPartialFailure {
                        source: InjectorError::input_interrupted(self.active_backend),
                    };
                    output_health::note_outcome(&outcome);
                    outcome
                }
                wayexpand_core::TransactionOutcome::NotApplied { .. } => {
                    // Do not restore the deferred match after new input has
                    // changed the target context, even though this output
                    // transaction itself is known not to have started.
                    let outcome = wayexpand_core::TransactionOutcome::UnknownApplicationOutcome {
                        source: InjectorError::input_interrupted(self.active_backend),
                    };
                    output_health::note_outcome(&outcome);
                    outcome
                }
                wayexpand_core::TransactionOutcome::UnknownApplicationOutcome { .. } => {
                    wayexpand_core::TransactionOutcome::UnknownApplicationOutcome {
                        source: InjectorError::input_interrupted(self.active_backend),
                    }
                }
                wayexpand_core::TransactionOutcome::UnknownPartialFailure { .. } => {
                    wayexpand_core::TransactionOutcome::UnknownPartialFailure {
                        source: InjectorError::input_interrupted(self.active_backend),
                    }
                }
            };
        }
        // The single interruption-latency sample is recorded by
        // resolve_pending_output, which takes ownership of the pending job.
        // Keep this pre-resolution path focused on outcome classification so
        // one interrupted transaction cannot be counted twice.
        self.resolve_pending_output(outcome, completed_result, trace_id)?;
        Ok(Step::Next)
    }

    fn resolve_pending_output(
        &mut self,
        outcome: wayexpand_core::TransactionOutcome,
        completed_result: Option<ExpansionResult>,
        trace_id: Option<String>,
    ) -> Result<()> {
        let Some(PendingOutput {
            origin,
            in_flight,
            queued,
            follow_up,
            mut integrity,
            interrupted_at,
            ..
        }) = self.pending_output.take()
        else {
            return Ok(());
        };
        if completed_result.is_some()
            && matches!(
                &outcome,
                wayexpand_core::TransactionOutcome::Applied
                    | wayexpand_core::TransactionOutcome::AppliedWithCursorPositionFailure { .. }
                    | wayexpand_core::TransactionOutcome::UnknownApplicationOutcome { .. }
                    | wayexpand_core::TransactionOutcome::UnknownPartialFailure { .. }
            )
        {
            if let (Some(trace_id), Some(interrupted_at)) = (trace_id.as_deref(), interrupted_at) {
                failure_trace::record_interrupt_to_backend_return(
                    trace_id,
                    interrupted_at.elapsed(),
                );
            }
        }
        let in_flight = match completed_result {
            Some(result) => in_flight.with_result(result),
            None => in_flight,
        };
        let invalidate_context = outcome_invalidates_context(&outcome);
        let invalidate_context = integrity.resolve(invalidate_context);
        // Recovery can itself fail. Apply the integrity action before
        // propagating that error, otherwise an already-applied replacement
        // with an unknown cursor could escape quarantine on the error path.
        if invalidate_context {
            // The input event has already reached the application. Do not
            // replay captured input or continue a queued batch against text
            // that may have changed while output was in progress. An
            // unacknowledged/partial backend failure has the same ambiguity
            // even when the physical-input monitor saw no new event. Reset
            // matcher context on every capture route; evdev additionally
            // needs its quiet-window quarantine.
            self.config.engine.reset_with_unknown_preceding_text();
            self.quarantine_evdev_input();
        }
        // A known pre-output refusal is retryable only while the target
        // context is still valid. Physical input, focus changes, or uncertain
        // output invalidate that context and must not resurrect a deferred
        // trigger (or retain undo state) after the matcher reset.
        let completed = self.complete_job(origin, in_flight, outcome, !invalidate_context)?;
        match output_continuation(origin, invalidate_context, completed) {
            OutputContinuation::Quarantine => {
                if !invalidate_context {
                    self.quarantine_evdev_input();
                }
                Ok(())
            }
            OutputContinuation::AdvanceBatch => self.advance_output(origin, queued, follow_up),
            OutputContinuation::ReplayFollowUp => self.replay_follow_up(follow_up),
        }
    }

    /// Non-exclusive evdev capture cannot prevent new keys from reaching the
    /// application during a slow simulated-key replacement. Detect that
    /// input while output is pending and stop the paced stream at its next
    /// backend checkpoint. The transaction remains uncertain; never replay it.
    fn watches_non_atomic_output_for_input(&self) -> bool {
        self.evdev_mode
            && self
                .injector
                .as_ref()
                .is_some_and(|injector| !injector.capabilities().has_atomic_replacement_contract())
            && self
                .pending_output
                .as_ref()
                .is_some_and(|pending| !pending.integrity.was_interrupted())
    }

    fn cancel_non_atomic_output_on_new_input(&mut self, timeout: Duration) {
        if !self.watches_non_atomic_output_for_input() {
            return;
        }

        #[cfg(test)]
        if let Some(events) = self.evdev_output_test_events.as_ref() {
            match if timeout.is_zero() {
                events.try_recv().map_err(|error| match error {
                    mpsc::TryRecvError::Empty => mpsc::RecvTimeoutError::Timeout,
                    mpsc::TryRecvError::Disconnected => mpsc::RecvTimeoutError::Disconnected,
                })
            } else {
                events.recv_timeout(timeout)
            } {
                Ok(_event) => self.interrupt_pending_non_atomic_output(
                    "test physical input arrived during non-atomic replacement",
                ),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.interrupt_pending_non_atomic_output(
                        "test physical input monitor disconnected during replacement",
                    );
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            return;
        }

        let Some(source) = self.evdev.as_mut() else {
            self.interrupt_pending_non_atomic_output(
                "evdev capture is unavailable while replacement is in progress",
            );
            return;
        };
        let input_result = source.next_event_during_output(timeout);
        status::set_evdev_input_diagnostics(source.input_diagnostics());
        match input_result {
            Ok(false) => {}
            Ok(true) => {
                self.interrupt_pending_non_atomic_output(
                    "physical input arrived during non-atomic replacement; current text may be partially changed",
                );
            }
            Err(error) => {
                self.evdev = None;
                self.schedule_input_retry();
                warn!(
                    %error,
                    event = "non_atomic_output_capture_lost",
                    "evdev capture became unavailable during non-atomic replacement; cancelling further output"
                );
                self.interrupt_pending_non_atomic_output(
                    "evdev capture was lost during non-atomic replacement",
                );
            }
        }
    }

    fn interrupt_pending_non_atomic_output(&mut self, reason: &str) {
        if let Some(pending) = self.pending_output.as_mut() {
            pending.integrity.interrupt_for_physical_input();
            pending.interrupted_at.get_or_insert_with(Instant::now);
        }
        if let Some(submitter) = self
            .output_worker
            .as_ref()
            .and_then(OutputWorkerState::submitter)
        {
            submitter.cancel_for_physical_input();
        }
        warn!(
            event = "non_atomic_output_interrupted",
            reason, "cancelling further simulated output; the transaction outcome may be partial"
        );
    }

    /// Match one evdev event and start any resulting output on the
    /// serialized worker, with the same gating as the synchronous path.
    pub(crate) fn start_evdev_output(&mut self, event: InputEvent) -> Result<()> {
        // The capture source delivered an event, so its reconnect backoff
        // starts over; output failures are recovered separately.
        self.reconnect_delay = Duration::from_millis(250);
        if matches!(
            &event,
            InputEvent::TextChar(_)
                | InputEvent::Text(_)
                | InputEvent::Key(_)
                | InputEvent::Backspace
                | InputEvent::Delimiter(_)
        ) {
            // This serialized path bypasses `process_event_with_pending_buffer`
            // so output can be submitted to its worker. Keep the same
            // content-free input accounting as that shared event path.
            crate::status::record_input_event();
        }
        if let InputEvent::Key(chord) = event {
            if let Some(undo) =
                process_key_event(&mut self.config.engine, chord, true, &self.policy)
            {
                return self.start_output(
                    OutputOrigin::Typed,
                    vec![OutputJob::Undo(undo)],
                    Vec::new(),
                );
            }
            // Ordinary physical key presses are also delivered as Key events
            // before their translated Text event. An empty output batch runs
            // the post-output quarantine and would discard that Text event,
            // preventing every typed trigger from reaching the matcher.
            return Ok(());
        }
        crate::failure_trace::note_matcher_evaluation();
        crate::latency::measure_matcher(|| {
            self.config
                .engine
                .process_deferred_into(event, &mut self.pending_results)
        });
        crate::failure_trace::note_matches(self.pending_results.len());
        dispatch_pending_results_with_pending_buffer(
            &mut self.config.engine,
            &mut self.pending_results,
            &mut self.ready_results,
            &self.policy,
            self.active_backend,
        );
        if self.ready_results.is_empty() {
            return Ok(());
        }
        let results = std::mem::take(&mut self.ready_results);
        if results.is_empty() {
            return Ok(());
        }
        let gating = apply_evdev_gating(results, &mut self.evdev, self.active_backend);
        restore_abandoned_results(&mut self.config.engine, gating.abandoned);
        self.start_output(
            OutputOrigin::Typed,
            gating
                .results
                .into_iter()
                .map(OutputJob::Expansion)
                .collect(),
            gating.follow_up,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_context_invalidation_distinguishes_focus_refusal_from_policy_refusal() {
        let focus_refusal = wayexpand_core::TransactionOutcome::NotApplied {
            source: InjectorError {
                backend: "libei",
                message: "refused: the focused window changed after the trigger was typed".into(),
                retryable: false,
            },
        };
        let policy_refusal = wayexpand_core::TransactionOutcome::NotApplied {
            source: InjectorError {
                backend: "libei",
                message: "refused: this route cannot verify the trigger at the target; compatibility_mode is off"
                    .into(),
                retryable: false,
            },
        };

        assert!(outcome_invalidates_context(&focus_refusal));
        assert!(!outcome_invalidates_context(&policy_refusal));
        assert!(outcome_invalidates_context(
            &wayexpand_core::TransactionOutcome::UnknownPartialFailure {
                source: InjectorError::input_interrupted("libei"),
            }
        ));
        assert!(!outcome_invalidates_context(
            &wayexpand_core::TransactionOutcome::Applied
        ));
    }

    #[test]
    fn focus_refusal_and_physical_input_compose_as_terminal_invalidation() {
        for physical_input_seen in [false, true] {
            for refusal_invalidates_target in [false, true] {
                let mut integrity = OutputIntegrityGate::default();
                if physical_input_seen {
                    integrity.interrupt_for_physical_input();
                }
                let outcome = wayexpand_core::TransactionOutcome::NotApplied {
                    source: InjectorError {
                        backend: "libei",
                        message: if refusal_invalidates_target {
                            "refused: the focused window changed after the trigger was typed"
                        } else {
                            "refused: this route cannot verify the trigger at the target; compatibility_mode is off"
                        }
                        .into(),
                        retryable: false,
                    },
                };

                let invalidated = integrity.resolve(outcome_invalidates_context(&outcome));
                assert_eq!(
                    invalidated,
                    physical_input_seen || refusal_invalidates_target,
                    "physical_input_seen={physical_input_seen}, refusal_invalidates_target={refusal_invalidates_target}"
                );

                // Resolution is terminal even if the physical monitor reports
                // another input after a clean policy refusal was observed.
                integrity.interrupt_for_physical_input();
                assert_eq!(
                    integrity.resolve(false),
                    invalidated,
                    "late input cannot rewrite the resolved transaction"
                );
            }
        }
    }

    #[test]
    fn output_continuation_never_advances_or_replays_an_invalidated_transaction() {
        for origin in [
            OutputOrigin::Typed,
            OutputOrigin::Command,
            OutputOrigin::Insert,
        ] {
            for invalidated in [false, true] {
                for completed in [false, true] {
                    let continuation = output_continuation(origin, invalidated, completed);
                    if invalidated {
                        assert_eq!(continuation, OutputContinuation::Quarantine);
                    } else if completed {
                        assert_eq!(continuation, OutputContinuation::AdvanceBatch);
                    } else if origin == OutputOrigin::Typed {
                        assert_eq!(continuation, OutputContinuation::Quarantine);
                    } else {
                        assert_eq!(continuation, OutputContinuation::ReplayFollowUp);
                    }

                    if invalidated {
                        assert_ne!(continuation, OutputContinuation::AdvanceBatch);
                        assert_ne!(continuation, OutputContinuation::ReplayFollowUp);
                    }
                }
            }
        }
    }

    #[test]
    fn integrity_gate_exhaustively_checks_input_and_completion_orderings() {
        #[derive(Clone, Copy)]
        enum Event {
            PhysicalInput,
            AppliedCompletion,
            AppliedCursorFailure,
            UncertainCompletion,
            FocusChangedDuringOutput,
            Resolve,
        }

        fn check_permutations(events: &mut [Event], index: usize) {
            if index == events.len() {
                let resolve_index = events
                    .iter()
                    .position(|event| matches!(event, Event::Resolve))
                    .unwrap();
                if resolve_index != events.len() - 1 {
                    return;
                }

                let mut gate = OutputIntegrityGate::default();
                let mut outcome_invalidates_context = false;
                let mut physical_input_seen = false;
                for event in events.iter().copied() {
                    match event {
                        Event::PhysicalInput => {
                            physical_input_seen = true;
                            gate.interrupt_for_physical_input();
                        }
                        Event::AppliedCompletion => {}
                        Event::AppliedCursorFailure
                        | Event::UncertainCompletion
                        | Event::FocusChangedDuringOutput => {
                            outcome_invalidates_context = true;
                        }
                        Event::Resolve => {
                            assert!(
                                gate.resolve(outcome_invalidates_context)
                                    == (physical_input_seen || outcome_invalidates_context),
                                "physical input, cursor failure, focus change, or uncertain completion quarantines"
                            );
                            gate.interrupt_for_physical_input();
                            assert_eq!(
                                gate.was_interrupted(),
                                physical_input_seen,
                                "a late event cannot mutate a resolved transaction"
                            );
                        }
                    }
                }
                return;
            }

            for swap in index..events.len() {
                events.swap(index, swap);
                check_permutations(events, index + 1);
                events.swap(index, swap);
            }
        }

        for completion in [
            Event::AppliedCompletion,
            Event::AppliedCursorFailure,
            Event::UncertainCompletion,
        ] {
            for physical_input in [false, true] {
                let mut sequence = vec![completion, Event::Resolve];
                if physical_input {
                    sequence.push(Event::PhysicalInput);
                }
                check_permutations(&mut sequence, 0);
            }
        }
        let mut focus_sequence = [Event::FocusChangedDuringOutput, Event::Resolve];
        check_permutations(&mut focus_sequence, 0);

        // An exact focus change is also covered alongside an input interrupt
        // in both orders relative to output completion.
        for input_first in [false, true] {
            let mut sequence = if input_first {
                [
                    Event::PhysicalInput,
                    Event::FocusChangedDuringOutput,
                    Event::Resolve,
                ]
            } else {
                [
                    Event::FocusChangedDuringOutput,
                    Event::PhysicalInput,
                    Event::Resolve,
                ]
            };
            check_permutations(&mut sequence, 0);
        }
    }

    #[test]
    fn integrity_gate_preserves_invariants_for_generated_event_sequences() {
        #[derive(Clone, Copy, Debug)]
        enum Event {
            PhysicalInput,
            FocusRefusal,
            PolicyRefusal,
            OutputApplied,
            OutputUncertain,
        }

        fn check(sequence: &[Event]) -> bool {
            let mut gate = OutputIntegrityGate::default();
            let mut physical_input_seen = false;
            // The transaction has one terminal backend outcome. Additional
            // terminal-looking events in a generated sequence model late or
            // duplicate notifications; they must not overwrite the first
            // result (in particular, Applied must not erase an earlier
            // uncertain result).
            let mut outcome = None;

            for event in sequence {
                match event {
                    Event::PhysicalInput => {
                        physical_input_seen = true;
                        gate.interrupt_for_physical_input();
                    }
                    Event::FocusRefusal => {
                        outcome.get_or_insert_with(|| wayexpand_core::TransactionOutcome::NotApplied {
                            source: InjectorError {
                                backend: "test",
                                message:
                                    "refused: the focused window changed after the trigger was typed"
                                        .into(),
                                retryable: false,
                            },
                        });
                    }
                    Event::PolicyRefusal => {
                        outcome.get_or_insert_with(|| wayexpand_core::TransactionOutcome::NotApplied {
                            source: InjectorError {
                                backend: "test",
                                message: "refused: this route cannot verify the trigger at the target; compatibility_mode is off"
                                    .into(),
                                retryable: false,
                            },
                        });
                    }
                    Event::OutputApplied => {
                        outcome.get_or_insert(wayexpand_core::TransactionOutcome::Applied);
                    }
                    Event::OutputUncertain => {
                        outcome.get_or_insert_with(|| {
                            wayexpand_core::TransactionOutcome::UnknownPartialFailure {
                                source: InjectorError::input_interrupted("test"),
                            }
                        });
                    }
                }
            }

            let outcome = outcome.unwrap_or(wayexpand_core::TransactionOutcome::Applied);
            let invalidating_result = outcome_invalidates_context(&outcome);
            let expected_quarantine = physical_input_seen || invalidating_result;
            let actual_quarantine = gate.resolve(invalidating_result);
            assert_eq!(
                actual_quarantine, expected_quarantine,
                "wrong integrity decision for event sequence {sequence:?}"
            );

            // Resolution is terminal: late physical input cannot change the
            // already-recorded transaction result or authorize another try.
            gate.interrupt_for_physical_input();
            assert_eq!(
                gate.was_interrupted(),
                physical_input_seen,
                "late events mutated resolved sequence {sequence:?}"
            );
            let replay_allowed = !actual_quarantine;
            assert!(
                !(physical_input_seen || invalidating_result) || !replay_allowed,
                "an interrupted, focus-stale, or uncertain transaction was replayable"
            );
            actual_quarantine
        }

        // Exhaust every ordering, including repeated input, focus and output
        // events. This deterministic property-style generator makes the
        // adversarial state space reproducible without scheduler or RNG seeds.
        fn generate(prefix: &mut Vec<Event>, remaining: usize) {
            check(prefix);
            if remaining == 0 {
                return;
            }
            for event in [
                Event::PhysicalInput,
                Event::FocusRefusal,
                Event::PolicyRefusal,
                Event::OutputApplied,
                Event::OutputUncertain,
            ] {
                prefix.push(event);
                generate(prefix, remaining - 1);
                prefix.pop();
            }
        }

        generate(&mut Vec::new(), 6);
        assert!(
            check(&[Event::OutputUncertain, Event::OutputApplied]),
            "a late Applied notification must not clear an uncertain terminal result"
        );
    }

    #[test]
    fn integrity_gate_resolution_is_immutable_and_idempotent() {
        let mut focus_invalidated = OutputIntegrityGate::default();
        assert!(focus_invalidated.resolve(true));
        assert!(focus_invalidated.resolve(false));
        focus_invalidated.interrupt_for_physical_input();
        assert!(focus_invalidated.resolve(false));
        assert!(
            !focus_invalidated.was_interrupted(),
            "late input must not be recorded as part of a resolved transaction"
        );

        let mut clean = OutputIntegrityGate::default();
        assert!(!clean.resolve(false));
        clean.interrupt_for_physical_input();
        assert!(!clean.resolve(true));
        assert!(
            !clean.was_interrupted(),
            "a late interruption must not alter the resolved outcome"
        );
    }
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    /// A non-atomic backend whose operations block until released.
    struct GatedInjector {
        gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
        calls: Arc<Mutex<Vec<String>>>,
        delivered: Arc<Mutex<String>>,
        erase_started: Arc<std::sync::atomic::AtomicUsize>,
        insert_gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
        insert_waiting: Arc<std::sync::atomic::AtomicBool>,
        fail: bool,
        cursor_failure: bool,
        capabilities: InjectorCapabilities,
        checkpoint: wayexpand_core::OutputCheckpoint,
        /// Set once an operation (already admitted) is waiting at the gate.
        waiting: Arc<std::sync::atomic::AtomicBool>,
    }

    impl GatedInjector {
        fn wait_for_gate(&self) {
            self.waiting.store(true, Ordering::Release);
            let (open, changed) = &*self.gate;
            let mut open = open.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }

        fn wait_for_insert_return(&self) {
            self.insert_waiting.store(true, Ordering::Release);
            let (open, changed) = &*self.insert_gate;
            let mut open = open.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }
    }

    impl TextInjector for GatedInjector {
        fn name(&self) -> &'static str {
            "gated-test"
        }

        fn capabilities(&self) -> InjectorCapabilities {
            self.capabilities
        }

        fn erase(&mut self, trigger: &str) -> std::result::Result<(), InjectorError> {
            self.erase_started.fetch_add(1, Ordering::AcqRel);
            self.wait_for_gate();
            self.calls.lock().unwrap().push(format!("erase:{trigger}"));
            Ok(())
        }

        fn insert(&mut self, text: &str) -> std::result::Result<(), InjectorError> {
            self.checkpoint.check().map_err(|message| InjectorError {
                backend: "gated-test",
                message,
                retryable: true,
            })?;
            self.calls.lock().unwrap().push(format!("insert:{text}"));
            if self.fail {
                self.delivered.lock().unwrap().extend(text.chars().take(2));
                return Err(InjectorError {
                    backend: "gated-test",
                    message: "portal went away".into(),
                    retryable: true,
                });
            }
            self.delivered.lock().unwrap().push_str(text);
            self.wait_for_insert_return();
            Ok(())
        }

        fn move_cursor_left(&mut self, count: usize) -> std::result::Result<(), InjectorError> {
            self.calls.lock().unwrap().push(format!("cursor:{count}"));
            if self.cursor_failure {
                return Err(InjectorError {
                    backend: "gated-test",
                    message: "cursor reposition failed".into(),
                    retryable: false,
                });
            }
            Ok(())
        }

        fn set_output_checkpoint(&mut self, checkpoint: wayexpand_core::OutputCheckpoint) {
            self.checkpoint = checkpoint;
        }
    }

    struct Harness {
        daemon: Daemon,
        gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
        calls: Arc<Mutex<Vec<String>>>,
        delivered: Arc<Mutex<String>>,
        erase_started: Arc<std::sync::atomic::AtomicUsize>,
        waiting: Arc<std::sync::atomic::AtomicBool>,
        insert_gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
        insert_waiting: Arc<std::sync::atomic::AtomicBool>,
        physical_input: mpsc::Sender<InputEvent>,
        directory: PathBuf,
    }

    impl Harness {
        fn new(fail: bool) -> Self {
            Self::build(fail, false, None, None)
        }

        fn cursor_failure() -> Self {
            Self::build(false, true, None, None)
        }

        fn inconsistent_atomic_claim() -> Self {
            Self::build(
                false,
                false,
                None,
                Some(InjectorCapabilities {
                    atomic_replace: true,
                    replacement_guarantee:
                        wayexpand_core::ReplacementGuarantee::VerifiedSurroundingText,
                    ..InjectorCapabilities::default()
                }),
            )
        }

        /// The backend behind the assurance gate, as production connects it.
        fn guarded(guard: Arc<crate::assurance::GuardState>) -> Self {
            Self::build(false, false, Some(guard), None)
        }

        fn build(
            fail: bool,
            cursor_failure: bool,
            guard: Option<Arc<crate::assurance::GuardState>>,
            capabilities: Option<InjectorCapabilities>,
        ) -> Self {
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "wayexpand-pending-output-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::set_permissions(
                &directory,
                std::os::unix::fs::PermissionsExt::from_mode(0o700),
            )
            .unwrap();
            let path = directory.join("expansions.toml");
            std::fs::write(
                &path,
                "[[expansion]]\ntrigger = \":x\"\nreplacement = \"value\"\n\
                 [[expansion]]\ntrigger = \".ssl\"\nreplacement = \"Apache rewrite configuration\"\n\
                 [[expansion]]\ntrigger = \".clarity\"\nreplacement = \"ran updates, sent welcome email, setup\"\n",
            )
            .unwrap();
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
                .unwrap();
            let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            let calls = Arc::new(Mutex::new(Vec::new()));
            let delivered = Arc::new(Mutex::new(String::new()));
            let erase_started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let waiting = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let insert_gate = Arc::new((Mutex::new(true), std::sync::Condvar::new()));
            let insert_waiting = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (physical_input, physical_input_events) = mpsc::channel();
            let waker = waker::Waker::new().unwrap();
            let backend: Box<dyn TextInjector> = Box::new(GatedInjector {
                gate: Arc::clone(&gate),
                calls: Arc::clone(&calls),
                delivered: Arc::clone(&delivered),
                erase_started: Arc::clone(&erase_started),
                insert_gate: Arc::clone(&insert_gate),
                insert_waiting: Arc::clone(&insert_waiting),
                fail,
                cursor_failure,
                capabilities: capabilities.unwrap_or(InjectorCapabilities {
                    replacement_guarantee: wayexpand_core::ReplacementGuarantee::BestEffort,
                    cursor_reposition: cursor_failure,
                    ..InjectorCapabilities::default()
                }),
                checkpoint: wayexpand_core::OutputCheckpoint::default(),
                waiting: Arc::clone(&waiting),
            });
            let backend = match guard {
                Some(guard) => crate::assurance::AssuredInjector::wrap(backend, guard),
                None => backend,
            };
            let (injector, failures, worker) =
                spawn_async_injector(backend, Some(waker.clone())).unwrap();
            let config = ReloadableConfig::load_with_policy(
                &path,
                wayexpand_core::OrganizationPolicy::default(),
            )
            .unwrap();
            let daemon = Daemon {
                control: ControlServer::detached(),
                policy: wayexpand_core::OrganizationPolicy::default(),
                path: path.clone(),
                portal_token_path: None,
                window_tracker: None,
                active_source: "evdev",
                active_backend: "libei",
                backend_name: "libei",
                managed: true,
                control_only_mode: false,
                input_method_mode: false,
                evdev_mode: true,
                config,
                explain_snapshot_cache: None,
                injector: Some(injector),
                input_method: None,
                evdev: None,
                evdev_output_test_events: Some(physical_input_events),
                receiver: None,
                stdin_input_monitor: crate::input_loop::StdinInputMonitor::default(),
                stdin_quarantine_through: 0,
                status_publisher: StatusPublisher::default(),
                focus_state: FocusState::new(None),
                profile_assignments: wayexpand_core::ProfileAssignments::default(),
                profile_switch_pending: false,
                connection_state: "connected",
                paused: false,
                reconnect_delay: Duration::from_millis(250),
                output_retry_at: None,
                output_retry_delay: Duration::from_millis(250),
                output_connect: None,
                input_connect: None,
                input_retry_at: None,
                output_failures: Some(failures),
                output_worker: Some(worker),
                retiring_output_workers: Vec::new(),
                pending_output: None,
                pending_results: Vec::new(),
                ready_results: Vec::new(),
                evdev_quiet_since: None,
                stdin_closed: false,
                logged_queue_rejections: 0,
                waker,
                usage: usage::UsageRecorder::new(directory.join("usage.json")),
            };
            Self {
                daemon,
                gate,
                calls,
                delivered,
                erase_started,
                waiting,
                insert_gate,
                insert_waiting,
                physical_input,
                directory,
            }
        }

        fn open_gate(&self) {
            let (open, changed) = &*self.gate;
            *open.lock().unwrap() = true;
            changed.notify_all();
        }

        fn open_insert_gate(&self) {
            let (open, changed) = &*self.insert_gate;
            *open.lock().unwrap() = true;
            changed.notify_all();
        }

        /// Service the pending transaction until it resolves.
        fn run_until_resolved(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.daemon.pending_output.is_some() {
                assert!(Instant::now() < deadline, "transaction never resolved");
                self.daemon.service_pending_output().unwrap();
            }
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.open_gate();
            self.open_insert_gate();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn result(trigger: &str, insert: &str) -> ExpansionResult {
        ExpansionResult {
            snippet_id: String::new().into(),
            trigger: trigger.into(),
            matched_text: trigger.into(),
            insert: insert.into(),
            cursor_offset: None,
            reinsert_after: None,
            command_backed: false,
            undoable: true,
            folded_suffix_chars: 0,
        }
    }

    #[test]
    fn pending_transaction_does_not_block_stop_or_status() {
        let mut harness = Harness::new(false);
        assert!(harness.daemon.serialized_output());
        harness
            .daemon
            .start_output(
                OutputOrigin::Typed,
                vec![OutputJob::Expansion(result(":x", "value"))],
                Vec::new(),
            )
            .unwrap();
        assert!(harness.daemon.pending_output.is_some());

        // The backend is blocked; a turn returns promptly and publishes
        // status instead of waiting for the acknowledgement.
        let started = Instant::now();
        assert!(matches!(
            harness.daemon.service_pending_output().unwrap(),
            Step::Next
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(harness
            .daemon
            .control
            .status_text()
            .contains("output_health="));

        harness
            .daemon
            .control
            .stop_requested
            .store(true, Ordering::Release);
        let started = Instant::now();
        assert!(matches!(
            harness.daemon.service_pending_output().unwrap(),
            Step::Stop
        ));
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn refused_keystroke_output_types_nothing_and_keeps_the_route_healthy() {
        let guard = Arc::new(crate::assurance::GuardState::default());
        let mut harness = Harness::guarded(Arc::clone(&guard));
        harness.open_gate();
        harness
            .daemon
            .start_output(
                OutputOrigin::Typed,
                vec![
                    OutputJob::Expansion(result(":a", "first")),
                    OutputJob::Expansion(result(":b", "second")),
                ],
                Vec::new(),
            )
            .unwrap();
        harness.run_until_resolved();

        assert!(harness.calls.lock().unwrap().is_empty(), "nothing typed");
        // As for any failed job, the rest of the batch is not output.
        assert_eq!(guard.refused_total(), 1);
        // A refusal is not a transport failure: no reconnect, no retry.
        assert!(harness.daemon.injector.is_some());
        assert!(harness
            .daemon
            .output_failures
            .as_ref()
            .is_some_and(|failures| failures.try_recv().is_err()));
        assert_eq!(harness.daemon.connection_state, "connected");
    }

    #[test]
    fn output_batch_starts_next_transaction_only_after_previous_is_acknowledged() {
        let mut harness = Harness::new(false);
        harness
            .daemon
            .start_output(
                OutputOrigin::Typed,
                vec![
                    OutputJob::Expansion(result(":a", "first")),
                    OutputJob::Expansion(result(":b", "second")),
                ],
                Vec::new(),
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while harness.erase_started.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < deadline, "first transaction never started");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(harness.erase_started.load(Ordering::Acquire), 1);
        assert!(harness.daemon.pending_output.is_some());

        // A follow-up replacement in the same batch must not reach the
        // non-atomic injector until the first operation has returned and its
        // transaction outcome has been acknowledged by the reactor.
        harness.daemon.service_pending_output().unwrap();
        assert_eq!(harness.erase_started.load(Ordering::Acquire), 1);
        harness.open_gate();
        harness.run_until_resolved();
        assert_eq!(harness.erase_started.load(Ordering::Acquire), 2);
        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase::a", "insert:first", "erase::b", "insert:second"]
        );
        assert!(harness.daemon.serialized_output(), "route stays connected");
    }

    #[test]
    fn adjacent_triggers_processed_after_output_are_each_inserted_once() {
        let mut harness = Harness::new(false);

        // Keep the first transaction blocked, then release it before delivering
        // the second event. This covers sequential back-to-back expansions;
        // it does not model physical typing during an in-flight replacement.
        harness
            .daemon
            .start_evdev_output(InputEvent::Text(".ssl".into()))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.waiting.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "first replacement never started");
            std::thread::sleep(Duration::from_millis(1));
        }
        harness.open_gate();
        harness.run_until_resolved();
        harness
            .daemon
            .start_evdev_output(InputEvent::Text(".clarity".into()))
            .unwrap();
        harness.run_until_resolved();
        harness
            .daemon
            .start_evdev_output(InputEvent::Delimiter(' '))
            .unwrap();

        assert_eq!(
            *harness.calls.lock().unwrap(),
            [
                "erase:.ssl",
                "insert:Apache rewrite configuration",
                "erase:.clarity",
                "insert:ran updates, sent welcome email, setup",
            ],
            "each queued trigger must produce exactly one output transaction"
        );
        assert!(harness.daemon.pending_output.is_none());
    }

    #[test]
    fn serialized_evdev_matching_updates_content_free_diagnostics() {
        let mut harness = Harness::new(false);
        let input_events_before = crate::status::input_events_total();
        let diagnostics_before = crate::failure_trace::totals();

        harness
            .daemon
            .start_evdev_output(InputEvent::Text(".ssl".into()))
            .unwrap();

        let diagnostics_after = crate::failure_trace::totals();
        assert!(crate::status::input_events_total() > input_events_before);
        assert!(diagnostics_after.matcher_evaluations > diagnostics_before.matcher_evaluations);
        assert!(diagnostics_after.matches_found > diagnostics_before.matches_found);

        harness.open_gate();
        harness.run_until_resolved();
    }

    #[test]
    fn ordinary_key_event_does_not_quarantine_its_following_text_event() {
        let mut harness = Harness::new(false);
        let ordinary_key = || {
            InputEvent::Key(wayexpand_core::KeyChord {
                modifiers: wayexpand_core::Modifiers::default(),
                key: "A".into(),
            })
        };

        for character in ".ssl".chars() {
            harness.daemon.start_evdev_output(ordinary_key()).unwrap();
            assert!(
                harness.daemon.evdev_quiet_since.is_none(),
                "a non-output key event must not start the post-output quiet window"
            );
            harness
                .daemon
                .start_evdev_output(InputEvent::Text(character.to_string()))
                .unwrap();
        }

        assert!(
            harness.daemon.pending_output.is_some(),
            "the translated text events must still complete the trigger"
        );
        harness.open_gate();
        harness.run_until_resolved();
        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase:.ssl", "insert:Apache rewrite configuration"]
        );
    }

    #[test]
    fn completed_serialized_output_quarantines_evdev_events_until_quiet() {
        let mut harness = Harness::new(false);
        harness.open_gate();
        harness
            .daemon
            .start_output(
                OutputOrigin::Typed,
                vec![OutputJob::Expansion(result(":slow", "replacement"))],
                Vec::new(),
            )
            .unwrap();
        harness.run_until_resolved();

        assert!(harness.daemon.evdev_quiet_since.is_some());
        let first_activity = harness.daemon.evdev_quiet_since.unwrap();
        let discarded_before = crate::counters::snapshot().evdev_quarantined_events_total;
        std::thread::sleep(Duration::from_millis(2));
        assert!(harness.daemon.discard_quarantined_evdev_input());
        assert!(harness.daemon.evdev_quiet_since.unwrap() > first_activity);
        assert_eq!(
            crate::counters::snapshot().evdev_quarantined_events_total,
            discarded_before + 1
        );
        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase::slow", "insert:replacement"],
            "a discarded captured event must not start another output transaction"
        );
    }

    #[test]
    fn observed_physical_input_cancels_output_and_drops_queued_work() {
        let mut harness = Harness::new(false);
        let uncertain_before = output_health::snapshot().uncertain_total;
        harness
            .daemon
            .start_output(
                OutputOrigin::Command,
                vec![
                    OutputJob::Expansion(result(":a", "first")),
                    OutputJob::Expansion(result(":b", "second")),
                ],
                vec![InputEvent::Text(".ssl".into()), InputEvent::Delimiter(' ')],
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.waiting.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "first transaction never started");
            std::thread::sleep(Duration::from_millis(1));
        }

        harness
            .physical_input
            .send(InputEvent::Text("user input".into()))
            .unwrap();
        harness.daemon.service_pending_output().unwrap();
        assert!(harness
            .daemon
            .pending_output
            .as_ref()
            .unwrap()
            .integrity
            .was_interrupted());
        harness.open_gate();
        harness.run_until_resolved();

        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase::a"],
            "cancellation checkpoint prevents further simulated output"
        );
        assert!(
            output_health::snapshot().uncertain_total > uncertain_before,
            "an interrupted non-atomic replacement must be surfaced as uncertain"
        );
        assert!(harness.daemon.evdev_quiet_since.is_some());
    }

    #[test]
    fn interruption_monitor_does_not_trust_an_inconsistent_atomic_claim() {
        let mut harness = Harness::inconsistent_atomic_claim();
        harness
            .daemon
            .start_output(
                OutputOrigin::Typed,
                vec![OutputJob::Expansion(result(":x", "replacement"))],
                Vec::new(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.waiting.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "replacement never reached injector"
            );
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(harness.daemon.watches_non_atomic_output_for_input());
        harness
            .physical_input
            .send(InputEvent::Key(wayexpand_core::KeyChord {
                modifiers: wayexpand_core::Modifiers::default(),
                key: "x".into(),
            }))
            .unwrap();
        harness
            .daemon
            .cancel_non_atomic_output_on_new_input(Duration::ZERO);
        assert!(harness
            .daemon
            .pending_output
            .as_ref()
            .unwrap()
            .integrity
            .was_interrupted());

        harness.open_gate();
        harness.run_until_resolved();
        assert_eq!(harness.delivered.lock().unwrap().as_str(), "");
    }

    #[test]
    fn cursor_reposition_failure_invalidates_context_and_stops_queued_output() {
        let mut harness = Harness::cursor_failure();
        let mut first = result(":first", "first");
        first.cursor_offset = Some(1);
        harness
            .daemon
            .start_output(
                OutputOrigin::Command,
                vec![
                    OutputJob::Expansion(first),
                    OutputJob::Expansion(result(":second", "second")),
                ],
                vec![InputEvent::Text("follow-up".into())],
            )
            .unwrap();

        assert!(harness.daemon.serialized_output());
        assert!(harness.daemon.pending_output.is_some());
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.waiting.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "cursor-failure transaction never started"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        harness.open_gate();
        let deadline = Instant::now() + Duration::from_secs(5);
        while harness.daemon.pending_output.is_some() {
            assert!(Instant::now() < deadline, "transaction never resolved");
            if let Err(error) = harness.daemon.service_pending_output() {
                assert!(error.to_string().contains("cursor repositioning failed"));
                break;
            }
        }
        assert!(harness.daemon.pending_output.is_none());

        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase::first", "insert:first", "cursor:1"],
            "unknown cursor position must stop queued transactions"
        );
        assert_eq!(
            harness.delivered.lock().unwrap().as_str(),
            "first",
            "known inserted text remains, but no further output is attempted"
        );
        assert!(
            harness.daemon.evdev_quiet_since.is_some(),
            "follow-up input must be quarantined after cursor-position failure"
        );
    }

    #[test]
    fn input_observed_after_worker_success_downgrades_the_commit_and_reuses_the_actor() {
        let partial_before = output_health::snapshot().partial_total;
        let mut harness = Harness::new(false);
        harness
            .daemon
            .start_output(
                OutputOrigin::Typed,
                vec![OutputJob::Expansion(result(":x", "replacement"))],
                Vec::new(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.waiting.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "transaction never started");
            std::thread::sleep(Duration::from_millis(1));
        }

        // The backend has applied the complete text, but the reactor has not
        // consumed its acknowledgement. A real key may have arrived before
        // the final synthetic key and only become visible in this turn.
        harness.open_gate();
        while harness.delivered.lock().unwrap().as_str() != "replacement" {
            assert!(
                Instant::now() < deadline,
                "backend did not finish insertion"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        harness
            .physical_input
            .send(InputEvent::Text("concurrent input".into()))
            .unwrap();
        harness.daemon.service_pending_output().unwrap();
        harness.run_until_resolved();

        assert!(
            output_health::snapshot().partial_total > partial_before,
            "a physical-input race must override the worker's provisional success"
        );
        assert!(
            harness
                .daemon
                .output_worker
                .as_ref()
                .is_some_and(OutputWorkerState::is_running),
            "transaction cancellation must not strand the serialized output actor"
        );

        harness
            .daemon
            .start_output(
                OutputOrigin::Typed,
                vec![OutputJob::Expansion(result(":next", "second"))],
                Vec::new(),
            )
            .unwrap();
        harness.run_until_resolved();
        assert_eq!(
            *harness.calls.lock().unwrap(),
            [
                "erase::x",
                "insert:replacement",
                "erase::next",
                "insert:second"
            ]
        );
        assert_eq!(
            harness.delivered.lock().unwrap().as_str(),
            "replacementsecond"
        );
    }

    #[test]
    fn missing_evdev_monitor_aborts_non_atomic_output_even_if_status_says_connected() {
        let mut harness = Harness::new(false);
        // Exercise the production fallback branch rather than the injected
        // physical-input test channel. A stale "connected" status must never
        // make a non-atomic transaction run without an input monitor.
        harness.daemon.evdev_output_test_events = None;
        let uncertain_before = output_health::snapshot().uncertain_total;
        harness
            .daemon
            .start_output(
                OutputOrigin::Command,
                vec![OutputJob::Expansion(result(":x", "replacement"))],
                Vec::new(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.waiting.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "transaction did not start");
            std::thread::sleep(Duration::from_millis(1));
        }

        harness.daemon.service_pending_output().unwrap();
        assert!(harness
            .daemon
            .pending_output
            .as_ref()
            .unwrap()
            .integrity
            .was_interrupted());
        harness.open_gate();
        harness.run_until_resolved();

        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase::x"],
            "missing input monitoring must stop output at the next checkpoint"
        );
        assert!(
            output_health::snapshot().uncertain_total > uncertain_before,
            "the unobservable non-atomic operation must be reported as uncertain"
        );
    }

    #[test]
    fn input_arriving_during_the_short_poll_cancels_before_the_batch_continues() {
        let mut harness = Harness::new(false);
        harness
            .daemon
            .start_output(
                OutputOrigin::Command,
                vec![
                    OutputJob::Expansion(result(":a", "first")),
                    OutputJob::Expansion(result(":b", "second")),
                ],
                Vec::new(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.waiting.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "first transaction never started");
            std::thread::sleep(Duration::from_millis(1));
        }

        let sender = harness.physical_input.clone();
        let input_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            sender
                .send(InputEvent::Text("concurrent key".into()))
                .unwrap();
        });
        let poll_deadline = Instant::now() + Duration::from_millis(100);
        while !harness
            .daemon
            .pending_output
            .as_ref()
            .is_some_and(|pending| pending.integrity.was_interrupted())
        {
            assert!(
                Instant::now() < poll_deadline,
                "input was not observed promptly"
            );
            harness.daemon.service_pending_output().unwrap();
        }
        input_thread.join().unwrap();
        harness.open_gate();
        harness.run_until_resolved();

        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase::a"],
            "the cancelled transaction must not continue insertion or start queued work"
        );
    }

    #[test]
    fn focus_moving_during_non_atomic_output_stops_the_stream_and_drops_the_batch() {
        use wayexpand_core::{window_update_channel, CompatibilityMode, WindowContext};
        let window = |instance: &str| {
            Some(WindowContext {
                app_id: Some("editor".into()),
                title: None,
                instance_id: Some(instance.into()),
            })
        };
        let guard = Arc::new(crate::assurance::GuardState::default());
        guard.set_mode(CompatibilityMode::TrackedFocus);
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        // The reactor last applied window w1: the trigger was typed there.
        sender.send(window("w1")).unwrap();
        guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
        let mut harness = Harness::guarded(Arc::clone(&guard));
        harness.daemon.window_tracker =
            Some(backend_lifecycle::WindowTrackerHandle::for_test(receiver));

        harness
            .daemon
            .start_output(
                OutputOrigin::Command,
                vec![
                    OutputJob::Expansion(result(":a", "first")),
                    OutputJob::Expansion(result(":b", "second")),
                ],
                Vec::new(),
            )
            .unwrap();
        // The first transaction was admitted in w1 and is now typing.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.waiting.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "first transaction never started");
            std::thread::sleep(Duration::from_millis(1));
        }

        // Focus moves to w2 while it types. Pending-output turns must not
        // apply the change: that would restamp the guard to w2, and the
        // queued transaction would then pass its focus check in w2.
        sender.send(window("w2")).unwrap();
        for _ in 0..3 {
            harness.daemon.service_pending_output().unwrap();
        }
        harness.open_gate();
        harness.run_until_resolved();

        // The first transaction is uncertain after its erase step. The
        // checkpoint prevents its insertion and the queued transaction never
        // starts, so no synthetic text is delivered into w2.
        assert_eq!(*harness.calls.lock().unwrap(), ["erase::a"]);
        assert!(harness.daemon.injector.is_none());
        assert_eq!(harness.daemon.retiring_output_workers.len(), 1);
        // The focus update is still waiting for the reactor's next turn.
        let (pending_window, _) = harness
            .daemon
            .window_tracker
            .as_ref()
            .unwrap()
            .receiver
            .try_recv_stamped()
            .expect("the focus change was left for the next reactor turn");
        assert_eq!(pending_window, window("w2"));
    }

    #[test]
    fn focus_aba_before_worker_start_discards_the_deferred_trigger() {
        use wayexpand_core::{window_update_channel, CompatibilityMode, WindowContext};
        let window = |instance: &str| {
            Some(WindowContext {
                app_id: Some("editor".into()),
                title: None,
                instance_id: Some(instance.into()),
            })
        };
        let guard = Arc::new(crate::assurance::GuardState::default());
        guard.set_mode(CompatibilityMode::TrackedFocus);
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        sender.send(window("w1")).unwrap();
        guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
        let mut harness = Harness::guarded(Arc::clone(&guard));
        harness.daemon.window_tracker =
            Some(backend_lifecycle::WindowTrackerHandle::for_test(receiver));
        process_event(
            &mut harness.daemon.config.engine,
            InputEvent::WindowChanged(window("w1")),
            None,
            &harness.daemon.policy,
            harness.daemon.active_backend,
        )
        .unwrap();
        let pending = harness
            .daemon
            .config
            .engine
            .process_deferred(InputEvent::Text(":x".into()))
            .pop()
            .expect("test trigger should be reserved in the original window");
        let result = match harness
            .daemon
            .config
            .engine
            .dispatch_pending_with_policy(pending, 0)
            .unwrap()
        {
            wayexpand_core::PendingExpansionDispatch::Ready(result) => result,
            wayexpand_core::PendingExpansionDispatch::Queued => {
                panic!("static replacement must be ready")
            }
        };

        // Focus crosses to another window and back after matching but before
        // the output worker starts. The live tracker witness advances while
        // the reactor still holds the original focus stamp.
        sender.send(window("w2")).unwrap();
        sender.send(window("w1")).unwrap();
        harness
            .daemon
            .start_output(
                OutputOrigin::Typed,
                vec![OutputJob::Expansion(result)],
                Vec::new(),
            )
            .unwrap();
        harness.run_until_resolved();

        assert!(harness.calls.lock().unwrap().is_empty());
        assert_eq!(guard.refused_total(), 1);
        assert!(
            harness
                .daemon
                .config
                .engine
                .process_deferred(InputEvent::Text(" ".into()))
                .is_empty(),
            "the old trigger must not return to the matcher after focus ABA"
        );
        assert!(harness.daemon.evdev_quiet_since.is_some());
    }

    #[test]
    fn focus_change_after_insertion_but_before_ack_refuses_the_next_batch_item() {
        use wayexpand_core::{window_update_channel, CompatibilityMode, WindowContext};
        let window = |instance: &str| {
            Some(WindowContext {
                app_id: Some("editor".into()),
                title: None,
                instance_id: Some(instance.into()),
            })
        };
        let guard = Arc::new(crate::assurance::GuardState::default());
        guard.set_mode(CompatibilityMode::TrackedFocus);
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        sender.send(window("w1")).unwrap();
        guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
        let mut harness = Harness::guarded(Arc::clone(&guard));
        harness.daemon.window_tracker =
            Some(backend_lifecycle::WindowTrackerHandle::for_test(receiver));
        *harness.insert_gate.0.lock().unwrap() = false;

        harness
            .daemon
            .start_output(
                OutputOrigin::Command,
                vec![
                    OutputJob::Expansion(result(":a", "first")),
                    OutputJob::Expansion(result(":b", "second")),
                ],
                // This trigger was already captured in the original target
                // before focus changed. Replaying it after the output ack must
                // not authorize insertion into the new focus.
                vec![InputEvent::Text(":x".into())],
            )
            .unwrap();
        harness.open_gate();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !harness.insert_waiting.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "first insertion never reached its ack gap"
            );
            std::thread::sleep(Duration::from_millis(1));
        }

        // The first replacement is fully delivered, but its worker result has
        // not yet reached the reactor. Publish a new live focus stamp without
        // draining it into the engine, as happens while pending output is
        // serviced.
        sender.send(window("w2")).unwrap();
        harness.open_insert_gate();
        harness.run_until_resolved();

        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase::a", "insert:first"],
            "the next transaction must be refused before erasing in the new window"
        );
        assert_eq!(harness.delivered.lock().unwrap().as_str(), "first");
        assert!(harness.daemon.injector.is_some());
        assert_eq!(
            guard.last_refusal(),
            Some(crate::assurance::Refusal::FocusChanged),
            "the buffered trigger must be rejected against the changed target"
        );
        let (pending_window, _) = harness
            .daemon
            .window_tracker
            .as_ref()
            .unwrap()
            .receiver
            .try_recv_stamped()
            .expect("the focus event should remain queued for the next reactor turn");
        assert_eq!(pending_window, window("w2"));
    }

    #[test]
    fn failure_stops_the_batch_and_retires_the_route() {
        let mut harness = Harness::new(true);
        harness.open_gate();
        let hotkey_marker = harness.directory.join("follow-up-hotkey-ran");
        let mut config = wayexpand_core::Config::parse(
            "[[hotkey]]\nchord = 'Ctrl+Alt+F12'\ncommand = { program = '/usr/bin/touch' }\n",
        )
        .unwrap();
        config.hotkey[0].command.args.push(
            hotkey_marker
                .to_str()
                .expect("test path is UTF-8")
                .to_owned(),
        );
        let mut engine = wayexpand_core::ExpansionEngine::new(config).unwrap();
        assert!(engine.enable_async_commands());
        harness.daemon.config.engine = engine;
        let uncertain_before = output_health::snapshot().uncertain_total;
        harness
            .daemon
            .start_output(
                OutputOrigin::Command,
                vec![
                    OutputJob::Expansion(result(":a", "first")),
                    OutputJob::Expansion(result(":b", "second")),
                ],
                vec![InputEvent::Key(
                    wayexpand_core::KeyChord::parse("Ctrl+Alt+F12").unwrap(),
                )],
            )
            .unwrap();
        harness.run_until_resolved();
        // The failed transaction is not replayed and the rest of the batch
        // is never sent to a possibly inconsistent target.
        assert_eq!(*harness.calls.lock().unwrap(), ["erase::a", "insert:first"]);
        assert_eq!(
            harness.delivered.lock().unwrap().as_str(),
            "fi",
            "the fake backend applies a prefix before reporting the lost connection"
        );
        assert!(harness.daemon.injector.is_none());
        assert_eq!(harness.daemon.retiring_output_workers.len(), 1);
        assert!(harness.daemon.output_retry_at.is_some());
        assert!(
            output_health::snapshot().uncertain_total > uncertain_before,
            "failure after trigger deletion must be recorded as an uncertain partial result"
        );
        let command_metrics = harness.daemon.config.engine.command_metrics();
        assert_eq!(command_metrics.hotkey_queue_depth, 0);
        assert_eq!(command_metrics.hotkey_in_flight, 0);
        assert!(
            !hotkey_marker.exists(),
            "follow-up input after uncertain insertion must be quarantined, not dispatched"
        );
    }

    #[test]
    fn unacknowledged_transaction_times_out_as_uncertain() {
        let mut harness = Harness::new(false);
        harness
            .daemon
            .start_output(
                OutputOrigin::Insert,
                vec![OutputJob::Expansion(result(":x", "value"))],
                Vec::new(),
            )
            .unwrap();
        let before = output_health::snapshot().uncertain_total;
        harness
            .daemon
            .pending_output
            .as_mut()
            .unwrap()
            .transaction
            .deadline = Instant::now();
        harness.daemon.service_pending_output().unwrap();
        assert!(harness.daemon.pending_output.is_none());
        // An insert failure is never fatal; the stalled worker is retired
        // and must stop before output reconnects.
        assert!(harness.daemon.injector.is_none());
        assert_eq!(harness.daemon.retiring_output_workers.len(), 1);
        assert!(harness.daemon.retiring_output_workers[0]
            .submitter()
            .is_none());
        assert!(output_health::snapshot().uncertain_total > before);
        assert!(harness.calls.lock().unwrap().is_empty());

        // Once the backend call returns, the retired worker must stop; a
        // leftover command sender would keep it waiting forever and block
        // output reconnection.
        harness.open_gate();
        let deadline = Instant::now() + Duration::from_secs(2);
        while harness.daemon.retiring_output_workers[0].is_running() {
            assert!(Instant::now() < deadline, "retired worker never stopped");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn output_reconnect_runs_off_the_reactor_and_is_collected_later() {
        let mut harness = Harness::new(false);
        harness.daemon.retire_output_injector();
        let deadline = Instant::now() + Duration::from_secs(2);
        while harness
            .daemon
            .retiring_output_workers
            .iter()
            .any(OutputWorkerState::is_running)
        {
            assert!(Instant::now() < deadline, "retired worker never stopped");
            std::thread::sleep(Duration::from_millis(10));
        }
        // An unknown backend fails permanently, which stands in for a portal
        // that times out: the turn that starts it must return at once.
        harness.daemon.backend_name = "unknown";
        harness.daemon.output_retry_at = Some(Instant::now());
        let started = Instant::now();
        while harness.daemon.output_connect.is_none() {
            harness.daemon.recover_output().unwrap();
            assert!(started.elapsed() < Duration::from_secs(2));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let error = loop {
            match harness.daemon.recover_output() {
                Ok(()) => {
                    assert!(Instant::now() < deadline, "reconnect result never arrived");
                    harness.daemon.waker.wait(Duration::from_millis(50));
                }
                Err(error) => break error,
            }
        };
        assert!(error.to_string().contains("unknown output backend"));
        assert!(harness.daemon.output_connect.is_none());
    }
}
