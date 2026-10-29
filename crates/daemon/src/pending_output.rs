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
    dispatch_pending_results, expansion_allowed, finish_expansion, finish_undo, process_key_event,
};
use crate::output_loop::SubmittedTransaction;
use crate::source_steps::Step;
use crate::*;

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

pub(crate) enum OutputJob {
    Expansion(ExpansionResult),
    Undo(ExpansionResult),
}

impl OutputJob {
    fn result(&self) -> &ExpansionResult {
        match self {
            Self::Expansion(result) | Self::Undo(result) => result,
        }
    }
}

pub(crate) struct PendingOutput {
    origin: OutputOrigin,
    in_flight: OutputJob,
    transaction: SubmittedTransaction,
    queued: VecDeque<OutputJob>,
    follow_up: Vec<InputEvent>,
    physical_input_interrupted: bool,
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
            let submitted = match self
                .output_worker
                .as_ref()
                .and_then(OutputWorkerState::submitter)
            {
                Some(submitter) => submitter.submit(job.result()),
                None => Err(InjectorError {
                    backend: self.active_backend,
                    message: "serialized output worker is no longer available".into(),
                    retryable: true,
                }),
            };
            match submitted {
                Ok(transaction) => {
                    self.pending_output = Some(PendingOutput {
                        origin,
                        in_flight: job,
                        transaction,
                        queued,
                        follow_up,
                        physical_input_interrupted: false,
                    });
                    return Ok(());
                }
                // The worker never received the transaction, so nothing was
                // typed: report it as not applied.
                Err(source) => {
                    let outcome = wayexpand_core::TransactionOutcome::NotApplied { source };
                    if !self.complete_job(origin, job, outcome)? {
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
    ) -> Result<bool> {
        let finished = match job {
            OutputJob::Expansion(result) => {
                finish_expansion(&mut self.config.engine, result, outcome)
            }
            OutputJob::Undo(result) => finish_undo(&mut self.config.engine, result, outcome),
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
        if origin == OutputOrigin::Typed {
            self.quarantine_evdev_input();
            return Ok(());
        }
        self.replay_follow_up(follow_up)
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
        self.answer_explain_request();
        self.cancel_non_atomic_output_on_physical_input(Duration::ZERO);
        let metrics = self.config.engine.command_metrics();
        self.publish_status(metrics);
        let Some(pending) = self.pending_output.as_ref() else {
            return Ok(Step::Next);
        };
        // Outcomes reported by the worker were counted when it applied them;
        // only the ones synthesized here still need counting.
        let outcome = match pending.transaction.outcome.try_recv() {
            Ok(outcome) => outcome,
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
                outcome
            }
            Err(mpsc::TryRecvError::Empty) => {
                let remaining = pending
                    .transaction
                    .deadline
                    .saturating_duration_since(Instant::now());
                if !remaining.is_zero() {
                    let wait = remaining.min(input_loop::IDLE_MAINTENANCE_INTERVAL);
                    if self.watches_non_atomic_output_for_input() {
                        self.cancel_non_atomic_output_on_physical_input(wait);
                    } else {
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
                outcome
            }
        };
        self.resolve_pending_output(outcome)?;
        Ok(Step::Next)
    }

    fn resolve_pending_output(
        &mut self,
        outcome: wayexpand_core::TransactionOutcome,
    ) -> Result<()> {
        let Some(PendingOutput {
            origin,
            in_flight,
            queued,
            follow_up,
            physical_input_interrupted,
            ..
        }) = self.pending_output.take()
        else {
            return Ok(());
        };
        let completed = self.complete_job(origin, in_flight, outcome)?;
        if physical_input_interrupted {
            // The input event has already reached the application. Do not
            // replay captured input or continue a queued batch against text
            // that may have changed while output was in progress.
            self.quarantine_evdev_input();
            Ok(())
        } else if completed {
            self.advance_output(origin, queued, follow_up)
        } else {
            self.end_failed_batch(origin, follow_up)
        }
    }

    /// Non-exclusive evdev capture cannot prevent new keys from reaching the
    /// application during a slow simulated-key replacement. Detect that
    /// input while output is pending and stop the paced stream at its next
    /// backend checkpoint. The transaction remains uncertain; never replay it.
    fn watches_non_atomic_output_for_input(&self) -> bool {
        #[cfg(test)]
        let test_event_pending = self.evdev_output_test_event.is_some();
        #[cfg(not(test))]
        let test_event_pending = false;

        self.evdev_mode
            && (self.evdev.is_some() || self.connection_state != "connected" || test_event_pending)
            && self
                .injector
                .as_ref()
                .is_some_and(|injector| !injector.capabilities().atomic_replace)
            && self
                .pending_output
                .as_ref()
                .is_some_and(|pending| !pending.physical_input_interrupted)
    }

    fn cancel_non_atomic_output_on_physical_input(&mut self, timeout: Duration) {
        if !self.watches_non_atomic_output_for_input() {
            return;
        }

        #[cfg(test)]
        if self.evdev_output_test_event.take().is_some() {
            self.interrupt_pending_non_atomic_output(
                "test physical input arrived during non-atomic replacement",
            );
            return;
        }

        let Some(source) = self.evdev.as_mut() else {
            self.interrupt_pending_non_atomic_output(
                "evdev capture is unavailable while replacement is in progress",
            );
            return;
        };
        match source.next_event_during_output(timeout) {
            Ok(None) => {}
            Ok(Some(_event)) => {
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
            pending.physical_input_interrupted = true;
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
        if let InputEvent::Key(chord) = event {
            let undo = process_key_event(&mut self.config.engine, chord, true, &self.policy);
            return self.start_output(
                OutputOrigin::Typed,
                undo.map(OutputJob::Undo).into_iter().collect(),
                Vec::new(),
            );
        }
        let pending = self.config.engine.process_deferred(event);
        let results = dispatch_pending_results(
            &mut self.config.engine,
            pending,
            &self.policy,
            self.active_backend,
        );
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
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    /// A non-atomic backend whose operations block until released.
    struct GatedInjector {
        gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
        calls: Arc<Mutex<Vec<String>>>,
        erase_started: Arc<std::sync::atomic::AtomicUsize>,
        fail: bool,
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
    }

    impl TextInjector for GatedInjector {
        fn name(&self) -> &'static str {
            "gated-test"
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
                return Err(InjectorError {
                    backend: "gated-test",
                    message: "portal went away".into(),
                    retryable: true,
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
        erase_started: Arc<std::sync::atomic::AtomicUsize>,
        waiting: Arc<std::sync::atomic::AtomicBool>,
        directory: PathBuf,
    }

    impl Harness {
        fn new(fail: bool) -> Self {
            Self::build(fail, None)
        }

        /// The backend behind the assurance gate, as production connects it.
        fn guarded(guard: Arc<crate::assurance::GuardState>) -> Self {
            Self::build(false, Some(guard))
        }

        fn build(fail: bool, guard: Option<Arc<crate::assurance::GuardState>>) -> Self {
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
            let erase_started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let waiting = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let waker = waker::Waker::new().unwrap();
            let backend: Box<dyn TextInjector> = Box::new(GatedInjector {
                gate: Arc::clone(&gate),
                calls: Arc::clone(&calls),
                erase_started: Arc::clone(&erase_started),
                fail,
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
                input_method_mode: false,
                evdev_mode: true,
                config,
                injector: Some(injector),
                input_method: None,
                evdev: None,
                evdev_output_test_event: None,
                receiver: None,
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
                erase_started,
                waiting,
                directory,
            }
        }

        fn open_gate(&self) {
            let (open, changed) = &*self.gate;
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
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn result(trigger: &str, insert: &str) -> ExpansionResult {
        ExpansionResult {
            snippet_id: String::new(),
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

        harness.daemon.evdev_output_test_event = Some(InputEvent::Text("user input".into()));
        harness.daemon.service_pending_output().unwrap();
        assert!(
            harness
                .daemon
                .pending_output
                .as_ref()
                .unwrap()
                .physical_input_interrupted
        );
        harness.open_gate();
        harness.run_until_resolved();

        assert_eq!(
            *harness.calls.lock().unwrap(),
            ["erase::a"],
            "cancellation checkpoint prevents further simulated output"
        );
        assert!(harness.daemon.evdev_quiet_since.is_some());
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
    fn failure_stops_the_batch_and_retires_the_route() {
        let mut harness = Harness::new(true);
        harness.open_gate();
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
        harness.run_until_resolved();
        // The failed transaction is not replayed and the rest of the batch
        // is never sent to a possibly inconsistent target.
        assert_eq!(*harness.calls.lock().unwrap(), ["erase::a", "insert:first"]);
        assert!(harness.daemon.injector.is_none());
        assert_eq!(harness.daemon.retiring_output_workers.len(), 1);
        assert!(harness.daemon.output_retry_at.is_some());
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
