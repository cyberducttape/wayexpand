//! Insertion assurance: what an output route can promise before it erases.
//!
//! Verified routes (input-method-v2, IBus) check the target's own text before
//! deleting and are bound to the focused text field by their protocol.
//! Keystroke routes (libei, wlroots virtual keyboard) type Backspaces into
//! whatever has focus, so they cannot verify what they erase. This module is
//! the single gate for those routes:
//!
//! - they never erase unless the user enabled `compatibility_mode`;
//! - a focus change detected between matching and typing always refuses;
//! - focus that no tracker can confirm refuses unless the user accepted
//!   `untracked-focus`.
//!
//! Refusals happen before the first keystroke, so the transaction is "not
//! applied" and nothing is retried. Every refusal is counted and published
//! in daemon status so users can see why an expansion did not happen.

use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc, Mutex, OnceLock,
};

use tracing::warn;
use wayexpand_core::{
    CompatibilityMode, FocusCheck, FocusStamp, FocusWitness, InjectorCapabilities, InjectorError,
    KeyEventState, Modifiers, ReplacementGuarantee, TextInjector,
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
            Self::CompatibilityModeOff => {
                "refused: this keystroke route cannot verify what it erases and compatibility_mode is off"
            }
            Self::FocusChanged => {
                "refused: the focused window changed after the trigger was typed"
            }
            Self::FocusUnconfirmed => {
                "refused: no window tracker confirmed the focused window and compatibility_mode is tracked-focus"
            }
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
    /// Keystroke route with the user's consent; erasure is unverified.
    Compatibility,
    /// Keystroke route without consent: typed expansions are refused.
    Blocked,
    /// No output route is connected.
    Unavailable,
}

impl Assurance {
    pub(crate) fn of(capabilities: Option<&InjectorCapabilities>, mode: CompatibilityMode) -> Self {
        match capabilities {
            None => Self::Unavailable,
            Some(capabilities) if verifies_erasure(capabilities) => Self::Verified,
            Some(_) if mode == CompatibilityMode::Off => Self::Blocked,
            Some(_) => Self::Compatibility,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Compatibility => "compatibility",
            Self::Blocked => "blocked",
            Self::Unavailable => "unavailable",
        }
    }
}

fn verifies_erasure(capabilities: &InjectorCapabilities) -> bool {
    capabilities.replacement_guarantee >= ReplacementGuarantee::VerifiedSurroundingText
}

/// The reactor's view of consent and focus, shared with output threads.
#[derive(Default)]
pub(crate) struct GuardState {
    mode: AtomicU8,
    /// Focus identity of the window snapshot the engine currently reflects.
    stamp: Mutex<FocusStamp>,
    witness: Mutex<Option<FocusWitness>>,
    refused_total: AtomicU64,
    last_refusal: AtomicU8,
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
        self.mode.store(code, Ordering::Release);
    }

    pub(crate) fn mode(&self) -> CompatibilityMode {
        match self.mode.load(Ordering::Acquire) {
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

    fn focus(&self) -> FocusCheck {
        let stamp = *self
            .stamp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.witness
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map_or(FocusCheck::Unconfirmed, |witness| witness.check(stamp))
    }

    /// Decide whether an output may start. Outputs that erase nothing, and
    /// routes that verify erasure, are never refused here.
    pub(crate) fn decide(
        &self,
        capabilities: &InjectorCapabilities,
        erases: bool,
    ) -> Result<(), Refusal> {
        if !erases || verifies_erasure(capabilities) {
            return Ok(());
        }
        let mode = self.mode();
        if mode == CompatibilityMode::Off {
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

    fn status_detail(&self) -> &'static str {
        self.inner.status_detail()
    }

    fn admit(&self, erases: bool) -> Result<(), InjectorError> {
        self.inner.admit(erases)?;
        self.guard
            .admit(self.inner.name(), &self.inner.capabilities(), erases)
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
        window_update_channel, ExpansionEngine, ExpansionResult, TransactionOutcome, WindowContext,
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
            snippet_id: String::new(),
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
    fn verified_routes_and_insert_only_output_are_never_refused() {
        let guard = Arc::new(GuardState::default());
        let (mut verified, typed) = guarded(&guard, ReplacementGuarantee::VerifiedSurroundingText);
        assert!(ExpansionEngine::apply(verified.as_mut(), &expansion(":sig")).is_applied());
        assert_eq!(typed.lock().unwrap().len(), 2);

        // Inserting without erasing cannot delete unverified text.
        let (mut keystrokes, typed) = guarded(&guard, ReplacementGuarantee::BestEffort);
        assert!(ExpansionEngine::apply(keystrokes.as_mut(), &expansion("")).is_applied());
        // The default replace() erases an empty trigger, which deletes nothing.
        assert_eq!(*typed.lock().unwrap(), ["erase:", "insert:replacement"]);
        assert_eq!(guard.refused_total(), 0);
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
        assert!(ExpansionEngine::apply(injector.as_mut(), &expansion(":sig")).is_applied());
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

        assert!(ExpansionEngine::apply(injector.as_mut(), &expansion(":sig")).is_applied());
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
            replacement_guarantee: ReplacementGuarantee::Atomic,
            ..InjectorCapabilities::default()
        };
        assert_eq!(
            Assurance::of(Some(&atomic), CompatibilityMode::Off),
            Assurance::Verified
        );
        assert_eq!(
            Assurance::of(Some(&keystrokes), CompatibilityMode::Off),
            Assurance::Blocked
        );
        assert_eq!(
            Assurance::of(Some(&keystrokes), CompatibilityMode::TrackedFocus),
            Assurance::Compatibility
        );
        assert_eq!(
            Assurance::of(None, CompatibilityMode::UntrackedFocus),
            Assurance::Unavailable
        );
    }
}
