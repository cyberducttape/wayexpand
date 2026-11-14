//! Small, content-free history of expansion output transactions.
//!
//! The trace records only transaction stages and the negotiated backend. It
//! never stores triggers, replacement text, key events, or backend errors,
//! since those strings may contain user content.

#[cfg(test)]
use std::sync::MutexGuard;
use std::{
    cell::Cell,
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock, TryLockError,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use wayexpand_core::{
    InjectorCapabilities, InjectorError, KeyEventState, Modifiers, OutputCheckpoint, TextInjector,
    TransactionOutcome, TransactionPhase,
};

const HISTORY_LIMIT: usize = 8;
const FAILURE_STATUS_LIMIT: usize = 3;
const OUTCOME_STATUS_LIMIT: usize = 4;
static SUPPRESSED_EXPANSIONS_TOTAL: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
static MATCHER_EVALUATIONS: AtomicU64 = AtomicU64::new(0);
static MATCHES_FOUND: AtomicU64 = AtomicU64::new(0);
static EXPANSION_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static OUTPUT_ACKNOWLEDGEMENTS: AtomicU64 = AtomicU64::new(0);
static EXPANSIONS_REJECTED: AtomicU64 = AtomicU64::new(0);
static DIAGNOSTIC_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0);
static LAST_OUTPUT_ACKNOWLEDGED_UNIX: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static TEST_HISTORY_GATE: Mutex<()> = Mutex::new(());
#[cfg(test)]
std::thread_local! {
    static TEST_HISTORY_GATE_HELD: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) struct HistoryTestGuard {
    _lock: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for HistoryTestGuard {
    fn drop(&mut self) {
        TEST_HISTORY_GATE_HELD.with(|held| held.set(false));
    }
}

/// Keep process-global diagnostic history assertions isolated from parallel
/// tests that exercise unrelated expansion paths. Production history remains
/// bounded and best-effort; this gate exists only in test builds.
#[cfg(test)]
pub(crate) fn history_test_guard() -> HistoryTestGuard {
    let lock = TEST_HISTORY_GATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    TEST_HISTORY_GATE_HELD.with(|held| held.set(true));
    HistoryTestGuard { _lock: lock }
}

#[cfg(test)]
fn history_test_gate() -> Option<MutexGuard<'static, ()>> {
    if TEST_HISTORY_GATE_HELD.with(Cell::get) {
        None
    } else {
        Some(
            TEST_HISTORY_GATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

#[cfg(not(test))]
fn history_test_gate() -> Option<()> {
    None
}

pub(crate) fn last_output_acknowledged_unix() -> u64 {
    LAST_OUTPUT_ACKNOWLEDGED_UNIX.load(Ordering::Relaxed)
}

pub(crate) fn note_output_acknowledged() {
    LAST_OUTPUT_ACKNOWLEDGED_UNIX.store(now_unix(), Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DiagnosticTotals {
    pub matcher_evaluations: u64,
    pub matches_found: u64,
    pub expansion_attempts: u64,
    pub output_acknowledgements: u64,
    pub expansions_rejected: u64,
    pub events_dropped: u64,
}

pub(crate) fn totals() -> DiagnosticTotals {
    DiagnosticTotals {
        matcher_evaluations: MATCHER_EVALUATIONS.load(Ordering::Relaxed),
        matches_found: MATCHES_FOUND.load(Ordering::Relaxed),
        expansion_attempts: EXPANSION_ATTEMPTS.load(Ordering::Relaxed),
        output_acknowledgements: OUTPUT_ACKNOWLEDGEMENTS.load(Ordering::Relaxed),
        expansions_rejected: EXPANSIONS_REJECTED.load(Ordering::Relaxed),
        events_dropped: DIAGNOSTIC_EVENTS_DROPPED.load(Ordering::Relaxed),
    }
}

pub(crate) fn note_matcher_evaluation() {
    MATCHER_EVALUATIONS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_matches(count: usize) {
    MATCHES_FOUND.fetch_add(count as u64, Ordering::Relaxed);
}

pub(crate) fn expansion_attempts() -> u64 {
    EXPANSION_ATTEMPTS.load(Ordering::Relaxed)
}

pub(crate) fn note_expansion_attempt() {
    EXPANSION_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_expansion_outcome(applied: bool) {
    if applied {
        OUTPUT_ACKNOWLEDGEMENTS.fetch_add(1, Ordering::Relaxed);
    } else {
        EXPANSIONS_REJECTED.fetch_add(1, Ordering::Relaxed);
    }
}

/// Number of matched expansions denied by a policy, safety gate, or
/// preparation failure before reaching the output backend.
pub(crate) fn suppressed_expansions_total() -> u64 {
    SUPPRESSED_EXPANSIONS_TOTAL.load(Ordering::Relaxed)
}

#[derive(Clone, Debug)]
pub(crate) struct FailureTrace {
    transaction_id: String,
    at_unix: u64,
    duration_us: Option<u64>,
    /// Time from observing overlapping input until the output backend's
    /// transaction call returned. This is not an application paint ack.
    interrupt_to_backend_return_us: Option<u64>,
    backend: String,
    expected_insert_units: Option<usize>,
    trigger_recognized: &'static str,
    focus_admission: &'static str,
    backend_selected: &'static str,
    erase_acknowledged: &'static str,
    insertion_attempted: bool,
    transaction_phases: Vec<&'static str>,
    completion: &'static str,
    progress_tracked: bool,
    erased_units: usize,
    inserted_units: usize,
}

#[derive(Default)]
struct Stages {
    backend: String,
    admission: Cell<&'static str>,
    erase: &'static str,
    insertion_attempted: bool,
}

pub(crate) struct TracedInjector<'a> {
    inner: &'a mut dyn TextInjector,
    stages: Stages,
    started: Instant,
    expected_insert_units: usize,
}

impl<'a> TracedInjector<'a> {
    pub(crate) fn new(inner: &'a mut dyn TextInjector, expected_insert_units: usize) -> Self {
        Self {
            stages: Stages {
                backend: inner.name().to_owned(),
                admission: Cell::new("not reached"),
                ..Stages::default()
            },
            inner,
            started: Instant::now(),
            expected_insert_units,
        }
    }

    pub(crate) fn record_outcome(
        self,
        outcome: &TransactionOutcome,
        phases: &[TransactionPhase],
        progress: wayexpand_core::OutputProgress,
    ) -> String {
        let completion = match outcome {
            TransactionOutcome::Applied => "applied",
            TransactionOutcome::AppliedWithCursorPositionFailure { .. } => {
                "inserted; cursor placement failed"
            }
            TransactionOutcome::NotApplied { .. } => "not applied",
            TransactionOutcome::UnknownApplicationOutcome { .. } => "application outcome unknown",
            TransactionOutcome::UnknownPartialFailure { .. } => "partial_write",
        };
        let erase_acknowledged = if self.stages.erase.is_empty() {
            if self.inner.capabilities().has_atomic_replacement_contract() {
                "atomic replacement not acknowledged"
            } else {
                "substep not observable"
            }
        } else {
            self.stages.erase
        };
        let transaction_phases = phases.iter().map(|phase| phase.as_str()).collect();
        let transaction_id = next_transaction_id();
        let duration_us = self.started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        push(FailureTrace {
            transaction_id: transaction_id.clone(),
            at_unix: now_unix(),
            duration_us: Some(duration_us),
            interrupt_to_backend_return_us: None,
            backend: self.stages.backend,
            expected_insert_units: Some(self.expected_insert_units),
            trigger_recognized: "yes",
            focus_admission: self.stages.admission.get(),
            backend_selected: "yes",
            erase_acknowledged,
            insertion_attempted: self.stages.insertion_attempted,
            transaction_phases,
            completion,
            progress_tracked: progress.tracked,
            erased_units: progress.erased_units,
            inserted_units: progress.inserted_units,
        });
        transaction_id
    }
}

/// Correct a successful worker-side trace when the reactor observes physical
/// input before it accepts the transaction result. The user's input may have
/// interleaved with the last generated characters, so the target outcome is
/// uncertain even if the backend finished its send loop.
pub(crate) fn mark_partial_after_input(transaction_id: &str, backend: &str) -> bool {
    let _test_gate = history_test_gate();
    let mut history = match history().try_lock() {
        Ok(history) => history,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        // This is called only after the unusual physical-input interruption
        // path. Losing the correction would leave the correlated outcome
        // reporting `applied` even though the reactor correctly downgraded
        // the transaction to partial. The history is strictly bounded and
        // its readers hold the lock only while serializing a few records, so
        // wait here to preserve the safety diagnostic.
        Err(TryLockError::WouldBlock) => history()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    };
    let Some(trace) = history.outcomes.iter_mut().find(|trace| {
        trace.transaction_id == transaction_id
            && matches!(
                trace.completion,
                "applied" | "inserted; cursor placement failed"
            )
    }) else {
        // The worker's first trace write is deliberately nonblocking. If it
        // was dropped under contention (or evicted before the reactor
        // consumed its acknowledgement), preserve the safety-critical result
        // with a minimal record. Do not imply that the erase or insertion was
        // acknowledged when the detailed progress record is unavailable.
        let backend = backend
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || "-_.+".contains(*character))
            .take(64)
            .collect::<String>();
        let trace = FailureTrace {
            transaction_id: transaction_id.to_owned(),
            at_unix: now_unix(),
            duration_us: None,
            interrupt_to_backend_return_us: None,
            backend,
            expected_insert_units: None,
            trigger_recognized: "yes",
            focus_admission: "not recorded",
            backend_selected: "yes",
            erase_acknowledged: "unknown",
            insertion_attempted: true,
            transaction_phases: vec!["uncertain"],
            completion: "partial_write",
            progress_tracked: false,
            erased_units: 0,
            inserted_units: 0,
        };
        push_bounded(&mut history.failures, HISTORY_LIMIT, trace.clone());
        push_bounded(&mut history.outcomes, OUTCOME_STATUS_LIMIT, trace);
        return false;
    };
    trace.completion = "partial_write";
    if let Some(last_phase) = trace.transaction_phases.last_mut() {
        if *last_phase == "committed" {
            *last_phase = "uncertain";
        }
    }
    let failed_trace = trace.clone();
    push_bounded(&mut history.failures, HISTORY_LIMIT, failed_trace);
    true
}

/// Add content-free timing for an interrupted output whose backend call has
/// now returned. This measures from daemon observation to backend return; it
/// does not claim that the compositor or target application finished painting.
pub(crate) fn record_interrupt_to_backend_return(
    transaction_id: &str,
    elapsed: std::time::Duration,
) {
    let elapsed_us = elapsed.as_micros().min(u64::MAX as u128) as u64;
    crate::latency::record_interrupt_to_backend_return(elapsed);
    // The aggregate metric above is the durable diagnostic signal. Updating
    // this per-transaction ring is best-effort: status readers must never
    // stall the input reactor while it resolves an interrupted output.
    let mut history = match history().try_lock() {
        Ok(history) => history,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    for trace in &mut history.outcomes {
        if trace.transaction_id == transaction_id && trace.completion == "partial_write" {
            trace.interrupt_to_backend_return_us = Some(elapsed_us);
        }
    }
    for trace in &mut history.failures {
        if trace.transaction_id == transaction_id && trace.completion == "partial_write" {
            trace.interrupt_to_backend_return_us = Some(elapsed_us);
        }
    }
}

/// Record a matched expansion stopped before output. Only callers supply
/// fixed reason labels; no trigger, replacement, policy text, or user input is
/// retained. Backend names are bounded and stripped to a small identifier set
/// because they are serialized into the daemon status response.
pub(crate) fn record_blocked_expansion(backend: &str, reason: &'static str) {
    SUPPRESSED_EXPANSIONS_TOTAL.fetch_add(1, Ordering::Relaxed);
    let backend = backend
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "-_.+".contains(*character))
        .take(64)
        .collect::<String>();
    push(FailureTrace {
        transaction_id: next_transaction_id(),
        at_unix: now_unix(),
        duration_us: None,
        interrupt_to_backend_return_us: None,
        backend,
        expected_insert_units: None,
        trigger_recognized: "yes",
        focus_admission: "not reached",
        backend_selected: "not reached",
        erase_acknowledged: "not attempted",
        insertion_attempted: false,
        transaction_phases: vec!["prepared", "failed"],
        completion: reason,
        progress_tracked: false,
        erased_units: 0,
        inserted_units: 0,
    });
}

pub(crate) fn record_command_failure(backend: &str, kind: wayexpand_core::CommandErrorKind) {
    record_blocked_expansion(backend, command_failure_reason(kind));
}

fn command_failure_reason(kind: wayexpand_core::CommandErrorKind) -> &'static str {
    use wayexpand_core::CommandErrorKind as Kind;
    match kind {
        Kind::Timeout => "command timed out before output",
        Kind::BrokerUnavailable => "action broker unavailable",
        Kind::OutputTooLarge => "command output exceeded policy limit",
        Kind::Policy => "command blocked by policy",
        Kind::UnsafeContext => "command context was no longer safe",
        Kind::StaleInput => "command result became stale",
        Kind::WindowIdentity => "command focus could not be verified",
        Kind::FormTargetUnverifiable => "form requires verified replacement",
        Kind::QueueFull | Kind::WorkerUnavailable => "command worker queue unavailable",
        Kind::Spawn
        | Kind::ProcessWait
        | Kind::BrokerProtocol
        | Kind::BrokerRejected
        | Kind::NonZeroExit
        | Kind::InvalidOutput
        | Kind::OutputChannel
        | Kind::IncompleteOutput => "command execution failed before output",
        Kind::InvalidTemplate => "form values could not render the template",
        Kind::InvalidFormOutput => "form submission values are invalid or incomplete",
        Kind::IncompleteFormOutput => "form helper output stream did not close",
    }
}

/// Preserve actionable, content-free reasons when a matched static snippet
/// cannot be prepared. In particular, forms require stronger replacement
/// guarantees than ordinary static insertions.
pub(crate) fn record_preparation_failure(backend: &str, error: &wayexpand_core::CommandError) {
    record_blocked_expansion(backend, preparation_failure_reason(error));
}

fn preparation_failure_reason(error: &wayexpand_core::CommandError) -> &'static str {
    use wayexpand_core::CommandError as Error;
    match error {
        Error::SpawnFailed { .. } => "helper could not start",
        Error::Timeout => "helper timed out",
        Error::WaitFailed { .. } => "helper process supervision failed",
        Error::BrokerUnavailable { .. } => "action broker unavailable",
        Error::BrokerProtocol { .. } => "action broker protocol failed",
        Error::BrokerRejected(_) => "action broker rejected the request",
        Error::NonZeroExit { .. } => "helper exited unsuccessfully",
        Error::OutputTooLarge | Error::PolicyOutputTooLarge { .. } => {
            "helper output exceeded the configured limit"
        }
        Error::FormTargetUnverifiable => "form requires verified replacement",
        Error::WindowIdentityUnavailable => "form requires exact input-target identity",
        Error::PolicyBlocked => "blocked by organization policy",
        Error::StaleInput => "input changed before expansion was prepared",
        Error::QueueFull | Error::WorkerUnavailable => "form worker unavailable",
        Error::InvalidUtf8 => "helper output was not valid UTF-8",
        Error::TemplateRenderFailed => "form values could not render the template",
        Error::InvalidFormOutput => "form submission values are invalid or incomplete",
        Error::IncompleteFormOutput => "form helper output stream did not close",
        Error::OutputChannelLost => "helper output channel was lost",
        Error::IncompleteOutput => "helper output was incomplete",
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Opaque process-local correlation key. It is independent of user text and
/// is included only in the bounded output-transaction history.
fn next_transaction_id() -> String {
    let sequence = TRANSACTION_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{:x}", std::process::id(), sequence)
}

impl TextInjector for TracedInjector<'_> {
    fn shutdown(self: Box<Self>) {}
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn capabilities(&self) -> InjectorCapabilities {
        self.inner.capabilities()
    }
    fn uses_paced_synthetic_output(&self) -> bool {
        self.inner.uses_paced_synthetic_output()
    }
    fn status_detail(&self) -> &'static str {
        self.inner.status_detail()
    }
    fn replace_error_may_have_applied(&self) -> bool {
        self.inner.replace_error_may_have_applied()
    }
    fn admit(&self, erases: bool) -> Result<(), InjectorError> {
        match self.inner.admit(erases) {
            Ok(()) => {
                // This stage means the backend's admission contract accepted
                // the target. Routes without focus verification say so in
                // their capability dashboard; the trace does not infer it.
                self.stages.admission.set("passed");
                Ok(())
            }
            Err(error) => {
                self.stages.admission.set("refused");
                Err(error)
            }
        }
    }
    fn preflight_text(&mut self, text: &str) -> Result<(), InjectorError> {
        self.inner.preflight_text(text)
    }
    fn set_output_checkpoint(&mut self, checkpoint: OutputCheckpoint) {
        self.inner.set_output_checkpoint(checkpoint)
    }
    fn reset_output_progress(&mut self) {
        self.inner.reset_output_progress()
    }
    fn output_progress(&self) -> wayexpand_core::OutputProgress {
        self.inner.output_progress()
    }
    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        match self.inner.erase(trigger) {
            Ok(()) => {
                self.stages.erase = "yes";
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.stages.insertion_attempted = true;
        self.inner.insert(text)
    }
    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        self.stages.insertion_attempted = true;
        let atomic = self.inner.capabilities().has_atomic_replacement_contract();
        match self.inner.replace(trigger, text) {
            Ok(()) => {
                self.stages.erase = if atomic {
                    "atomic replacement acknowledged"
                } else {
                    "transaction acknowledged; substeps not exposed"
                };
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
    fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        self.inner.move_cursor_left(count)
    }
    fn inject_key(&mut self, keycode: u32) -> Result<(), InjectorError> {
        self.inner.inject_key(keycode)
    }
    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        self.inner.inject_key_with_modifiers(keycode, modifiers)
    }
    fn inject_key_event(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        self.inner.inject_key_event(keycode, modifiers, state)
    }
}

/// Failures and all outcomes are kept in separate rings so a run of
/// successful expansions cannot evict the failures diagnostics report.
#[derive(Default)]
struct History {
    failures: VecDeque<FailureTrace>,
    outcomes: VecDeque<FailureTrace>,
}

fn history() -> &'static Mutex<History> {
    static HISTORY: OnceLock<Mutex<History>> = OnceLock::new();
    HISTORY.get_or_init(|| Mutex::new(History::default()))
}

fn push_bounded(ring: &mut VecDeque<FailureTrace>, limit: usize, trace: FailureTrace) {
    if ring.len() == limit {
        ring.pop_front();
    }
    ring.push_back(trace);
}

fn push(trace: FailureTrace) {
    let _test_gate = history_test_gate();
    match history().try_lock() {
        Ok(mut history) => push_history(&mut history, trace),
        Err(TryLockError::Poisoned(poisoned)) => push_history(&mut poisoned.into_inner(), trace),
        Err(TryLockError::WouldBlock) => {
            DIAGNOSTIC_EVENTS_DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn push_history(history: &mut History, trace: FailureTrace) {
    if trace.completion != "applied" {
        push_bounded(&mut history.failures, HISTORY_LIMIT, trace.clone());
    }
    push_bounded(&mut history.outcomes, OUTCOME_STATUS_LIMIT, trace);
}

pub(crate) fn recent_json() -> String {
    let _test_gate = history_test_gate();
    let history = history()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let skip = history.failures.len().saturating_sub(FAILURE_STATUS_LIMIT);
    traces_json(history.failures.iter().skip(skip))
}

/// Recent successful and blocked output transactions. Each record contains
/// only stage outcomes and backend identity, never user text.
pub(crate) fn recent_outcomes_json() -> String {
    let _test_gate = history_test_gate();
    let history = history()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    traces_json(history.outcomes.iter())
}

fn traces_json<'a>(traces: impl Iterator<Item = &'a FailureTrace>) -> String {
    let traces = traces
        .map(|trace| {
            let application_result =
                application_result(trace.completion, trace.insertion_attempted);
            serde_json::json!({
                "transaction_id": trace.transaction_id,
                "at_unix": trace.at_unix,
                "duration_us": trace.duration_us,
                "interrupt_to_backend_return_us": trace.interrupt_to_backend_return_us,
                "backend": trace.backend,
                "expected_insert_units": trace.expected_insert_units,
                "trigger_recognized": trace.trigger_recognized,
                "focus_admission": trace.focus_admission,
                "backend_selected": trace.backend_selected,
                "erase_acknowledged": trace.erase_acknowledged,
                "insertion_attempted": trace.insertion_attempted,
                "transaction_phases": trace.transaction_phases,
                "completion": trace.completion,
                "application_result": application_result,
                "recovery": if trace.completion == "partial_write" {
                    "manual_review"
                } else {
                    "not_applicable"
                },
                "progress_tracked": trace.progress_tracked,
                "erased_units": trace.erased_units,
                "inserted_units": trace.inserted_units,
            })
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(traces).to_string()
}

/// The daemon observes backend/protocol acknowledgements, not the target
/// application's resulting document. Keep that separate from the transaction
/// completion label so consumers cannot mistake an accepted output call for
/// verified text insertion.
fn application_result(completion: &str, insertion_attempted: bool) -> &'static str {
    match completion {
        "applied" | "inserted; cursor placement failed" => "unverified",
        "partial_write" | "application outcome unknown" => "unknown",
        "not applied" if insertion_attempted => "not_applied",
        "not applied" => "not_attempted",
        _ if insertion_attempted => "unknown",
        _ => "not_attempted",
    }
}

/// Fill the history with its longest possible records, for status-size tests.
#[cfg(test)]
pub(crate) fn saturate_for_test() {
    for _ in 0..HISTORY_LIMIT {
        push(FailureTrace {
            transaction_id: next_transaction_id(),
            at_unix: u64::MAX,
            duration_us: Some(0),
            interrupt_to_backend_return_us: None,
            backend: "wlroots-virtual-keyboard".into(),
            expected_insert_units: Some(0),
            trigger_recognized: "yes",
            focus_admission: "not reached",
            backend_selected: "yes",
            erase_acknowledged: "transaction acknowledged; substeps not exposed",
            insertion_attempted: true,
            transaction_phases: vec!["prepared", "validated", "applying", "committed"],
            completion: "inserted; cursor placement failed",
            progress_tracked: false,
            erased_units: 0,
            inserted_units: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_result_distinguishes_refusal_from_confirmed_no_effect() {
        assert_eq!(application_result("not applied", false), "not_attempted");
        assert_eq!(application_result("not applied", true), "not_applied");
        assert_eq!(application_result("applied", true), "unverified");
        assert_eq!(application_result("partial_write", true), "unknown");
        assert_eq!(application_result("unexpected failure", true), "unknown");
    }

    struct MockInjector;

    impl TextInjector for MockInjector {
        fn name(&self) -> &'static str {
            "mock-output"
        }

        fn erase(&mut self, _trigger: &str) -> Result<(), InjectorError> {
            Ok(())
        }

        fn insert(&mut self, _text: &str) -> Result<(), InjectorError> {
            Ok(())
        }
    }

    struct PartialProgressInjector {
        checkpoint: wayexpand_core::OutputCheckpoint,
    }

    impl TextInjector for PartialProgressInjector {
        fn name(&self) -> &'static str {
            "progress-test"
        }
        fn set_output_checkpoint(&mut self, checkpoint: wayexpand_core::OutputCheckpoint) {
            self.checkpoint = checkpoint;
        }
        fn reset_output_progress(&mut self) {
            self.checkpoint.reset_progress();
        }
        fn output_progress(&self) -> wayexpand_core::OutputProgress {
            self.checkpoint.progress()
        }
        fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
            self.checkpoint.record_erasure(trigger.chars().count());
            Ok(())
        }
        fn insert(&mut self, _text: &str) -> Result<(), InjectorError> {
            self.checkpoint.record_insertion(2);
            Err(InjectorError {
                backend: "progress-test",
                message: "simulated interruption".into(),
                retryable: false,
            })
        }
    }

    #[test]
    fn successful_outputs_do_not_evict_reported_failures() {
        let _history_guard = history_test_guard();
        let mut history = History::default();
        push_history(
            &mut history,
            FailureTrace {
                transaction_id: next_transaction_id(),
                at_unix: 1,
                duration_us: None,
                interrupt_to_backend_return_us: None,
                backend: "evict-check".into(),
                expected_insert_units: None,
                trigger_recognized: "yes",
                focus_admission: "not reached",
                backend_selected: "not reached",
                erase_acknowledged: "not attempted",
                insertion_attempted: false,
                transaction_phases: vec!["prepared", "failed"],
                completion: "blocked by organization policy",
                progress_tracked: false,
                erased_units: 0,
                inserted_units: 0,
            },
        );
        for _ in 0..HISTORY_LIMIT * 2 {
            push_history(
                &mut history,
                FailureTrace {
                    transaction_id: next_transaction_id(),
                    at_unix: 2,
                    duration_us: Some(1),
                    interrupt_to_backend_return_us: None,
                    backend: "mock-output".into(),
                    expected_insert_units: Some(5),
                    trigger_recognized: "yes",
                    focus_admission: "passed",
                    backend_selected: "yes",
                    erase_acknowledged: "atomic replacement acknowledged",
                    insertion_attempted: true,
                    transaction_phases: vec!["prepared", "validated", "applying", "committed"],
                    completion: "applied",
                    progress_tracked: false,
                    erased_units: 0,
                    inserted_units: 0,
                },
            );
        }
        assert_eq!(history.failures.len(), 1);
        assert_eq!(history.failures[0].backend, "evict-check");
        assert_eq!(history.outcomes.len(), OUTCOME_STATUS_LIMIT);
        assert!(history
            .outcomes
            .iter()
            .all(|trace| trace.completion == "applied"));
    }

    #[test]
    fn successful_output_is_in_outcomes_and_excluded_from_failure_history() {
        let _history_guard = history_test_guard();
        let mut injector = MockInjector;
        let mut traced = TracedInjector::new(&mut injector, 5);
        let report = wayexpand_core::ExpansionEngine::apply_with_report(
            &mut traced,
            &wayexpand_core::ExpansionResult {
                snippet_id: "test".into(),
                trigger: ":x".into(),
                matched_text: ":x".into(),
                insert: "value".into(),
                cursor_offset: None,
                reinsert_after: None,
                command_backed: false,
                undoable: false,
                folded_suffix_chars: 0,
            },
        );
        let _trace_id = traced.record_outcome(&report.outcome, &report.phases, report.progress);

        let outcomes: serde_json::Value =
            serde_json::from_str(&recent_outcomes_json()).expect("outcomes should be JSON");
        assert!(outcomes.as_array().unwrap().iter().any(|trace| {
            trace["completion"] == "applied"
                && trace["application_result"] == "unverified"
                && trace["duration_us"].as_u64().is_some()
                && trace["expected_insert_units"] == 5
                && trace["transaction_phases"]
                    == serde_json::json!(["prepared", "validated", "applying", "committed"])
        }));
        let failures: serde_json::Value =
            serde_json::from_str(&recent_json()).expect("failures should be JSON");
        assert!(failures
            .as_array()
            .unwrap()
            .iter()
            .all(|trace| trace["completion"] != "applied"));
    }

    #[test]
    fn correlated_success_can_be_downgraded_when_physical_input_overlaps() {
        let _history_guard = history_test_guard();
        let mut injector = MockInjector;
        let mut traced = TracedInjector::new(&mut injector, 5);
        let report = wayexpand_core::ExpansionEngine::apply_with_report(
            &mut traced,
            &wayexpand_core::ExpansionResult {
                snippet_id: "test".into(),
                trigger: ":x".into(),
                matched_text: ":x".into(),
                insert: "value".into(),
                cursor_offset: None,
                reinsert_after: None,
                command_backed: false,
                undoable: false,
                folded_suffix_chars: 0,
            },
        );
        assert!(report.outcome.is_output_acknowledged());
        let transaction_id =
            traced.record_outcome(&report.outcome, &report.phases, report.progress);
        assert!(mark_partial_after_input(&transaction_id, "evdev+libei"));
        record_interrupt_to_backend_return(
            &transaction_id,
            std::time::Duration::from_micros(4_200),
        );

        let outcomes: serde_json::Value =
            serde_json::from_str(&recent_outcomes_json()).expect("outcomes should be JSON");
        let trace = outcomes
            .as_array()
            .unwrap()
            .iter()
            .find(|trace| trace["transaction_id"] == transaction_id)
            .expect("the correlated transaction remains in bounded history");
        assert_eq!(trace["completion"], "partial_write");
        assert_eq!(trace["interrupt_to_backend_return_us"], 4_200);
        assert_eq!(
            trace["transaction_phases"].as_array().unwrap().last(),
            Some(&serde_json::json!("uncertain"))
        );
        assert_eq!(trace["recovery"], "manual_review");
    }

    #[test]
    fn interrupted_trace_enrichment_does_not_wait_for_a_status_reader() {
        let _history_guard = history_test_guard();
        let history_lock = history().lock().unwrap();
        let (finished, completion) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            record_interrupt_to_backend_return(
                "contended-history-test",
                std::time::Duration::from_micros(100),
            );
            finished.send(()).unwrap();
        });

        assert!(completion
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok());
        drop(history_lock);
        worker.join().unwrap();
    }

    #[test]
    fn dropped_worker_trace_still_records_physical_input_as_partial() {
        let _history_guard = history_test_guard();
        let transaction_id = next_transaction_id();

        assert!(!mark_partial_after_input(&transaction_id, "evdev+libei"));

        let outcomes: serde_json::Value =
            serde_json::from_str(&recent_outcomes_json()).expect("outcomes should be JSON");
        assert!(outcomes.as_array().unwrap().iter().any(|trace| {
            trace["transaction_id"] == transaction_id
                && trace["backend"] == "evdev+libei"
                && trace["completion"] == "partial_write"
                && trace["erase_acknowledged"] == "unknown"
                && trace["progress_tracked"] == false
        }));
    }

    #[test]
    fn partial_write_trace_includes_content_free_confirmed_progress() {
        let _history_guard = history_test_guard();
        let mut injector = PartialProgressInjector {
            checkpoint: wayexpand_core::OutputCheckpoint::default(),
        };
        let mut traced = TracedInjector::new(&mut injector, 11);
        let report = wayexpand_core::ExpansionEngine::apply_with_report(
            &mut traced,
            &wayexpand_core::ExpansionResult {
                snippet_id: "test".into(),
                trigger: ":x".into(),
                matched_text: ":x".into(),
                insert: "replacement".into(),
                cursor_offset: None,
                reinsert_after: None,
                command_backed: false,
                undoable: false,
                folded_suffix_chars: 0,
            },
        );
        assert!(matches!(
            report.outcome,
            TransactionOutcome::UnknownPartialFailure { .. }
        ));
        assert_eq!(report.progress.erased_units, 2);
        assert_eq!(report.progress.inserted_units, 2);
        let _trace_id = traced.record_outcome(&report.outcome, &report.phases, report.progress);

        let outcomes: serde_json::Value =
            serde_json::from_str(&recent_outcomes_json()).expect("outcomes should be JSON");
        assert!(outcomes.as_array().unwrap().iter().any(|trace| {
            trace["completion"] == "partial_write"
                && trace["duration_us"].as_u64().is_some()
                && trace["expected_insert_units"] == 11
                && trace["progress_tracked"] == true
                && trace["erased_units"] == 2
                && trace["inserted_units"] == 2
                && trace["backend"] == "progress-test"
        }));
    }

    #[test]
    fn pre_output_blocks_are_content_free_outcomes() {
        let _history_guard = history_test_guard();
        record_blocked_expansion("evdev+libei", "input arrived before output");
        record_command_failure("input-method-v2", wayexpand_core::CommandErrorKind::Timeout);

        let outcomes: serde_json::Value =
            serde_json::from_str(&recent_outcomes_json()).expect("outcomes should be JSON");
        assert!(outcomes.as_array().unwrap().iter().any(|trace| {
            trace["completion"] == "input arrived before output"
                && trace["backend"] == "evdev+libei"
                && trace["trigger_recognized"] == "yes"
                && trace["focus_admission"] == "not reached"
                && trace["backend_selected"] == "not reached"
                && trace["erase_acknowledged"] == "not attempted"
                && trace["insertion_attempted"] == false
        }));
        let block = outcomes
            .as_array()
            .unwrap()
            .iter()
            .find(|trace| trace["completion"] == "input arrived before output")
            .unwrap();
        assert!(block.get("trigger").is_none());
        assert!(block.get("replacement").is_none());
        assert!(block["transaction_id"]
            .as_str()
            .is_some_and(|id| id.contains('-')));
        assert!(outcomes.as_array().unwrap().iter().any(|trace| {
            trace["backend"] == "input-method-v2"
                && trace["completion"] == "command timed out before output"
                && trace["insertion_attempted"] == false
        }));
    }

    #[test]
    fn correlation_ids_are_unique_and_independent_of_content() {
        let first = next_transaction_id();
        let second = next_transaction_id();
        assert_ne!(first, second);
        assert!(!first.contains("trigger"));
        assert!(!second.contains("replacement"));
    }

    #[test]
    fn form_route_failure_has_a_specific_content_free_reason() {
        let _history_guard = history_test_guard();
        record_preparation_failure(
            "evdev+libei",
            &wayexpand_core::CommandError::FormTargetUnverifiable,
        );
        let failures: serde_json::Value =
            serde_json::from_str(&recent_json()).expect("failures should be JSON");
        assert!(failures.as_array().unwrap().iter().any(|trace| {
            trace["completion"] == "form requires verified replacement"
                && trace["backend"] == "evdev+libei"
                && trace["trigger_recognized"] == "yes"
                && trace["backend_selected"] == "not reached"
                && trace["insertion_attempted"] == false
        }));
    }

    #[test]
    fn form_render_failure_has_a_specific_content_free_reason() {
        let _history_guard = history_test_guard();
        record_command_failure(
            "evdev+libei",
            wayexpand_core::CommandErrorKind::InvalidTemplate,
        );
        let failures: serde_json::Value =
            serde_json::from_str(&recent_json()).expect("failures should be JSON");
        assert!(failures.as_array().unwrap().iter().any(|trace| {
            trace["completion"] == "form values could not render the template"
                && trace["backend"] == "evdev+libei"
                && trace["trigger_recognized"] == "yes"
                && trace["backend_selected"] == "not reached"
                && trace["insertion_attempted"] == false
                && trace.get("trigger").is_none()
                && trace.get("replacement").is_none()
        }));
    }

    #[test]
    fn preparation_failures_use_specific_content_free_reasons() {
        use wayexpand_core::CommandError;

        let cases = [
            (
                CommandError::SpawnFailed {
                    detail: "secret path must not be recorded".into(),
                },
                "helper could not start",
            ),
            (
                CommandError::InvalidUtf8,
                "helper output was not valid UTF-8",
            ),
            (
                CommandError::OutputChannelLost,
                "helper output channel was lost",
            ),
            (
                CommandError::IncompleteOutput,
                "helper output was incomplete",
            ),
        ];

        for (error, expected) in cases {
            // Test the pure mapping rather than racing the process-wide bounded
            // diagnostic ring against unrelated daemon tests running in
            // parallel. The recorder accepts only this fixed, content-free
            // label, so helper details cannot enter the public trace.
            assert_eq!(preparation_failure_reason(&error), expected);
        }
    }

    #[test]
    fn form_command_failures_have_specific_content_free_reasons() {
        assert_eq!(
            command_failure_reason(wayexpand_core::CommandErrorKind::IncompleteFormOutput),
            "form helper output stream did not close"
        );
        assert_eq!(
            command_failure_reason(wayexpand_core::CommandErrorKind::InvalidFormOutput),
            "form submission values are invalid or incomplete"
        );
    }
}
