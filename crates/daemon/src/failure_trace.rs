//! Small, content-free history of expansion output transactions.
//!
//! The trace records only transaction stages and the negotiated backend. It
//! never stores triggers, replacement text, key events, or backend errors,
//! since those strings may contain user content.

use std::{
    cell::Cell,
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use wayexpand_core::{
    InjectorCapabilities, InjectorError, KeyEventState, Modifiers, OutputCheckpoint, TextInjector,
    TransactionOutcome,
};

const HISTORY_LIMIT: usize = 8;
const FAILURE_STATUS_LIMIT: usize = 3;
const OUTCOME_STATUS_LIMIT: usize = 4;

#[derive(Clone, Debug)]
pub(crate) struct FailureTrace {
    at_unix: u64,
    backend: String,
    trigger_recognized: &'static str,
    focus_admission: &'static str,
    backend_selected: &'static str,
    erase_acknowledged: &'static str,
    insertion_attempted: bool,
    completion: &'static str,
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
}

impl<'a> TracedInjector<'a> {
    pub(crate) fn new(inner: &'a mut dyn TextInjector) -> Self {
        Self {
            stages: Stages {
                backend: inner.name().to_owned(),
                admission: Cell::new("not reached"),
                ..Stages::default()
            },
            inner,
        }
    }

    pub(crate) fn record_outcome(self, outcome: &TransactionOutcome) {
        let completion = match outcome {
            TransactionOutcome::Applied => "applied",
            TransactionOutcome::AppliedWithCursorPositionFailure { .. } => {
                "inserted; cursor placement failed"
            }
            TransactionOutcome::NotApplied { .. } => "not applied",
            TransactionOutcome::UnknownApplicationOutcome { .. } => "application outcome unknown",
            TransactionOutcome::UnknownPartialFailure { .. } => "partial result unknown",
        };
        let erase_acknowledged = if self.stages.erase.is_empty() {
            if self.inner.capabilities().atomic_replace {
                "atomic replacement not acknowledged"
            } else {
                "substep not observable"
            }
        } else {
            self.stages.erase
        };
        push(FailureTrace {
            at_unix: now_unix(),
            backend: self.stages.backend,
            trigger_recognized: "yes",
            focus_admission: self.stages.admission.get(),
            backend_selected: "yes",
            erase_acknowledged,
            insertion_attempted: self.stages.insertion_attempted,
            completion,
        });
    }
}

/// Record a matched expansion stopped before output. Only callers supply
/// fixed reason labels; no trigger, replacement, policy text, or user input is
/// retained. Backend names are bounded and stripped to a small identifier set
/// because they are serialized into the daemon status response.
pub(crate) fn record_blocked_expansion(backend: &str, reason: &'static str) {
    let backend = backend
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "-_.+".contains(*character))
        .take(64)
        .collect::<String>();
    push(FailureTrace {
        at_unix: now_unix(),
        backend,
        trigger_recognized: "yes",
        focus_admission: "not reached",
        backend_selected: "not reached",
        erase_acknowledged: "not attempted",
        insertion_attempted: false,
        completion: reason,
    });
}

pub(crate) fn record_command_failure(backend: &str, kind: wayexpand_core::CommandErrorKind) {
    use wayexpand_core::CommandErrorKind as Kind;
    let reason = match kind {
        Kind::Timeout => "command timed out before output",
        Kind::BrokerUnavailable => "action broker unavailable",
        Kind::OutputTooLarge => "command output exceeded policy limit",
        Kind::Policy => "command blocked by policy",
        Kind::UnsafeContext => "command context was no longer safe",
        Kind::StaleInput => "command result became stale",
        Kind::WindowIdentity => "command focus could not be verified",
        Kind::QueueFull | Kind::WorkerUnavailable => "command worker queue unavailable",
        Kind::Spawn
        | Kind::ProcessWait
        | Kind::BrokerProtocol
        | Kind::BrokerRejected
        | Kind::NonZeroExit
        | Kind::InvalidOutput
        | Kind::OutputChannel
        | Kind::IncompleteOutput => "command execution failed before output",
    };
    record_blocked_expansion(backend, reason);
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

impl TextInjector for TracedInjector<'_> {
    fn shutdown(self: Box<Self>) {}
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn capabilities(&self) -> InjectorCapabilities {
        self.inner.capabilities()
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
    fn set_output_checkpoint(&mut self, checkpoint: OutputCheckpoint) {
        self.inner.set_output_checkpoint(checkpoint)
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
        let atomic = self.inner.capabilities().atomic_replace;
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

fn history() -> &'static Mutex<VecDeque<FailureTrace>> {
    static HISTORY: OnceLock<Mutex<VecDeque<FailureTrace>>> = OnceLock::new();
    HISTORY.get_or_init(|| Mutex::new(VecDeque::with_capacity(HISTORY_LIMIT)))
}

fn push(trace: FailureTrace) {
    let mut history = history()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if history.len() == HISTORY_LIMIT {
        history.pop_front();
    }
    history.push_back(trace);
}

pub(crate) fn recent_json() -> String {
    let history = history()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    traces_json(
        history
            .iter()
            .filter(|trace| trace.completion != "applied")
            .rev()
            .take(FAILURE_STATUS_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    )
}

/// Recent successful and blocked output transactions. Each record contains
/// only stage outcomes and backend identity, never user text.
pub(crate) fn recent_outcomes_json() -> String {
    let history = history()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    traces_json(
        history
            .iter()
            .rev()
            .take(OUTCOME_STATUS_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    )
}

fn traces_json<'a>(traces: impl Iterator<Item = &'a FailureTrace>) -> String {
    let traces = traces
        .map(|trace| {
            serde_json::json!({
                "at_unix": trace.at_unix,
                "backend": trace.backend,
                "trigger_recognized": trace.trigger_recognized,
                "focus_admission": trace.focus_admission,
                "backend_selected": trace.backend_selected,
                "erase_acknowledged": trace.erase_acknowledged,
                "insertion_attempted": trace.insertion_attempted,
                "completion": trace.completion,
            })
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(traces).to_string()
}

/// Fill the history with its longest possible records, for status-size tests.
#[cfg(test)]
pub(crate) fn saturate_for_test() {
    for _ in 0..HISTORY_LIMIT {
        push(FailureTrace {
            at_unix: u64::MAX,
            backend: "wlroots-virtual-keyboard".into(),
            trigger_recognized: "yes",
            focus_admission: "not reached",
            backend_selected: "yes",
            erase_acknowledged: "transaction acknowledged; substeps not exposed",
            insertion_attempted: true,
            completion: "inserted; cursor placement failed",
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn successful_output_is_in_outcomes_and_excluded_from_failure_history() {
        let mut injector = MockInjector;
        TracedInjector::new(&mut injector).record_outcome(&TransactionOutcome::Applied);

        let outcomes: serde_json::Value =
            serde_json::from_str(&recent_outcomes_json()).expect("outcomes should be JSON");
        assert!(outcomes
            .as_array()
            .unwrap()
            .iter()
            .any(|trace| trace["completion"] == "applied"));
        let failures: serde_json::Value =
            serde_json::from_str(&recent_json()).expect("failures should be JSON");
        assert!(failures
            .as_array()
            .unwrap()
            .iter()
            .all(|trace| trace["completion"] != "applied"));
    }

    #[test]
    fn pre_output_blocks_are_content_free_outcomes() {
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
        assert!(outcomes.as_array().unwrap().iter().any(|trace| {
            trace["backend"] == "input-method-v2"
                && trace["completion"] == "command timed out before output"
                && trace["insertion_attempted"] == false
        }));
    }
}
