//! Insertion assurance: what an output route can promise before it erases.
//!
//! Target-verified routes (input-method-v2, IBus) check the target's text
//! before deleting. Atomic routes also avoid the erase/insert gap. Routes
//! without target verification require explicit compatibility consent:
//!
//! - unverified routes never erase unless the user enabled `compatibility_mode`;
//! - a focus change detected between matching and typing always refuses;
//! - focus that no tracker can confirm refuses unless the user accepted
//!   `untracked-focus`.
//!
//! Refusals happen before the first keystroke. Every refusal is counted and
//! published in daemon status so the original trigger remains available for
//! manual recovery.

use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc, Mutex, OnceLock,
};

use tracing::warn;
use wayexpand_core::{
    CompatibilityMode, FocusCheck, FocusStamp, FocusWitness, InjectorCapabilities, InjectorError,
    KeyEventState, Modifiers, OutputCheckpoint, ReplacementAdmission, TextInjector,
};

/// Why an output was refused before anything was typed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refusal {
    /// The route cannot verify erasure and compatibility mode is off.
    CompatibilityModeOff,
    /// Focus moved, became unknown, or the tracker stopped after the match.
    FocusChanged,
    /// No tracker confirmed focus and the user did not accept that.
    FocusUnconfirmed,
}

impl Refusal {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CompatibilityModeOff => "compatibility-mode-off",
            Self::FocusChanged => "focus-changed",
            Self::FocusUnconfirmed => "focus-unconfirmed",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::CompatibilityModeOff => InjectorError::REFUSAL_COMPATIBILITY_MODE_OFF,
            Self::FocusChanged => InjectorError::REFUSAL_FOCUS_CHANGED,
            Self::FocusUnconfirmed => InjectorError::REFUSAL_FOCUS_UNCONFIRMED,
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::CompatibilityModeOff => 1,
            Self::FocusChanged => 2,
            Self::FocusUnconfirmed => 3,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::CompatibilityModeOff),
            2 => Some(Self::FocusChanged),
            3 => Some(Self::FocusUnconfirmed),
            _ => None,
        }
    }
}

/// How an output route is assured, for status and diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Assurance {
    /// The route checks the target's text before erasing.
    Verified,
    /// The tracker confirms the focused window, but the route cannot inspect
    /// the text or cursor position it will erase.
    FocusTrackedBestEffort,
    /// The user allowed a keystroke route without confirmed target focus.
    Unverified,
    /// Keystroke route without consent: typed expansions are refused.
    Blocked,
    /// No output route is connected.
    Unavailable,
}

impl Assurance {
    pub(crate) fn of(
        capabilities: Option<&InjectorCapabilities>,
        mode: CompatibilityMode,
        focus: FocusCheck,
    ) -> Self {
        match capabilities {
            None => Self::Unavailable,
            Some(capabilities) if capabilities.has_atomic_replacement_contract() => Self::Verified,
            Some(_) if mode == CompatibilityMode::Off => Self::Blocked,
            Some(_) => match focus {
                FocusCheck::Unchanged => Self::FocusTrackedBestEffort,
                FocusCheck::Changed => Self::Blocked,
                FocusCheck::Unconfirmed if mode == CompatibilityMode::UntrackedFocus => {
                    Self::Unverified
                }
                FocusCheck::Unconfirmed => Self::Blocked,
            },
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::FocusTrackedBestEffort => "focus-tracked-best-effort",
            Self::Unverified => "unverified",
            Self::Blocked => "blocked",
            Self::Unavailable => "unavailable",
        }
    }

    /// Preserve the older, coarser field for clients that predate schema 13.
    pub(crate) fn legacy_as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::FocusTrackedBestEffort | Self::Unverified => "compatibility",
            Self::Blocked => "blocked",
            Self::Unavailable => "unavailable",
        }
    }
}

/// The reactor's view of consent and focus, shared with output threads.
#[derive(Default)]
pub(crate) struct GuardState {
    /// Low two bits hold CompatibilityMode; upper bits form a monotonically
    /// advancing generation so disable/re-enable cannot ABA an active output.
    mode_state: AtomicU64,
    /// Focus identity of the window snapshot the engine currently reflects.
    stamp: Mutex<FocusStamp>,
    /// Immutable authorization baseline for the output transaction currently
    /// running on the serialized injector. Neither a new focus stamp nor a
    /// changed compatibility mode can re-authorize an in-flight stream.
    output_baseline: Mutex<Option<OutputBaseline>>,
    witness: Mutex<Option<FocusWitness>>,
    refused_total: AtomicU64,
    last_refusal: AtomicU8,
}

#[derive(Clone, Copy)]
struct OutputBaseline {
    stamp: FocusStamp,
    mode: CompatibilityMode,
    mode_state: u64,
    focus_was_confirmed: bool,
}

/// The output worker and scheduler tests share this final admission rule.
fn output_baseline_is_valid(
    baseline: OutputBaseline,
    current_mode_state: u64,
    focus: FocusCheck,
) -> bool {
    if current_mode_state != baseline.mode_state {
        return false;
    }
    match focus {
        FocusCheck::Changed => false,
        FocusCheck::Unchanged => true,
        // Tracked focus, and any transaction that began with an exact target,
        // must remain confirmed for the entire output. Explicit untracked
        // consent permits only a transaction that was already unconfirmed
        // when it started to remain unconfirmed.
        FocusCheck::Unconfirmed => {
            baseline.mode == CompatibilityMode::UntrackedFocus && !baseline.focus_was_confirmed
        }
    }
}

/// Keep the production baseline type private while allowing cross-module
/// scheduler models to exercise the exact admission predicate.
#[cfg(all(test, feature = "loom-tests"))]
pub(crate) fn tracked_output_baseline_is_valid(current_mode_state: u64, focus: FocusCheck) -> bool {
    output_baseline_is_valid(
        OutputBaseline {
            stamp: FocusStamp::default(),
            mode: CompatibilityMode::TrackedFocus,
            mode_state: (1 << 2) | 1,
            focus_was_confirmed: true,
        },
        current_mode_state,
        focus,
    )
}

impl GuardState {
    /// The daemon's guard state. One daemon runs per process.
    pub(crate) fn global() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<GuardState>> = OnceLock::new();
        Arc::clone(GLOBAL.get_or_init(Arc::default))
    }

    pub(crate) fn set_mode(&self, mode: CompatibilityMode) {
        let code = match mode {
            CompatibilityMode::Off => 0,
            CompatibilityMode::TrackedFocus => 1,
            CompatibilityMode::UntrackedFocus => 2,
        };
        let mut current = self.mode_state.load(Ordering::Acquire);
        loop {
            if current & 0b11 == code {
                return;
            }
            // Reserve two low bits for the mode. Wrapping would require more
            // than four quintillion policy transitions in one process.
            let generation = (current >> 2).wrapping_add(1);
            let next = (generation << 2) | code;
            match self.mode_state.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn mode(&self) -> CompatibilityMode {
        Self::mode_from_state(self.mode_state.load(Ordering::Acquire))
    }

    fn mode_from_state(state: u64) -> CompatibilityMode {
        match state & 0b11 {
            1 => CompatibilityMode::TrackedFocus,
            2 => CompatibilityMode::UntrackedFocus,
            _ => CompatibilityMode::Off,
        }
    }

    pub(crate) fn set_witness(&self, witness: Option<FocusWitness>) {
        *self
            .witness
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = witness;
    }

    pub(crate) fn set_stamp(&self, stamp: FocusStamp) {
        *self
            .stamp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = stamp;
    }

    pub(crate) fn focus(&self) -> FocusCheck {
        let stamp = *self
            .stamp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.focus_at(stamp)
    }

    fn focus_at(&self, stamp: FocusStamp) -> FocusCheck {
        self.witness
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map_or(FocusCheck::Unconfirmed, |witness| witness.check(stamp))
    }

    fn begin_output_transaction(&self) {
        let mode_state = self.mode_state.load(Ordering::Acquire);
        let mode = Self::mode_from_state(mode_state);
        let stamp = *self
            .stamp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *self
            .output_baseline
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(OutputBaseline {
            stamp,
            mode,
            mode_state,
            focus_was_confirmed: self.focus_at(stamp) == FocusCheck::Unchanged,
        });
    }

    fn output_is_valid(&self) -> bool {
        let baseline = *self
            .output_baseline
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(baseline) = baseline else {
            return self.focus() != FocusCheck::Changed;
        };
        output_baseline_is_valid(
            baseline,
            self.mode_state.load(Ordering::Acquire),
            self.focus_at(baseline.stamp),
        )
    }

    /// Decide whether an output may start. Insert-only output is always
    /// admissible. Replacement is admissible without compatibility consent
    /// only when erase and insertion are covered by the same atomic contract;
    /// verifying surrounding text does not make separate operations atomic.
    pub(crate) fn decide(
        &self,
        capabilities: &InjectorCapabilities,
        erases: bool,
    ) -> Result<(), Refusal> {
        if !erases
            || capabilities.replacement_admission(self.mode()) == ReplacementAdmission::Atomic
        {
            return Ok(());
        }
        let mode = self.mode();
        if capabilities.replacement_admission(mode) == ReplacementAdmission::Refused {
            return Err(Refusal::CompatibilityModeOff);
        }
        match (self.focus(), mode) {
            (FocusCheck::Unchanged, _) => Ok(()),
            (FocusCheck::Changed, _) => Err(Refusal::FocusChanged),
            (FocusCheck::Unconfirmed, CompatibilityMode::UntrackedFocus) => Ok(()),
            (FocusCheck::Unconfirmed, _) => Err(Refusal::FocusUnconfirmed),
        }
    }

    fn admit(
        &self,
        backend: &'static str,
        capabilities: &InjectorCapabilities,
        erases: bool,
    ) -> Result<(), InjectorError> {
        self.decide(capabilities, erases).map_err(|refusal| {
            self.refused_total.fetch_add(1, Ordering::Relaxed);
            self.last_refusal.store(refusal.code(), Ordering::Relaxed);
            warn!(
                backend,
                reason = refusal.as_str(),
                "output refused before typing; nothing was erased or inserted"
            );
            InjectorError {
                backend,
                message: refusal.message().into(),
                retryable: false,
            }
        })
    }

    pub(crate) fn refused_total(&self) -> u64 {
        self.refused_total.load(Ordering::Relaxed)
    }

    pub(crate) fn last_refusal(&self) -> Option<Refusal> {
        Refusal::from_code(self.last_refusal.load(Ordering::Relaxed))
    }

    /// Stops an output stream that is already running when the tracker
    /// reports a different window than the one the output was admitted for.
    /// Unconfirmed focus was already decided by `decide` before output.
    fn focus_checkpoint(guard: Arc<Self>) -> OutputCheckpoint {
        OutputCheckpoint::new(move || {
            if !guard.output_is_valid() {
                warn!("focus or replacement consent changed during output; stopping output");
                Err("focus or replacement consent changed during output".into())
            } else {
                Ok(())
            }
        })
    }

    /// How focus is confirmed for keystroke routes right now.
    pub(crate) fn focus_assurance(&self) -> &'static str {
        match self.focus() {
            FocusCheck::Unchanged => "tracker",
            FocusCheck::Changed | FocusCheck::Unconfirmed => "unconfirmed",
        }
    }
}

/// A keystroke-synthesis backend behind the assurance gate. Every expansion
/// transaction is admitted through [`GuardState::decide`], and erasing
/// operations repeat the check in case a caller bypassed `admit`.
pub(crate) struct AssuredInjector {
    inner: Box<dyn TextInjector>,
    guard: Arc<GuardState>,
}

impl AssuredInjector {
    pub(crate) fn wrap(
        inner: Box<dyn TextInjector>,
        guard: Arc<GuardState>,
    ) -> Box<dyn TextInjector> {
        let mut inner = inner;
        inner.set_output_checkpoint(GuardState::focus_checkpoint(Arc::clone(&guard)));
        Box::new(Self { inner, guard })
    }

    fn admit_erase(&self) -> Result<(), InjectorError> {
        self.guard
            .admit(self.inner.name(), &self.inner.capabilities(), true)
    }
}

impl TextInjector for AssuredInjector {
    fn shutdown(self: Box<Self>) {
        self.inner.shutdown();
    }

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

    fn set_output_checkpoint(&mut self, checkpoint: OutputCheckpoint) {
        // Keep the focus check; add the caller's (for example cancellation).
        self.inner.set_output_checkpoint(
            checkpoint.and(GuardState::focus_checkpoint(Arc::clone(&self.guard))),
        );
    }

    fn reset_output_progress(&mut self) {
        self.guard.begin_output_transaction();
        self.inner.reset_output_progress();
    }

    fn admit(&self, erases: bool) -> Result<(), InjectorError> {
        self.inner.admit(erases)?;
        self.guard
            .admit(self.inner.name(), &self.inner.capabilities(), erases)
    }

    fn preflight_text(&mut self, text: &str) -> Result<(), InjectorError> {
        self.inner.preflight_text(text)
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.admit_erase()?;
        self.inner.erase(trigger)
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.inner.insert(text)
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        if !trigger.is_empty() {
            self.admit_erase()?;
        }
        self.inner.replace(trigger, text)
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

#[cfg(test)]
mod tests {
    use super::*;
    use wayexpand_core::{
        window_update_channel, ExpansionEngine, ExpansionResult, ReplacementGuarantee,
        TransactionOutcome, WindowContext,
    };

    /// A keystroke backend that records everything it is asked to type.
    struct Keystrokes {
        typed: Arc<Mutex<Vec<String>>>,
        guarantee: ReplacementGuarantee,
    }

    impl TextInjector for Keystrokes {
        fn name(&self) -> &'static str {
            "keystrokes"
        }

        fn capabilities(&self) -> InjectorCapabilities {
            InjectorCapabilities {
                insertion_mode: "test keystrokes",
                full_unicode: true,
                atomic_replace: self.guarantee == ReplacementGuarantee::Atomic,
                replacement_guarantee: self.guarantee,
                ..InjectorCapabilities::default()
            }
        }

        fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
            self.typed.lock().unwrap().push(format!("erase:{trigger}"));
            Ok(())
        }

        fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
            self.typed.lock().unwrap().push(format!("insert:{text}"));
            Ok(())
        }
    }

    fn guarded(
        guard: &Arc<GuardState>,
        guarantee: ReplacementGuarantee,
    ) -> (Box<dyn TextInjector>, Arc<Mutex<Vec<String>>>) {
        let typed = Arc::new(Mutex::new(Vec::new()));
        let injector = AssuredInjector::wrap(
            Box::new(Keystrokes {
                typed: Arc::clone(&typed),
                guarantee,
            }),
            Arc::clone(guard),
        );
        (injector, typed)
    }

    fn expansion(matched: &str) -> ExpansionResult {
        ExpansionResult {
            snippet_id: String::new().into(),
            trigger: matched.into(),
            matched_text: matched.into(),
            insert: "replacement".into(),
            cursor_offset: None,
            reinsert_after: None,
            command_backed: false,
            undoable: true,
            folded_suffix_chars: 0,
        }
    }

    fn window(instance: &str) -> Option<WindowContext> {
        Some(WindowContext {
            app_id: Some("editor".into()),
            title: Some("draft".into()),
            instance_id: Some(instance.into()),
        })
    }

    fn refused_before_typing(outcome: &TransactionOutcome) -> bool {
        matches!(outcome, TransactionOutcome::NotApplied { source } if !source.retryable)
    }

    /// Types one character at a time, running the installed checkpoint
    /// before each like the libei keysym fallback, and moves focus to
    /// another window after `switch_after` characters.
    struct PacedKeystrokes {
        typed: Arc<Mutex<String>>,
        checkpoint: OutputCheckpoint,
        focus_sender: wayexpand_core::WindowUpdateSender,
        focus_receiver: wayexpand_core::WindowUpdateReceiver,
        guard: Arc<GuardState>,
        switch_after: usize,
    }

    impl TextInjector for PacedKeystrokes {
        fn name(&self) -> &'static str {
            "paced"
        }

        fn capabilities(&self) -> InjectorCapabilities {
            InjectorCapabilities {
                insertion_mode: "test paced keystrokes",
                full_unicode: true,
                replacement_guarantee: ReplacementGuarantee::BestEffort,
                expected_throughput_chars_per_sec: Some(83),
                ..InjectorCapabilities::default()
            }
        }

        fn set_output_checkpoint(&mut self, checkpoint: OutputCheckpoint) {
            self.checkpoint = checkpoint;
        }

        fn erase(&mut self, _trigger: &str) -> Result<(), InjectorError> {
            Ok(())
        }

        fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
            for (index, character) in text.chars().enumerate() {
                if index == self.switch_after {
                    self.focus_sender.send(window("w2")).unwrap();
                    // Model the daemon draining the tracker and advancing
                    // its current-focus baseline while this output remains
                    // in flight. The transaction checkpoint must retain the
                    // original window's stamp.
                    let (_, stamp) = self.focus_receiver.try_recv_stamped().unwrap();
                    self.guard.set_stamp(stamp);
                }
                self.checkpoint.check().map_err(|message| InjectorError {
                    backend: "paced",
                    message,
                    retryable: false,
                })?;
                self.typed.lock().unwrap().push(character);
            }
            Ok(())
        }
    }

    #[test]
    fn a_focus_change_while_typing_stops_the_stream_and_is_never_retried() {
        let guard = Arc::new(GuardState::default());
        guard.set_mode(CompatibilityMode::TrackedFocus);
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        sender.send(window("w1")).unwrap();
        guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
        let typed = Arc::new(Mutex::new(String::new()));
        let mut injector = AssuredInjector::wrap(
            Box::new(PacedKeystrokes {
                typed: Arc::clone(&typed),
                checkpoint: OutputCheckpoint::default(),
                focus_sender: sender,
                focus_receiver: receiver,
                guard: Arc::clone(&guard),
                switch_after: 3,
            }),
            Arc::clone(&guard),
        );

        let mut result = expansion(":sig");
        result.trigger = ":x".into();
        result.matched_text = ":x".into();
        result.insert = "data".into();
        let outcome = ExpansionEngine::apply(injector.as_mut(), &result);

        // This fixture remains within the interactive pacing budget. Three
        // characters reached the original window; nothing went to the new
        // one, and the result is uncertain rather than retried.
        assert_eq!(*typed.lock().unwrap(), "dat");
        assert!(
            matches!(outcome, TransactionOutcome::UnknownPartialFailure { .. }),
            "{outcome}"
        );
    }

    #[test]
    fn an_in_flight_output_stops_when_consent_or_confirmed_focus_is_lost() {
        // Revoking tracked-focus consent invalidates the already-running
        // non-atomic transaction at its next output checkpoint.
        let consent_guard = Arc::new(GuardState::default());
        consent_guard.set_mode(CompatibilityMode::TrackedFocus);
        let (consent_sender, consent_receiver) = window_update_channel();
        consent_guard.set_witness(Some(consent_receiver.focus_witness()));
        consent_sender.send(window("window-a")).unwrap();
        consent_guard.set_stamp(consent_receiver.try_recv_stamped().unwrap().1);
        consent_guard.begin_output_transaction();
        let consent_checkpoint = GuardState::focus_checkpoint(Arc::clone(&consent_guard));
        assert!(consent_checkpoint.check().is_ok());
        // Reapplying the same effective setting does not cancel output.
        consent_guard.set_mode(CompatibilityMode::TrackedFocus);
        assert!(consent_checkpoint.check().is_ok());
        // A revoke followed by a re-grant must not ABA the generation and
        // revive a transaction admitted under the earlier consent.
        consent_guard.set_mode(CompatibilityMode::Off);
        consent_guard.set_mode(CompatibilityMode::TrackedFocus);
        assert!(consent_checkpoint.check().is_err());

        // An explicitly untracked transaction may begin without a witness,
        // but a transaction that began with exact focus cannot continue after
        // that confirmation disappears.
        let focus_guard = Arc::new(GuardState::default());
        focus_guard.set_mode(CompatibilityMode::UntrackedFocus);
        let (focus_sender, focus_receiver) = window_update_channel();
        focus_guard.set_witness(Some(focus_receiver.focus_witness()));
        focus_sender.send(window("window-a")).unwrap();
        focus_guard.set_stamp(focus_receiver.try_recv_stamped().unwrap().1);
        focus_guard.begin_output_transaction();
        let focus_checkpoint = GuardState::focus_checkpoint(Arc::clone(&focus_guard));
        assert!(focus_checkpoint.check().is_ok());
        focus_guard.set_witness(None);
        assert!(focus_checkpoint.check().is_err());
    }

    #[test]
    fn a_focus_change_before_the_worker_starts_invalidates_its_transaction() {
        let guard = Arc::new(GuardState::default());
        guard.set_mode(CompatibilityMode::TrackedFocus);
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        sender.send(window("window-a")).unwrap();
        guard.set_stamp(receiver.try_recv_stamped().unwrap().1);

        // The reactor queues a transaction for window-a, then focus changes
        // before the serialized worker begins it. Even if focus returns to
        // window-a before the worker runs, the witness sequence records that
        // the queued transaction crossed a target boundary.
        sender.send(window("window-b")).unwrap();
        sender.send(window("window-a")).unwrap();
        guard.begin_output_transaction();

        let checkpoint = GuardState::focus_checkpoint(guard);
        assert!(checkpoint.check().is_err());
    }

    #[test]
    fn keystroke_routes_never_erase_without_compatibility_mode() {
        let guard = Arc::new(GuardState::default());
        let (mut injector, typed) = guarded(&guard, ReplacementGuarantee::BestEffort);

        let outcome = ExpansionEngine::apply(injector.as_mut(), &expansion(":sig"));

        assert!(refused_before_typing(&outcome), "{outcome}");
        assert!(typed.lock().unwrap().is_empty(), "nothing may be typed");
        assert_eq!(guard.refused_total(), 1);
        assert_eq!(guard.last_refusal(), Some(Refusal::CompatibilityModeOff));
        // Bypassing admit() does not bypass the gate either.
        assert!(injector.erase(":sig").is_err());
        assert!(typed.lock().unwrap().is_empty());
    }

    #[test]
    fn consent_revoked_after_admission_is_reported_as_clean_refusal() {
        struct RevokeConsentAfterPreflight {
            inner: Box<dyn TextInjector>,
            guard: Arc<GuardState>,
        }

        impl TextInjector for RevokeConsentAfterPreflight {
            fn shutdown(self: Box<Self>) {
                self.inner.shutdown();
            }
            fn name(&self) -> &'static str {
                self.inner.name()
            }
            fn capabilities(&self) -> InjectorCapabilities {
                self.inner.capabilities()
            }
            fn admit(&self, erases: bool) -> Result<(), InjectorError> {
                self.inner.admit(erases)
            }
            fn preflight_text(&mut self, text: &str) -> Result<(), InjectorError> {
                self.inner.preflight_text(text)?;
                self.guard.set_mode(CompatibilityMode::Off);
                Ok(())
            }
            fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
                self.inner.replace(trigger, text)
            }
            fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
                self.inner.erase(trigger)
            }
            fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
                self.inner.insert(text)
            }
        }

        let guard = Arc::new(GuardState::default());
        guard.set_mode(CompatibilityMode::TrackedFocus);
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        sender.send(window("window-a")).unwrap();
        guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
        let (inner, typed) = guarded(&guard, ReplacementGuarantee::BestEffort);
        let mut injector = RevokeConsentAfterPreflight { inner, guard };

        let report = ExpansionEngine::apply_with_report(&mut injector, &expansion(":sig"));

        assert!(matches!(
            report.outcome,
            TransactionOutcome::NotApplied { .. }
        ));
        assert_eq!(report.progress.erased_units, 0);
        assert_eq!(report.progress.inserted_units, 0);
        assert_eq!(*typed.lock().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn non_atomic_verified_routes_are_blocked_but_atomic_and_insert_only_are_allowed() {
        let guard = Arc::new(GuardState::default());
        let (mut verified, typed) = guarded(&guard, ReplacementGuarantee::VerifiedSurroundingText);
        let outcome = ExpansionEngine::apply(verified.as_mut(), &expansion(":sig"));
        assert!(refused_before_typing(&outcome), "{outcome}");
        assert!(typed.lock().unwrap().is_empty());

        let guard = Arc::new(GuardState::default());
        let (mut atomic, typed) = guarded(&guard, ReplacementGuarantee::Atomic);
        assert!(
            ExpansionEngine::apply(atomic.as_mut(), &expansion(":sig")).is_output_acknowledged()
        );
        assert_eq!(typed.lock().unwrap().len(), 2);

        // Inserting without erasing cannot delete unverified text.
        let (mut keystrokes, typed) = guarded(&guard, ReplacementGuarantee::BestEffort);
        assert!(
            ExpansionEngine::apply(keystrokes.as_mut(), &expansion("")).is_output_acknowledged()
        );
        // The default replace() erases an empty trigger, which deletes nothing.
        assert_eq!(*typed.lock().unwrap(), ["erase:", "insert:replacement"]);
        assert_eq!(guard.refused_total(), 0);
    }

    #[test]
    fn verified_non_atomic_replacement_requires_consent_and_continuous_focus() {
        let guard = Arc::new(GuardState::default());
        let (mut injector, typed) = guarded(&guard, ReplacementGuarantee::VerifiedSurroundingText);

        // A surrounding-text precondition alone does not authorize a split
        // erase/insert transaction.
        let outcome = ExpansionEngine::apply(injector.as_mut(), &expansion(":sig"));
        assert!(refused_before_typing(&outcome), "{outcome}");
        assert!(typed.lock().unwrap().is_empty());

        // Explicit compatibility consent is still tied to an exact tracker
        // identity for tracked-focus mode.
        guard.set_mode(CompatibilityMode::TrackedFocus);
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        sender.send(window("window-a")).unwrap();
        guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
        assert!(
            ExpansionEngine::apply(injector.as_mut(), &expansion(":sig")).is_output_acknowledged()
        );
        let admitted_output = typed.lock().unwrap().clone();
        assert_eq!(admitted_output.len(), 2);

        // The same tracker witness invalidates the old target even before
        // the reactor publishes a new stamp. No second erase may be issued.
        sender.send(window("window-b")).unwrap();
        let outcome = ExpansionEngine::apply(injector.as_mut(), &expansion(":sig"));
        assert!(refused_before_typing(&outcome), "{outcome}");
        assert_eq!(*typed.lock().unwrap(), admitted_output);
    }

    #[test]
    fn tracked_focus_requires_a_tracker_to_confirm_the_window() {
        let guard = Arc::new(GuardState::default());
        guard.set_mode(CompatibilityMode::TrackedFocus);
        let (mut injector, typed) = guarded(&guard, ReplacementGuarantee::BestEffort);

        // No tracker at all.
        let outcome = ExpansionEngine::apply(injector.as_mut(), &expansion(":sig"));
        assert!(refused_before_typing(&outcome), "{outcome}");
        assert_eq!(guard.last_refusal(), Some(Refusal::FocusUnconfirmed));

        // A tracker that confirms the exact window the engine reflects.
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        sender.send(window("w1")).unwrap();
        guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
        assert!(
            ExpansionEngine::apply(injector.as_mut(), &expansion(":sig")).is_output_acknowledged()
        );
        assert_eq!(*typed.lock().unwrap(), ["erase::sig", "insert:replacement"]);
    }

    #[test]
    fn a_focus_change_after_the_match_is_refused_in_every_mode() {
        for mode in [
            CompatibilityMode::TrackedFocus,
            CompatibilityMode::UntrackedFocus,
        ] {
            let guard = Arc::new(GuardState::default());
            guard.set_mode(mode);
            let (sender, receiver) = window_update_channel();
            guard.set_witness(Some(receiver.focus_witness()));
            sender.send(window("w1")).unwrap();
            guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
            let (mut injector, typed) = guarded(&guard, ReplacementGuarantee::BestEffort);

            // Focus moves (for example during the key-release wait) before
            // the reactor has even received the new snapshot.
            sender.send(window("w2")).unwrap();
            let outcome = ExpansionEngine::apply(injector.as_mut(), &expansion(":sig"));

            assert!(refused_before_typing(&outcome), "{mode:?}: {outcome}");
            assert!(typed.lock().unwrap().is_empty());
            assert_eq!(guard.last_refusal(), Some(Refusal::FocusChanged));

            // A tracker that stops without a final report is also a change.
            guard.set_stamp(receiver.try_recv_stamped().unwrap().1);
            drop(sender);
            let outcome = ExpansionEngine::apply(injector.as_mut(), &expansion(":sig"));
            assert!(refused_before_typing(&outcome), "{mode:?}: {outcome}");
        }
    }

    #[test]
    fn untracked_focus_is_allowed_only_when_explicitly_accepted() {
        let guard = Arc::new(GuardState::default());
        guard.set_mode(CompatibilityMode::UntrackedFocus);
        let (mut injector, typed) = guarded(&guard, ReplacementGuarantee::BestEffort);

        assert!(
            ExpansionEngine::apply(injector.as_mut(), &expansion(":sig")).is_output_acknowledged()
        );
        assert_eq!(typed.lock().unwrap().len(), 2);
        assert_eq!(guard.focus_assurance(), "unconfirmed");
    }

    #[test]
    fn status_assurance_distinguishes_verified_compatibility_and_blocked_routes() {
        let keystrokes = InjectorCapabilities {
            replacement_guarantee: ReplacementGuarantee::BestEffort,
            ..InjectorCapabilities::default()
        };
        let atomic = InjectorCapabilities {
            atomic_replace: true,
            replacement_guarantee: ReplacementGuarantee::Atomic,
            ..InjectorCapabilities::default()
        };
        assert_eq!(
            Assurance::of(
                Some(&atomic),
                CompatibilityMode::Off,
                FocusCheck::Unconfirmed
            ),
            Assurance::Verified
        );
        assert_eq!(
            Assurance::of(
                Some(&keystrokes),
                CompatibilityMode::Off,
                FocusCheck::Unchanged
            ),
            Assurance::Blocked
        );
        assert_eq!(
            Assurance::of(
                Some(&keystrokes),
                CompatibilityMode::TrackedFocus,
                FocusCheck::Unchanged
            ),
            Assurance::FocusTrackedBestEffort
        );
        assert_eq!(
            Assurance::of(
                Some(&keystrokes),
                CompatibilityMode::TrackedFocus,
                FocusCheck::Unconfirmed
            ),
            Assurance::Blocked
        );
        assert_eq!(
            Assurance::of(
                Some(&keystrokes),
                CompatibilityMode::UntrackedFocus,
                FocusCheck::Unconfirmed
            ),
            Assurance::Unverified
        );
        assert_eq!(
            Assurance::of(
                None,
                CompatibilityMode::UntrackedFocus,
                FocusCheck::Unconfirmed
            ),
            Assurance::Unavailable
        );
    }

    #[test]
    fn generated_output_sequences_never_reauthorize_after_focus_or_consent_invalidation() {
        #[derive(Clone, Copy, Debug)]
        enum Event {
            Checkpoint,
            Send,
            FocusAway,
            FocusBack,
            RevokeConsent,
            RestoreConsent,
        }

        const BASELINE_MODE: u64 = (1 << 2) | 1;
        const REVOKED_MODE: u64 = 2 << 2;
        const RESTORED_MODE: u64 = (3 << 2) | 1;
        let baseline = OutputBaseline {
            stamp: FocusStamp::default(),
            mode: CompatibilityMode::TrackedFocus,
            mode_state: BASELINE_MODE,
            focus_was_confirmed: true,
        };

        fn check(events: &[Event], baseline: OutputBaseline) {
            const BASELINE_MODE: u64 = (1 << 2) | 1;
            let mut mode_state = BASELINE_MODE;
            let mut focus = FocusCheck::Unchanged;
            let mut authorized_event_pending = false;
            let mut invalidated = false;
            let mut sends_after_invalidation = 0;

            for event in events {
                match event {
                    Event::Checkpoint => {
                        authorized_event_pending =
                            output_baseline_is_valid(baseline, mode_state, focus);
                    }
                    Event::Send => {
                        if authorized_event_pending {
                            authorized_event_pending = false;
                            if invalidated {
                                // One backend event can cross an invalidation
                                // after its checkpoint. The following
                                // checkpoint must refuse continuation.
                                sends_after_invalidation += 1;
                            }
                        }
                    }
                    Event::FocusAway | Event::FocusBack => {
                        // The generation changes on both transitions. Returning
                        // to the same window does not restore the old stamp.
                        focus = FocusCheck::Changed;
                        invalidated = true;
                    }
                    Event::RevokeConsent => {
                        mode_state = REVOKED_MODE;
                        invalidated = true;
                    }
                    Event::RestoreConsent => {
                        // Same effective mode, new generation: this is not the
                        // authorization captured by the in-flight transaction.
                        mode_state = RESTORED_MODE;
                        invalidated = true;
                    }
                }

                if invalidated {
                    assert!(
                        !output_baseline_is_valid(baseline, mode_state, focus),
                        "invalidation must remain terminal for {events:?}"
                    );
                }
            }

            assert!(
                sends_after_invalidation <= 1,
                "at most one already-admitted backend event may cross invalidation: {events:?}"
            );
        }

        let choices = [
            Event::Checkpoint,
            Event::Send,
            Event::FocusAway,
            Event::FocusBack,
            Event::RevokeConsent,
            Event::RestoreConsent,
        ];

        // Exhaust every event ordering through six events. This includes
        // checkpoint/send boundaries on either side of invalidation, repeated
        // focus changes, and consent revoke/restore ABA sequences.
        fn enumerate(events: &mut Vec<Event>, remaining: usize, baseline: OutputBaseline) {
            check(events, baseline);
            if remaining == 0 {
                return;
            }
            for event in [
                Event::Checkpoint,
                Event::Send,
                Event::FocusAway,
                Event::FocusBack,
                Event::RevokeConsent,
                Event::RestoreConsent,
            ] {
                events.push(event);
                enumerate(events, remaining - 1, baseline);
                events.pop();
            }
        }

        for length in 0..=6 {
            enumerate(&mut Vec::new(), length, baseline);
        }

        // Also explore longer histories with a fixed seed. Include a
        // checkpoint/send pair in every case so generated suffixes exercise
        // the production admission predicate even when the random prefix
        // would otherwise contain no output boundary.
        let mut seed = 0x7a31_9d4b_c25e_806f_u64;
        for _case in 0..512 {
            let mut events = vec![Event::Checkpoint, Event::Send];
            for _ in 0..64 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                events.push(choices[(seed as usize) % choices.len()]);
            }
            check(&events, baseline);
        }
    }
}

/// Scheduler model for focus and compatibility consent invalidation during a
/// multi-event output stream. A checkpoint and one backend event are separate
/// operations: a single already-admitted event may cross the invalidation,
/// but the following checkpoint must refuse continuation.
#[cfg(all(test, feature = "loom-tests"))]
mod loom_models {
    use loom::sync::{Arc, Mutex};
    use loom::thread;
    use wayexpand_core::{CompatibilityMode, FocusCheck, FocusStamp};

    use super::{output_baseline_is_valid, OutputBaseline};

    #[derive(Default)]
    struct State {
        focus_sequence: u64,
        mode_state: u64,
        invalidated: bool,
        events_after_invalidation: usize,
    }

    fn model_invalidation(focus_change: bool) {
        loom::model(move || {
            const BASELINE_MODE: u64 = (1 << 2) | 1; // generation 1, tracked focus
            const REVOKED_AND_REGRANTED_MODE: u64 = (3 << 2) | 1;
            const BASELINE_FOCUS_SEQUENCE: u64 = 1;

            let baseline = OutputBaseline {
                stamp: FocusStamp::default(),
                mode: CompatibilityMode::TrackedFocus,
                mode_state: BASELINE_MODE,
                focus_was_confirmed: true,
            };
            let state = Arc::new(Mutex::new(State {
                focus_sequence: BASELINE_FOCUS_SEQUENCE,
                mode_state: BASELINE_MODE,
                ..State::default()
            }));

            let worker_state = Arc::clone(&state);
            let worker = thread::spawn(move || {
                // Each loop is one paced output unit. The backend checks the
                // production admission predicate before each unit.
                for _ in 0..3 {
                    let admitted = {
                        let state = worker_state.lock().unwrap();
                        let focus = if state.focus_sequence == BASELINE_FOCUS_SEQUENCE {
                            FocusCheck::Unchanged
                        } else {
                            FocusCheck::Changed
                        };
                        output_baseline_is_valid(baseline, state.mode_state, focus)
                    };
                    if !admitted {
                        break;
                    }

                    // Model the gap between a successful checkpoint and a
                    // backend event. At most this one event can be in flight
                    // when the tracker or consent changes.
                    thread::yield_now();
                    let mut state = worker_state.lock().unwrap();
                    if state.invalidated {
                        state.events_after_invalidation += 1;
                    }
                }
            });

            let invalidator_state = Arc::clone(&state);
            let invalidator = thread::spawn(move || {
                thread::yield_now();
                let mut state = invalidator_state.lock().unwrap();
                if focus_change {
                    // Sequence changes even if focus returns to the original
                    // application: A→B→A is still a different visit.
                    state.focus_sequence += 1;
                    state.focus_sequence += 1;
                } else {
                    // Revoke then restore the same effective mode. The
                    // generation prevents an ABA from reviving old consent.
                    state.mode_state = REVOKED_AND_REGRANTED_MODE;
                }
                state.invalidated = true;
            });

            worker.join().unwrap();
            invalidator.join().unwrap();

            let state = state.lock().unwrap();
            assert!(state.invalidated);
            assert!(state.events_after_invalidation <= 1);
        });
    }

    #[test]
    fn focus_away_and_back_does_not_reauthorize_output() {
        model_invalidation(true);
    }

    #[test]
    fn revoking_and_restoring_consent_does_not_reauthorize_output() {
        model_invalidation(false);
    }
}
