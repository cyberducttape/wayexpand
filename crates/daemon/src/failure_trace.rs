//! Small, content-free history of failed expansion transactions.
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

    pub(crate) fn record_failure(self, outcome: &TransactionOutcome) {
        if matches!(outcome, TransactionOutcome::Applied) {
            return;
        }
        let completion = match outcome {
            TransactionOutcome::Applied => "applied",
            TransactionOutcome::AppliedWithCursorPositionFailure { .. } => {
                "inserted; cursor placement failed"
            }
            TransactionOutcome::NotApplied { .. } => "not applied",
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
            at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_secs()),
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
    let traces = history
        .iter()
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
