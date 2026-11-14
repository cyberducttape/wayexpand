use std::{
    fmt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use std::{fs::OpenOptions, path::Path};

use crate::{InputEvent, Modifiers, WindowContext};

/// The longest replacement a paced keystroke route (one that types each
/// character as a key press) accepts, from `Settings::keystroke_max_chars`.
/// It is process-wide and read at the moment of typing, so the daemon applies
/// a reloaded value to the connected route without reconnecting it: a portal
/// session cannot always be re-established while the process runs.
static KEYSTROKE_MAX_CHARS: AtomicUsize = AtomicUsize::new(crate::DEFAULT_KEYSTROKE_MAX_CHARS);

/// Set the keystroke-typing limit (clamped to at least one character).
pub fn set_keystroke_max_chars(limit: usize) {
    KEYSTROKE_MAX_CHARS.store(limit.max(1), Ordering::Relaxed);
}

/// The current keystroke-typing limit; see [`set_keystroke_max_chars`].
pub fn keystroke_max_chars() -> usize {
    KEYSTROKE_MAX_CHARS.load(Ordering::Relaxed)
}

/// A check that paced output backends run before each bounded output step,
/// such as a synthesized character or direct-text chunk. This lets long
/// streams stop as soon as continuing is no longer safe (the focused window
/// changed, or the daemon abandoned the transaction). A failure means no
/// further output may be sent.
#[derive(Clone, Default)]
pub struct OutputCheckpoint {
    check: Option<Arc<dyn Fn() -> Result<(), String> + Send + Sync>>,
    erased_units: Arc<AtomicUsize>,
    inserted_units: Arc<AtomicUsize>,
    progress_tracked: Arc<std::sync::atomic::AtomicBool>,
}

/// Confirmed output units successfully flushed by the backend. These counts
/// describe protocol actions, not text content or proof that a client painted
/// the text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputProgress {
    pub tracked: bool,
    pub erased_units: usize,
    pub inserted_units: usize,
}

impl OutputCheckpoint {
    pub fn new(check: impl Fn() -> Result<(), String> + Send + Sync + 'static) -> Self {
        Self {
            check: Some(Arc::new(check)),
            ..Self::default()
        }
    }

    pub fn check(&self) -> Result<(), String> {
        self.check.as_ref().map_or(Ok(()), |check| check())
    }

    pub fn reset_progress(&self) {
        self.erased_units.store(0, Ordering::Relaxed);
        self.inserted_units.store(0, Ordering::Relaxed);
        self.progress_tracked
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_erasure(&self, units: usize) {
        self.progress_tracked
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.erased_units.fetch_add(units, Ordering::Relaxed);
    }

    pub fn record_insertion(&self, units: usize) {
        self.progress_tracked
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.inserted_units.fetch_add(units, Ordering::Relaxed);
    }

    pub fn progress(&self) -> OutputProgress {
        OutputProgress {
            tracked: self
                .progress_tracked
                .load(std::sync::atomic::Ordering::Relaxed),
            erased_units: self.erased_units.load(Ordering::Relaxed),
            inserted_units: self.inserted_units.load(Ordering::Relaxed),
        }
    }

    /// Both checks must pass; the first failure wins.
    pub fn and(self, other: Self) -> Self {
        let check = match (self.check, other.check) {
            (None, other) => other,
            (first, None) => first,
            (Some(first), Some(second)) => Some(Arc::new(move || {
                first()?;
                second()
            })
                as Arc<dyn Fn() -> Result<(), String> + Send + Sync>),
        };
        Self {
            check,
            // The receiver owns progress for the composed checkpoint. A
            // wrapper adding a guard should therefore compose the caller's
            // checkpoint on the left.
            erased_units: self.erased_units,
            inserted_units: self.inserted_units,
            progress_tracked: self.progress_tracked,
        }
    }
}

impl fmt::Debug for OutputCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputCheckpoint")
            .field("has_check", &self.check.is_some())
            .field("progress", &self.progress())
            .finish()
    }
}

/// Create a latest-value channel for focused-window snapshots. Window focus
/// changes are state, not a log: if producers outpace consumers, intermediate
/// snapshots are coalesced instead of accumulating an unbounded queue.
///
/// The channel also numbers focus identities (see [`FocusWitness`]) so output
/// can confirm, immediately before typing, that focus has not moved since a
/// trigger was matched, even when that snapshot has not been received yet.
pub fn window_update_channel() -> (WindowUpdateSender, WindowUpdateReceiver) {
    let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
    let state = Arc::new(Mutex::new(WindowChannelState {
        senders: 1,
        ..WindowChannelState::default()
    }));
    (
        WindowUpdateSender {
            state: Arc::clone(&state),
            wake: wake_sender,
        },
        WindowUpdateReceiver {
            state,
            wake: wake_receiver,
        },
    )
}

#[derive(Default)]
struct WindowChannelState {
    latest: Option<Option<WindowContext>>,
    /// Identity of the most recently published snapshot; titles are ignored
    /// because applications change them while the user types.
    identity: Option<(Option<String>, Option<String>)>,
    /// Bumped whenever the published identity changes or the last sender
    /// goes away, never for a title-only update.
    sequence: u64,
    senders: usize,
}

impl WindowChannelState {
    fn stamp(&self) -> FocusStamp {
        FocusStamp {
            sequence: self.sequence,
            exact: self.senders > 0
                && self.identity.as_ref().is_some_and(|(_, instance)| {
                    instance.as_deref().is_some_and(|id| !id.is_empty())
                }),
        }
    }
}

fn lock_state(state: &Mutex<WindowChannelState>) -> std::sync::MutexGuard<'_, WindowChannelState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct WindowUpdateSender {
    state: Arc<Mutex<WindowChannelState>>,
    wake: SyncSender<()>,
}

impl Clone for WindowUpdateSender {
    fn clone(&self) -> Self {
        lock_state(&self.state).senders += 1;
        Self {
            state: Arc::clone(&self.state),
            wake: self.wake.clone(),
        }
    }
}

impl Drop for WindowUpdateSender {
    fn drop(&mut self) {
        let mut state = lock_state(&self.state);
        state.senders = state.senders.saturating_sub(1);
        if state.senders == 0 {
            // A tracker that stops without reporting "no window" (for
            // example a panicked thread) must not leave its last window
            // looking confirmed.
            state.identity = None;
            state.sequence = state.sequence.wrapping_add(1);
        }
    }
}

pub struct WindowUpdateReceiver {
    state: Arc<Mutex<WindowChannelState>>,
    wake: Receiver<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowUpdateSendError;

impl WindowUpdateSender {
    /// Publish the current focus snapshot, replacing any older pending one.
    /// `None` is a real update meaning that no focused window is known.
    pub fn send(&self, window: Option<WindowContext>) -> Result<(), WindowUpdateSendError> {
        {
            let mut state = lock_state(&self.state);
            let identity = window
                .as_ref()
                .map(|window| (window.app_id.clone(), window.instance_id.clone()));
            if identity != state.identity {
                state.identity = identity;
                state.sequence = state.sequence.wrapping_add(1);
            }
            state.latest = Some(window);
        }
        match self.wake.try_send(()) {
            Ok(()) | Err(mpsc::TrySendError::Full(())) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(())) => Err(WindowUpdateSendError),
        }
    }
}

/// The focus identity a snapshot belonged to: compare it with a
/// [`FocusWitness`] later to learn whether focus moved in between.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FocusStamp {
    sequence: u64,
    /// The tracker reported an exact window instance, not only an
    /// application, so "unchanged" really means the same window.
    exact: bool,
}

/// Result of checking a [`FocusStamp`] against the tracker's current state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusCheck {
    /// The same exact window is still focused, as far as the tracker knows.
    Unchanged,
    /// Focus moved, became unknown, or the tracker stopped.
    Changed,
    /// Nothing changed, but the stamp never identified an exact window.
    Unconfirmed,
}

/// Live view of a window-update channel's focus identity, shareable with
/// threads that write output.
#[derive(Clone)]
pub struct FocusWitness {
    state: Arc<Mutex<WindowChannelState>>,
}

impl FocusWitness {
    pub fn check(&self, stamp: FocusStamp) -> FocusCheck {
        let current = lock_state(&self.state).stamp();
        if current.sequence != stamp.sequence {
            FocusCheck::Changed
        } else if stamp.exact && current.exact {
            FocusCheck::Unchanged
        } else {
            FocusCheck::Unconfirmed
        }
    }
}

impl WindowUpdateReceiver {
    fn take_latest(&self) -> Option<(Option<WindowContext>, FocusStamp)> {
        let mut state = lock_state(&self.state);
        let stamp = state.stamp();
        state.latest.take().map(|window| (window, stamp))
    }

    pub fn try_recv(&self) -> Result<Option<WindowContext>, TryRecvError> {
        self.try_recv_stamped().map(|(window, _)| window)
    }

    /// Like `try_recv`, also returning the focus identity of that snapshot,
    /// read atomically with it.
    pub fn try_recv_stamped(&self) -> Result<(Option<WindowContext>, FocusStamp), TryRecvError> {
        loop {
            self.wake.try_recv()?;
            if let Some(update) = self.take_latest() {
                return Ok(update);
            }
            // A concurrent publisher may have placed a wake token just after
            // this receiver consumed the latest snapshot. Consume that stale
            // token and keep looking for a real pending value.
        }
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<WindowContext>, mpsc::RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.wake
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
            if let Some((window, _)) = self.take_latest() {
                return Ok(window);
            }
        }
    }

    pub fn focus_witness(&self) -> FocusWitness {
        FocusWitness {
            state: Arc::clone(&self.state),
        }
    }
}

/// State of a low-level keyboard event sent through a pass-through injector.
/// Keeping this distinct from the text-expansion API is what lets an input
/// source preserve a physical key's complete press/release lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventState {
    Pressed,
    Released,
}

/// Reports which application is currently focused, for `app_filter`-scoped
/// expansions. Unlike `InputSource`, a tracker is polled/subscribed
/// independently of the typing stream -- there is no Wayland protocol that
/// works across compositors for this, so implementations are inherently
/// compositor-specific (see `crates/backend-kwin-window`) and callers should
/// treat every one of them as best-effort.
pub trait WindowTracker {
    fn name(&self) -> &'static str;
    /// Content-free category describing where the latest application identity
    /// came from. Implementations must never return the identity itself.
    fn identity_source(&self) -> WindowIdentitySource {
        WindowIdentitySource::Unavailable
    }
    /// Content-free state explaining whether optional same-user process
    /// identity lookup succeeded. Implementations must return only a closed
    /// category, never a PID, executable path, or process name.
    fn process_identity_state(&self) -> ProcessIdentityState {
        ProcessIdentityState::NotApplicable
    }
    /// Waits up to `timeout` for the focused window to change. Returns
    /// `Ok(None)` if nothing changed before the deadline -- the underlying
    /// notification mechanism (a compositor script/D-Bus callback, for the
    /// only implementation today) has no protocol-level health signal, so a
    /// bounded wait is the only way a caller can tell "not connected" apart
    /// from "connected but nothing happened yet" without risking an
    /// unbounded hang. `Ok(Some(None))` means the change was observed but
    /// could not be resolved to a window (e.g. focus moved to the desktop).
    fn next_window_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<Option<WindowContext>>, WindowTrackerError>;
}

/// Sanitized result of a compositor process-identity lookup. This is separate
/// from [`WindowIdentitySource`]: a compositor may report only a mutable
/// process name after an executable lookup fails.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ProcessIdentityState {
    #[default]
    NotApplicable = 0,
    NotRequested = 1,
    CompositorApplicationId = 2,
    ProcessExecutable = 3,
    ProcessIdUnavailable = 4,
    ProcessMetadataUnavailable = 5,
    DifferentUser = 6,
    ExecutablePermissionDenied = 7,
    ExecutableInvalid = 8,
    ProcessChanged = 9,
    ProcessNameUnavailable = 10,
    ExecutableLinkUnavailable = 11,
    CompositorProcessId = 12,
}

impl ProcessIdentityState {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::NotRequested,
            2 => Self::CompositorApplicationId,
            3 => Self::ProcessExecutable,
            4 => Self::ProcessIdUnavailable,
            5 => Self::ProcessMetadataUnavailable,
            6 => Self::DifferentUser,
            7 => Self::ExecutablePermissionDenied,
            8 => Self::ExecutableInvalid,
            9 => Self::ProcessChanged,
            10 => Self::ProcessNameUnavailable,
            11 => Self::ExecutableLinkUnavailable,
            12 => Self::CompositorProcessId,
            _ => Self::NotApplicable,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotApplicable => "not-applicable",
            Self::NotRequested => "not-requested",
            Self::CompositorApplicationId => "compositor-application-id",
            Self::ProcessExecutable => "process-executable",
            Self::ProcessIdUnavailable => "process-id-unavailable",
            Self::ProcessMetadataUnavailable => "process-metadata-unavailable",
            Self::DifferentUser => "different-user",
            Self::ExecutablePermissionDenied => "executable-permission-denied",
            Self::ExecutableInvalid => "executable-invalid",
            Self::ProcessChanged => "process-changed-during-lookup",
            Self::ProcessNameUnavailable => "process-name-unavailable",
            Self::ExecutableLinkUnavailable => "executable-link-unavailable",
            Self::CompositorProcessId => "compositor-process-id",
        }
    }
}

/// Sanitized provenance for a compositor-provided application identity.
/// Values are deliberately a closed set so diagnostics cannot accidentally
/// publish application IDs, window titles, paths, or other caller content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum WindowIdentitySource {
    #[default]
    Unavailable = 0,
    ResourceClass = 1,
    DesktopFileName = 2,
    ResourceName = 3,
    CompositorIdentityOnly = 4,
    ProcessExecutable = 5,
    ProcessExecutableUnavailable = 6,
    ProcessName = 7,
}

impl WindowIdentitySource {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::ResourceClass,
            2 => Self::DesktopFileName,
            3 => Self::ResourceName,
            4 => Self::CompositorIdentityOnly,
            5 => Self::ProcessExecutable,
            6 => Self::ProcessExecutableUnavailable,
            7 => Self::ProcessName,
            _ => Self::Unavailable,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::ResourceClass => "resource-class",
            Self::DesktopFileName => "desktop-file-name",
            Self::ResourceName => "resource-name",
            Self::CompositorIdentityOnly => "compositor-identity-only",
            Self::ProcessExecutable => "process-executable",
            Self::ProcessExecutableUnavailable => "process-executable-unavailable",
            Self::ProcessName => "process-name",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowTrackerError {
    pub backend: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl fmt::Display for WindowTrackerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.backend, self.message)
    }
}

impl std::error::Error for WindowTrackerError {}

/// Source of normalized input events. A source may be compositor-, portal-,
/// or test-backed; the matcher must not know which.
pub trait InputSource {
    fn name(&self) -> &'static str;
    /// Report the negotiated guarantees of the capture source. New sources
    /// default to the conservative profile so policy cannot assume safety
    /// properties merely because a backend was added to the daemon.
    fn capabilities(&self) -> InputSourceCapabilities {
        InputSourceCapabilities::default()
    }
    fn next_event(&mut self) -> Result<InputEvent, InputSourceError>;
}

/// Guarantees provided by the active keyboard-capture source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputSourceCapabilities {
    /// The source authoritatively reports password/sensitive content focus.
    pub sensitive_focus: bool,
    /// The source can prevent original physical keys reaching the application
    /// until WayExpand has decided whether to pass them through.
    pub exclusive_capture: bool,
    /// The source exposes reliable physical key press/release lifetimes.
    pub reliable_key_state: bool,
    /// Capture can forward unsupported keys through its attached injector.
    pub key_passthrough: bool,
    /// The source observes active external IME/preedit composition.
    pub composition_aware: bool,
    /// The source tracks local XKB dead-key and Compose sequences until
    /// their text is committed or the sequence is cancelled.
    pub local_compose_aware: bool,
    /// The source tracks compositor/runtime keyboard layout changes rather
    /// than relying on a startup-only local layout snapshot.
    pub layout_aware: bool,
}

impl InputSourceCapabilities {
    /// Conservative, non-exclusive capture profile for raw evdev.
    pub const EVDEV: Self = Self {
        sensitive_focus: false,
        exclusive_capture: false,
        reliable_key_state: true,
        key_passthrough: false,
        composition_aware: false,
        local_compose_aware: false,
        layout_aware: false,
    };

    /// Capture profile for the currently shipped input-method-v2 source.
    pub const INPUT_METHOD_V2: Self = Self {
        sensitive_focus: true,
        exclusive_capture: true,
        reliable_key_state: true,
        // This source can only claim pass-through when a compatible injector
        // was explicitly attached to the live session.
        key_passthrough: false,
        composition_aware: false,
        local_compose_aware: true,
        layout_aware: true,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSourceError {
    pub source: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl fmt::Display for InputSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.source, self.message)
    }
}

impl std::error::Error for InputSourceError {}

/// How strongly an injector guarantees that erasing a trigger removes exactly
/// the trigger that caused this expansion, in this target context. Ordered
/// from weakest to strongest so policy can require a minimum.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ReplacementGuarantee {
    /// The injector cannot replace text at all. Also the default, so a
    /// backend that does not declare a level can never satisfy a policy.
    #[default]
    Unsupported,
    /// Erases by sending key events without seeing the target's text; a
    /// focus change or cursor move in between can erase the wrong text.
    BestEffort,
    /// Refuses unless the target's reported surrounding text ends with the
    /// trigger at the cursor; erase and insert are separate steps.
    #[serde(rename = "verified")]
    VerifiedSurroundingText,
    /// Verified like `VerifiedSurroundingText`, and erase plus insert are
    /// staged and applied by one backend protocol transaction. This says
    /// nothing about whether the application ultimately rendered the expected
    /// text; that result needs a separate application-level observation.
    Atomic,
}

/// Admission decision shared by every output backend before it erases text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementAdmission {
    Atomic,
    ExplicitCompatibility,
    Refused,
}

impl ReplacementGuarantee {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::BestEffort => "best-effort",
            Self::VerifiedSurroundingText => "verified",
            Self::Atomic => "atomic",
        }
    }
}

/// Guarantees provided by a text-injection backend.
///
/// These are deliberately capability values rather than backend-name checks.
/// A backend may negotiate a different runtime mode (for example, libei can
/// provide either direct UTF-8 text or a keyboard-layout fallback), so policy
/// must be evaluated against the connected injector's actual contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InjectorCapabilities {
    /// Stable name for the negotiated insertion protocol or mode.
    pub insertion_mode: &'static str,
    /// Maximum number of Unicode scalar values accepted by this mode. Zero
    /// means that the backend does not advertise a mode-specific limit. Only
    /// paced keystroke modes set it, to [`keystroke_max_chars`].
    pub max_text_chars: usize,
    /// Approximate sustained output rate when the mode is paced.
    pub expected_throughput_chars_per_sec: Option<u32>,
    /// The backend can submit trigger deletion and replacement as one
    /// protocol transaction. This does not confirm that the target application
    /// applied or rendered the expected text.
    pub atomic_replace: bool,
    /// How safely the trigger erase is tied to this expansion's trigger.
    pub replacement_guarantee: ReplacementGuarantee,
    /// Every valid Unicode replacement can be represented without depending
    /// on the active keyboard layout.
    pub full_unicode: bool,
    /// The backend can reposition the insertion cursor after committing text.
    pub cursor_reposition: bool,
    /// The backend can preserve unsupported physical key press/release events.
    pub key_passthrough: bool,
}

impl InjectorCapabilities {
    /// Whether the negotiated output route provides the complete atomic
    /// replacement contract. Both declarations must agree; the boolean alone
    /// is not sufficient evidence for policy or runtime admission.
    pub fn has_atomic_replacement_contract(self) -> bool {
        self.atomic_replace && self.replacement_guarantee == ReplacementGuarantee::Atomic
    }

    /// Admit replacement only when the route is atomic or the user explicitly
    /// enabled a compatibility mode for non-atomic output.
    pub fn replacement_admission(
        self,
        compatibility: crate::CompatibilityMode,
    ) -> ReplacementAdmission {
        if self.has_atomic_replacement_contract() {
            ReplacementAdmission::Atomic
        } else if compatibility != crate::CompatibilityMode::Off
            && self.replacement_guarantee != ReplacementGuarantee::Unsupported
        {
            ReplacementAdmission::ExplicitCompatibility
        } else {
            ReplacementAdmission::Refused
        }
    }
}

/// The platform-independent operation required by the expansion engine.
///
/// Requires `Send` so a backend can be shut down by the daemon's lifecycle
/// supervisor without tying the input reactor to backend teardown.
pub trait TextInjector: Send {
    /// Explicitly end the backend lifecycle and release its resources.
    ///
    /// Implementations should cancel protocol/portal work before dropping
    /// their runtime. The default preserves compatibility for simple
    /// backends; the daemon still applies a deadline around this operation.
    fn shutdown(self: Box<Self>) {
        drop(self);
    }
    fn name(&self) -> &'static str;
    /// Report the negotiated guarantees of this connected injector.
    /// Implementations default to the conservative profile so a new backend
    /// cannot accidentally satisfy a security requirement by omission.
    fn capabilities(&self) -> InjectorCapabilities {
        InjectorCapabilities::default()
    }
    /// Whether this route emits replacement text as sequential synthetic key
    /// events. Diagnostics use it to label non-atomic paced output; length is
    /// bounded by the route's `max_text_chars`, not by a duration budget.
    fn uses_paced_synthetic_output(&self) -> bool {
        false
    }
    /// Whether a `replace` error can be reported after the target may already
    /// have received the transaction. Atomic backends that can lose
    /// acknowledgement after submission must return `true`; callers then
    /// treat errors as uncertain and never replay the trigger. The default is
    /// appropriate only when every replace error guarantees that no output
    /// was submitted.
    fn replace_error_may_have_applied(&self) -> bool {
        false
    }
    /// Human-readable negotiated capability detail for diagnostics. Backends
    /// that have no additional runtime mode may use the stable default.
    fn status_detail(&self) -> &'static str {
        ""
    }
    /// Last check before an output transaction's first keystroke. `erases`
    /// says whether the transaction deletes text. A refusal must be a
    /// non-retryable error and guarantees that nothing was written, so the
    /// transaction reports as not applied. The default admits everything;
    /// the daemon wraps keystroke-synthesis backends with its assurance
    /// policy.
    fn admit(&self, _erases: bool) -> Result<(), InjectorError> {
        Ok(())
    }
    /// Validate the replacement payload against current backend state before
    /// the transaction starts. A refusal guarantees that no output was sent.
    /// Stateful backends may refresh compositor state here before validating.
    fn preflight_text(&mut self, _text: &str) -> Result<(), InjectorError> {
        Ok(())
    }
    /// Install a check for backends that type over time. They must call it
    /// before each synthesized character and stop with a non-retryable error
    /// when it fails. Backends that write in one burst may ignore it.
    fn set_output_checkpoint(&mut self, _checkpoint: OutputCheckpoint) {}
    /// Clear and read confirmed output progress for one replacement. A
    /// backend increments the supplied checkpoint after each flushed erase
    /// or insertion unit. Defaults keep atomic/native backends compatible.
    fn reset_output_progress(&mut self) {}
    fn output_progress(&self) -> OutputProgress {
        OutputProgress::default()
    }
    /// Remove the exact trigger text immediately before the cursor.
    ///
    /// Backends may need the original UTF-8 string rather than only its
    /// character count (for example, input-method protocols express deletion
    /// in UTF-8 bytes).
    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError>;
    fn insert(&mut self, text: &str) -> Result<(), InjectorError>;
    /// Replace a trigger atomically when the backend supports it. Simple
    /// backends use the safe erase-then-insert default.
    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        self.erase(trigger)?;
        self.insert(text)
    }
    /// Move the text-insertion cursor left by `count` grapheme clusters, for
    /// a `{{cursor}}` placement marker. Backends synthesize one Left key per
    /// cluster; toolkit cursor behavior may vary. A failure is reported as an
    /// applied-with-cursor-position-failure transaction outcome; callers must
    /// not retry the replacement because its text was already inserted.
    fn move_cursor_left(&mut self, _count: usize) -> Result<(), InjectorError> {
        Ok(())
    }
    /// Inject a keyboard key event by Linux evdev keycode. Used for
    /// pass-through of unsupported keys in input-method-v2. Backends that
    /// cannot synthesize key events must return an error rather than a
    /// silent no-op: callers using an exclusive input grab would otherwise
    /// lose the user's key.
    fn inject_key(&mut self, _keycode: u32) -> Result<(), InjectorError> {
        Err(InjectorError {
            backend: self.name(),
            message: "backend cannot synthesize keyboard key events".into(),
            retryable: false,
        })
    }

    /// Inject a keyboard key event with active modifiers preserved. The
    /// default implementation only accepts an empty modifier set; keyboard
    /// event backends should override this when they can synthesize a full
    /// shortcut such as Ctrl+C or Alt+Left.
    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        if modifiers.ctrl || modifiers.alt || modifiers.shift || modifiers.super_key {
            return Err(InjectorError {
                backend: self.name(),
                message: "backend cannot synthesize modified keyboard shortcuts".into(),
                retryable: false,
            });
        }
        self.inject_key(keycode)
    }

    /// Inject one low-level keyboard event without synthesizing a tap.
    /// Keyboard pass-through sources use this for held keys, repeats, and
    /// modifier combinations. Existing injectors that only support taps keep
    /// the safe default: presses use the legacy operation and releases fail
    /// closed instead of silently leaving the target in an unknown state.
    fn inject_key_event(
        &mut self,
        keycode: u32,
        _modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        match state {
            KeyEventState::Pressed => self.inject_key(keycode),
            KeyEventState::Released => Err(InjectorError {
                backend: self.name(),
                message: "backend cannot synthesize keyboard key releases".into(),
                retryable: false,
            }),
        }
    }
}

impl<T: TextInjector + ?Sized> TextInjector for Box<T> {
    fn shutdown(self: Box<Self>) {
        (*self).shutdown()
    }

    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn status_detail(&self) -> &'static str {
        (**self).status_detail()
    }

    fn capabilities(&self) -> InjectorCapabilities {
        (**self).capabilities()
    }

    fn uses_paced_synthetic_output(&self) -> bool {
        (**self).uses_paced_synthetic_output()
    }

    fn replace_error_may_have_applied(&self) -> bool {
        (**self).replace_error_may_have_applied()
    }

    // Every provided method must be forwarded: a default here would silently
    // replace the boxed backend's own behavior.
    fn admit(&self, erases: bool) -> Result<(), InjectorError> {
        (**self).admit(erases)
    }

    fn preflight_text(&mut self, text: &str) -> Result<(), InjectorError> {
        (**self).preflight_text(text)
    }

    fn set_output_checkpoint(&mut self, checkpoint: OutputCheckpoint) {
        (**self).set_output_checkpoint(checkpoint)
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        (**self).erase(trigger)
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        (**self).insert(text)
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        (**self).replace(trigger, text)
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        (**self).move_cursor_left(count)
    }

    fn inject_key(&mut self, keycode: u32) -> Result<(), InjectorError> {
        (**self).inject_key(keycode)
    }

    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        (**self).inject_key_with_modifiers(keycode, modifiers)
    }

    fn inject_key_event(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        (**self).inject_key_event(keycode, modifiers, state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectorError {
    pub backend: &'static str,
    pub message: String,
    /// Whether recreating the backend may make the operation succeed. This
    /// lets the daemon distinguish a lost compositor/session from invalid
    /// expansion data that must not be retried indefinitely.
    pub retryable: bool,
}

/// Classifies an injector failure for the daemon's recovery policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectorErrorKind {
    /// The backend/session may succeed after it is recreated.
    TransportFailure,
    /// New user input overlapped non-atomic output. The output may be partial,
    /// but the backend itself remains usable and must not be reconnected just
    /// for this transaction outcome.
    InputInterrupted,
    /// This expansion cannot be represented by the current backend. The
    /// input session remains healthy and the expansion should be dropped.
    ExpansionRejected,
    /// The backend implementation itself cannot perform the requested
    /// operation. This is a daemon/backend failure, not bad snippet data.
    FatalBackendFailure,
}

impl InjectorError {
    const INPUT_INTERRUPTED_MESSAGE: &'static str =
        "non-atomic output interrupted by new input; target text may be partial";
    pub const REFUSAL_COMPATIBILITY_MODE_OFF: &'static str =
        "refused: this route cannot verify the trigger at the target; compatibility_mode is off";
    pub const REFUSAL_FOCUS_CHANGED: &'static str =
        "refused: the focused window changed after the trigger was typed";
    pub const REFUSAL_FOCUS_UNCONFIRMED: &'static str =
        "refused: no window tracker confirmed the focused window and compatibility_mode is tracked-focus";

    /// Build the content-free failure used when user input overlaps a
    /// non-atomic replacement. This is an uncertain output result, not a
    /// transport failure: keep the injector, discard this expansion, and do
    /// not replay it.
    pub fn input_interrupted(backend: &'static str) -> Self {
        Self {
            backend,
            message: Self::INPUT_INTERRUPTED_MESSAGE.into(),
            retryable: false,
        }
    }

    /// Content-free message for output checkpoints that stop because new
    /// input arrived while a non-atomic transaction was in progress.
    pub fn input_interrupted_message() -> &'static str {
        Self::INPUT_INTERRUPTED_MESSAGE
    }

    pub fn kind(&self) -> InjectorErrorKind {
        if self.retryable {
            InjectorErrorKind::TransportFailure
        } else if self.message.contains(Self::INPUT_INTERRUPTED_MESSAGE) {
            InjectorErrorKind::InputInterrupted
        } else if self.message.starts_with("backend cannot synthesize") {
            InjectorErrorKind::FatalBackendFailure
        } else {
            InjectorErrorKind::ExpansionRejected
        }
    }

    /// These exact admission errors are emitted by the daemon's output
    /// assurance gate before any output is submitted. Match the complete
    /// messages rather than a broad prefix: backend or compositor error text
    /// must never be mistaken for a proven no-effect refusal. The error itself
    /// carries this classification across serialized worker boundaries.
    pub fn is_pre_output_refusal(&self) -> bool {
        !self.retryable
            && matches!(
                self.message.as_str(),
                Self::REFUSAL_COMPATIBILITY_MODE_OFF
                    | Self::REFUSAL_FOCUS_CHANGED
                    | Self::REFUSAL_FOCUS_UNCONFIRMED
            )
    }

    /// Whether a known pre-output refusal proves that the trigger's original
    /// target context is no longer trustworthy. Unlike a compatibility-mode
    /// refusal, focus refusals must not restore the reserved trigger into the
    /// matcher's current context for a later retry.
    pub fn invalidates_target_context(&self) -> bool {
        !self.retryable
            && matches!(
                self.message.as_str(),
                Self::REFUSAL_FOCUS_CHANGED | Self::REFUSAL_FOCUS_UNCONFIRMED
            )
    }
}

impl fmt::Display for InjectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.backend, self.message)
    }
}

impl std::error::Error for InjectorError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    InputMethodV2,
    Evdev,
    Libei,
    WlrootsVirtualKeyboard,
    Uinput,
    Clipboard,
    WindowTracker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendState {
    Implemented,
    Available,
    Unavailable,
    NotImplemented,
    RequiresPermission,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InputMethodV2 => "input-method-v2",
            Self::Evdev => "evdev",
            Self::Libei => "libei",
            Self::WlrootsVirtualKeyboard => "wlroots-virtual-keyboard",
            Self::Uinput => "uinput",
            Self::Clipboard => "clipboard",
            Self::WindowTracker => "window-tracker",
        };
        f.write_str(name)
    }
}

impl BackendKind {
    /// Canonical organization-policy identity for selectable output
    /// backends. Input sources and non-selectable diagnostic entries return
    /// `None` because they do not independently determine daemon behavior.
    pub fn policy_name(self) -> Option<&'static str> {
        match self {
            Self::InputMethodV2 => Some("input-method-v2"),
            Self::Libei => Some("libei"),
            Self::WlrootsVirtualKeyboard => Some("wlroots"),
            Self::Evdev | Self::Uinput | Self::Clipboard | Self::WindowTracker => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendStatus {
    pub kind: BackendKind,
    pub state: BackendState,
    pub detail: String,
}

impl BackendStatus {
    pub fn implementation(&self) -> &'static str {
        match self.state {
            BackendState::NotImplemented => "NotImplemented",
            _ => "Implemented",
        }
    }

    pub fn availability(&self) -> &'static str {
        match self.state {
            BackendState::Unavailable | BackendState::RequiresPermission => "Unavailable",
            BackendState::Implemented => "Unknown",
            BackendState::Available => "Detected",
            BackendState::NotImplemented => "Unknown",
        }
    }

    pub fn permission(&self) -> &'static str {
        match self.state {
            BackendState::RequiresPermission => "Required",
            BackendState::Available
                if matches!(
                    self.kind,
                    BackendKind::Evdev | BackendKind::Libei | BackendKind::Uinput
                ) =>
            {
                "Granted"
            }
            BackendState::NotImplemented
            | BackendState::Unavailable
            | BackendState::Implemented
            | BackendState::Available => "NotApplicable",
        }
    }
}

/// Report environment-level facts without claiming protocol support that has
/// not yet been negotiated with a compositor.
pub fn discover_backends() -> Vec<BackendStatus> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let (uinput_state, uinput_detail) = discover_uinput();
    vec![
        BackendStatus {
            kind: BackendKind::InputMethodV2,
            state: BackendState::Implemented,
            detail: if wayland {
                "probe implemented; exclusive-grab integration is opt-in"
            } else {
                "Wayland session not detected"
            }
            .into(),
        },
        BackendStatus {
            kind: BackendKind::Libei,
            state: BackendState::Implemented,
            detail: if std::env::var_os("LIBEI_SOCKET").is_some() {
                "implemented; direct EIS socket configured (explicit backend only); prefers ei_text and reports live fallback mode in daemon status"
            } else if wayland {
                "implemented; portal connection requires explicit opt-in; prefers ei_text and reports live fallback mode in daemon status"
            } else {
                "Wayland session not detected"
            }
            .into(),
        },
        BackendStatus {
            kind: BackendKind::WlrootsVirtualKeyboard,
            state: BackendState::Implemented,
            detail: "output implemented; requires compositor protocol probe".into(),
        },
        {
            let (state, detail) = discover_evdev();
            BackendStatus {
                kind: BackendKind::Evdev,
                state,
                detail,
            }
        },
        BackendStatus {
            kind: BackendKind::Uinput,
            state: uinput_state,
            detail: uinput_detail,
        },
        BackendStatus {
            kind: BackendKind::Clipboard,
            state: BackendState::NotImplemented,
            detail: "paste-based clipboard injection was removed from the daemon to prevent silent XWayland window targeting; use an explicit backend (libei/wlroots) instead".into(),
        },
        {
            let (state, detail) = discover_window_tracker(wayland);
            BackendStatus {
                kind: BackendKind::WindowTracker,
                state,
                detail,
            }
        },
    ]
}

/// Environment-level guess at whether `app_filter`-scoped expansions can
/// work here. No Wayland protocol reports focused-window identity across
/// compositors, so this can only name which compositor-specific bridge (if
/// any) applies; live availability still depends on that bridge actually
/// connecting (KWin's scripting D-Bus interface, a wlroots
/// foreign-toplevel-management protocol, and so on).
fn discover_window_tracker(wayland: bool) -> (BackendState, String) {
    if !wayland {
        return (
            BackendState::Unavailable,
            "Wayland session not detected".into(),
        );
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    discover_window_tracker_for_desktop(&desktop)
}

fn discover_window_tracker_for_desktop(desktop: &str) -> (BackendState, String) {
    let gnome = desktop
        .to_lowercase()
        .split(':')
        .any(|part| part == "gnome");
    if gnome {
        (
            BackendState::Implemented,
            "GNOME detected; uses the WayExpand GNOME Shell extension (runtime-gated on GNOME Shell 46, 50, and 51), because \
             Mutter exposes no window-listing Wayland protocol and limits \
             org.gnome.Shell.Introspect to allowlisted callers; `wayexpand setup` installs and \
             enables it"
                .into(),
        )
    } else if desktop.to_lowercase().contains("kde") {
        (
            BackendState::Implemented,
            "KDE Plasma detected; uses KWin's scripting D-Bus interface (org.kde.kwin.Scripting), \
             the same mechanism tools like kdotool rely on since KWin exposes no window-listing \
             Wayland protocol"
                .into(),
        )
    } else {
        (
            BackendState::NotImplemented,
            format!(
                "app_filter-scoped expansions need a compositor-specific window tracker; \
                 KDE Plasma (KWin) and GNOME (WayExpand Shell extension) are implemented \
                 (detected desktop: {})",
                if desktop.is_empty() {
                    "unknown"
                } else {
                    &desktop
                }
            ),
        )
    }
}

/// A lightweight, dependency-free probe mirroring what
/// `wayexpand-backend-evdev` would find; kept here (rather than depending on
/// that backend crate from core) so core stays free of backend-specific
/// device access, matching how it never depends on wayland-client either.
fn discover_evdev() -> (BackendState, String) {
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return (
            BackendState::Unavailable,
            "/dev/input is unavailable".into(),
        );
    };
    let mut total = 0usize;
    let mut readable = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("event") {
            continue;
        }
        total += 1;
        if OpenOptions::new().read(true).open(entry.path()).is_ok() {
            readable += 1;
        }
    }
    if total == 0 {
        (
            BackendState::Unavailable,
            "no /dev/input/event* device nodes found".into(),
        )
    } else if readable == 0 {
        (
            BackendState::RequiresPermission,
            format!(
                "{total} input device(s) exist but none are readable by this user; \
                 install/repair active-seat logind/uaccess ACLs first (the broader legacy \
                 `input` group is an explicit fallback) and log in again. If this still fails after \
                 logging out and back in, your systemd --user manager likely did not restart and \
                 is still running with your old group list -- run `loginctl terminate-user \
                 $USER` (ends all your sessions) or reboot, then retry"
            ),
        )
    } else {
        (
            BackendState::Implemented,
            format!("{readable}/{total} input device(s) readable; evdev will verify keyboard capability at connection time"),
        )
    }
}

fn discover_uinput() -> (BackendState, String) {
    let path = Path::new("/dev/uinput");
    match OpenOptions::new().write(true).open(path) {
        Ok(_) => (
            BackendState::NotImplemented,
            "/dev/uinput is writable, but the uinput backend is not implemented".into(),
        ),
        Err(error) if path.exists() => (
            BackendState::RequiresPermission,
            format!("/dev/uinput exists but is not writable: {error}"),
        ),
        Err(_) => (
            BackendState::Unavailable,
            "/dev/uinput is unavailable".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_identity_source_codes_are_closed_and_content_free() {
        for (source, code, label) in [
            (WindowIdentitySource::Unavailable, 0, "unavailable"),
            (WindowIdentitySource::ResourceClass, 1, "resource-class"),
            (
                WindowIdentitySource::DesktopFileName,
                2,
                "desktop-file-name",
            ),
            (WindowIdentitySource::ResourceName, 3, "resource-name"),
            (
                WindowIdentitySource::CompositorIdentityOnly,
                4,
                "compositor-identity-only",
            ),
            (
                WindowIdentitySource::ProcessExecutable,
                5,
                "process-executable",
            ),
            (
                WindowIdentitySource::ProcessExecutableUnavailable,
                6,
                "process-executable-unavailable",
            ),
        ] {
            assert_eq!(source.code(), code);
            assert_eq!(WindowIdentitySource::from_code(code), source);
            assert_eq!(source.as_str(), label);
        }
        assert_eq!(
            WindowIdentitySource::from_code(u8::MAX),
            WindowIdentitySource::Unavailable
        );
    }

    #[test]
    fn gnome_window_tracking_diagnostic_only_names_runtime_gated_shell_majors() {
        let (state, detail) = discover_window_tracker_for_desktop("GNOME");
        assert_eq!(state, BackendState::Implemented);
        assert!(detail.contains("GNOME Shell 46, 50, and 51"));
        assert!(!detail.contains("GNOME 45+"));
    }

    #[test]
    fn only_known_assurance_refusals_prove_that_output_never_started() {
        let refusal = InjectorError {
            backend: "libei",
            message:
                "refused: this route cannot verify the trigger at the target; compatibility_mode is off"
                    .into(),
            retryable: false,
        };
        assert!(refusal.is_pre_output_refusal());

        for message in [
            "refused: compositor declined the request",
            "permission denied",
        ] {
            let backend_error = InjectorError {
                backend: "libei",
                message: message.into(),
                retryable: false,
            };
            assert!(!backend_error.is_pre_output_refusal(), "{message}");
        }

        let retryable_refusal = InjectorError {
            backend: "libei",
            message: refusal.message,
            retryable: true,
        };
        assert!(!retryable_refusal.is_pre_output_refusal());
    }

    #[test]
    fn only_focus_assurance_refusals_invalidate_the_target_context() {
        for message in [
            "refused: the focused window changed after the trigger was typed",
            "refused: no window tracker confirmed the focused window and compatibility_mode is tracked-focus",
        ] {
            let refusal = InjectorError {
                backend: "libei",
                message: message.into(),
                retryable: false,
            };
            assert!(refusal.invalidates_target_context(), "{message}");
            assert!(refusal.is_pre_output_refusal(), "{message}");
        }

        let compatibility_refusal = InjectorError {
            backend: "libei",
            message:
                "refused: this route cannot verify the trigger at the target; compatibility_mode is off"
                    .into(),
            retryable: false,
        };
        assert!(!compatibility_refusal.invalidates_target_context());

        let retryable_focus_error = InjectorError {
            backend: "libei",
            message: "refused: the focused window changed after the trigger was typed".into(),
            retryable: true,
        };
        assert!(!retryable_focus_error.invalidates_target_context());
    }

    fn focused(app: &str, instance: Option<&str>, title: &str) -> Option<WindowContext> {
        Some(WindowContext {
            app_id: Some(app.into()),
            title: Some(title.into()),
            instance_id: instance.map(Into::into),
        })
    }

    #[test]
    fn boxed_injectors_forward_admission_and_checkpoints() {
        #[derive(Default)]
        struct Refusing {
            checkpoint: Option<OutputCheckpoint>,
        }
        impl TextInjector for Refusing {
            fn name(&self) -> &'static str {
                "refusing"
            }
            fn admit(&self, _erases: bool) -> Result<(), InjectorError> {
                Err(InjectorError {
                    backend: "refusing",
                    message: "refused".into(),
                    retryable: false,
                })
            }
            fn set_output_checkpoint(&mut self, checkpoint: OutputCheckpoint) {
                self.checkpoint = Some(checkpoint);
            }
            fn erase(&mut self, _trigger: &str) -> Result<(), InjectorError> {
                Ok(())
            }
            fn insert(&mut self, _text: &str) -> Result<(), InjectorError> {
                Ok(())
            }
        }

        let mut boxed: Box<Box<dyn TextInjector>> =
            Box::new(Box::new(Refusing::default()) as Box<dyn TextInjector>);
        // Called through the Box impl, not the trait object.
        assert!(<Box<Box<dyn TextInjector>> as TextInjector>::admit(&boxed, true).is_err());
        <Box<Box<dyn TextInjector>> as TextInjector>::set_output_checkpoint(
            &mut boxed,
            OutputCheckpoint::new(|| Err("stop".into())),
        );
        assert!(<Box<Box<dyn TextInjector>> as TextInjector>::admit(&boxed, false).is_err());
    }

    #[test]
    fn output_checkpoints_combine_and_report_the_first_failure() {
        let pass = OutputCheckpoint::new(|| Ok(()));
        let fail = OutputCheckpoint::new(|| Err("focus".into()));
        assert_eq!(OutputCheckpoint::default().check(), Ok(()));
        assert_eq!(
            pass.clone().and(OutputCheckpoint::default()).check(),
            Ok(())
        );
        assert_eq!(pass.and(fail.clone()).check(), Err("focus".into()));
        assert_eq!(
            fail.and(OutputCheckpoint::new(|| Err("cancelled".into())))
                .check(),
            Err("focus".into())
        );
    }

    #[test]
    fn composed_checkpoints_preserve_the_callers_output_progress() {
        let transaction_progress = OutputCheckpoint::default();
        let target_guard = OutputCheckpoint::new(|| Ok(()));
        let combined = transaction_progress.clone().and(target_guard);

        combined.record_erasure(3);
        combined.record_insertion(7);

        assert_eq!(
            transaction_progress.progress(),
            OutputProgress {
                tracked: true,
                erased_units: 3,
                inserted_units: 7,
            }
        );
    }

    #[test]
    fn interruption_kind_survives_backend_error_context() {
        let message = format!(
            "typing stopped before the end of the text: {}",
            InjectorError::input_interrupted_message()
        );
        let error = InjectorError {
            backend: "test",
            message,
            retryable: false,
        };
        assert_eq!(error.kind(), InjectorErrorKind::InputInterrupted);
    }

    #[test]
    fn focus_witness_ignores_titles_and_detects_window_changes() {
        let (sender, receiver) = window_update_channel();
        let witness = receiver.focus_witness();
        sender.send(focused("editor", Some("w1"), "draft")).unwrap();
        let (_, stamp) = receiver.try_recv_stamped().unwrap();
        assert_eq!(witness.check(stamp), FocusCheck::Unchanged);

        // Editors retitle themselves while the user types.
        sender
            .send(focused("editor", Some("w1"), "draft *"))
            .unwrap();
        assert_eq!(witness.check(stamp), FocusCheck::Unchanged);

        // Detected before the reactor receives the snapshot.
        sender.send(focused("editor", Some("w2"), "draft")).unwrap();
        assert_eq!(witness.check(stamp), FocusCheck::Changed);
        let (_, stamp) = receiver.try_recv_stamped().unwrap();
        assert_eq!(witness.check(stamp), FocusCheck::Unchanged);

        sender.send(None).unwrap();
        assert_eq!(witness.check(stamp), FocusCheck::Changed);
    }

    #[test]
    fn focus_witness_never_confirms_app_only_identity_or_a_stopped_tracker() {
        let (sender, receiver) = window_update_channel();
        let witness = receiver.focus_witness();
        assert_eq!(
            witness.check(FocusStamp::default()),
            FocusCheck::Unconfirmed
        );

        sender.send(focused("terminal", None, "shell")).unwrap();
        let (_, stamp) = receiver.try_recv_stamped().unwrap();
        assert_eq!(witness.check(stamp), FocusCheck::Unconfirmed);
        sender.send(focused("browser", None, "page")).unwrap();
        assert_eq!(witness.check(stamp), FocusCheck::Changed);

        sender.send(focused("editor", Some("w1"), "draft")).unwrap();
        let (_, stamp) = receiver.try_recv_stamped().unwrap();
        let clone = sender.clone();
        drop(sender);
        assert_eq!(witness.check(stamp), FocusCheck::Unchanged);
        // The last tracker handle going away is a change, even unannounced.
        drop(clone);
        assert_eq!(witness.check(stamp), FocusCheck::Changed);
    }

    #[test]
    fn window_updates_coalesce_to_the_latest_snapshot() {
        let (sender, receiver) = window_update_channel();
        sender
            .send(Some(WindowContext {
                app_id: Some("old.app".into()),
                title: None,
                instance_id: None,
            }))
            .unwrap();
        sender
            .send(Some(WindowContext {
                app_id: Some("new.app".into()),
                title: None,
                instance_id: None,
            }))
            .unwrap();

        assert_eq!(
            receiver
                .try_recv()
                .unwrap()
                .and_then(|window| window.app_id),
            Some("new.app".into())
        );
        assert_eq!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty));
    }

    #[test]
    fn window_updates_preserve_unknown_focus_and_disconnect_state() {
        let (sender, receiver) = window_update_channel();
        sender
            .send(Some(WindowContext {
                app_id: Some("possibly-stale.app".into()),
                title: None,
                instance_id: None,
            }))
            .unwrap();
        sender.send(None).unwrap();
        drop(sender);

        assert_eq!(receiver.try_recv(), Ok(None));
        assert_eq!(receiver.try_recv(), Err(mpsc::TryRecvError::Disconnected));
    }

    #[test]
    fn window_updates_remain_bounded_under_a_producer_burst() {
        let (sender, receiver) = window_update_channel();
        let producer = std::thread::spawn(move || {
            for index in 0..10_000 {
                sender
                    .send(Some(WindowContext {
                        app_id: Some(index.to_string()),
                        title: None,
                        instance_id: None,
                    }))
                    .unwrap();
            }
        });
        producer.join().unwrap();

        assert_eq!(
            receiver
                .try_recv()
                .unwrap()
                .and_then(|window| window.app_id),
            Some("9999".into())
        );
        assert_eq!(receiver.try_recv(), Err(mpsc::TryRecvError::Disconnected));
    }

    #[test]
    fn input_source_profiles_keep_security_guarantees_separate() {
        fn assert_profile(
            profile: InputSourceCapabilities,
            sensitive_focus: bool,
            exclusive_capture: bool,
            key_passthrough: bool,
            composition_aware: bool,
            local_compose_aware: bool,
            layout_aware: bool,
        ) {
            assert_eq!(profile.sensitive_focus, sensitive_focus);
            assert_eq!(profile.exclusive_capture, exclusive_capture);
            assert_eq!(profile.key_passthrough, key_passthrough);
            assert_eq!(profile.composition_aware, composition_aware);
            assert_eq!(profile.local_compose_aware, local_compose_aware);
            assert_eq!(profile.layout_aware, layout_aware);
        }

        assert_profile(
            InputSourceCapabilities::EVDEV,
            false,
            false,
            false,
            false,
            false,
            false,
        );
        assert_profile(
            InputSourceCapabilities::INPUT_METHOD_V2,
            true,
            true,
            false,
            false,
            true,
            true,
        );
        assert_profile(
            InputSourceCapabilities::default(),
            false,
            false,
            false,
            false,
            false,
            false,
        );
    }

    #[test]
    fn unimplemented_backends_are_not_reported_as_available() {
        let statuses = discover_backends();
        let uinput = statuses
            .iter()
            .find(|status| status.kind == BackendKind::Uinput)
            .expect("uinput status is always reported");
        let clipboard = statuses
            .iter()
            .find(|status| status.kind == BackendKind::Clipboard)
            .expect("clipboard status is always reported");

        assert_ne!(uinput.state, BackendState::Available);
        assert_eq!(clipboard.state, BackendState::NotImplemented);
    }

    #[test]
    fn backend_names_are_stable_for_operator_output() {
        assert_eq!(BackendKind::InputMethodV2.to_string(), "input-method-v2");
        assert_eq!(BackendKind::Libei.to_string(), "libei");
        assert_eq!(
            BackendKind::WlrootsVirtualKeyboard.to_string(),
            "wlroots-virtual-keyboard"
        );
        assert_eq!(BackendKind::Uinput.to_string(), "uinput");
        assert_eq!(BackendKind::Clipboard.to_string(), "clipboard");
    }

    #[test]
    fn diagnostic_dimensions_use_documented_values() {
        for state in [
            BackendState::Implemented,
            BackendState::Available,
            BackendState::Unavailable,
            BackendState::NotImplemented,
            BackendState::RequiresPermission,
        ] {
            let status = BackendStatus {
                kind: BackendKind::Clipboard,
                state,
                detail: String::new(),
            };
            assert!(matches!(
                status.implementation(),
                "Implemented" | "NotImplemented"
            ));
            assert!(matches!(
                status.availability(),
                "Detected" | "Unavailable" | "Unknown"
            ));
            assert!(matches!(
                status.permission(),
                "Granted" | "Required" | "NotApplicable"
            ));
        }
    }

    #[test]
    fn detected_protocols_do_not_claim_permission_grants() {
        for kind in [
            BackendKind::InputMethodV2,
            BackendKind::WlrootsVirtualKeyboard,
            BackendKind::WindowTracker,
        ] {
            let status = BackendStatus {
                kind,
                state: BackendState::Available,
                detail: String::new(),
            };
            assert_eq!(status.permission(), "NotApplicable", "{kind}");
        }
    }

    #[test]
    fn available_permission_bearing_routes_report_a_grant() {
        for kind in [BackendKind::Evdev, BackendKind::Libei, BackendKind::Uinput] {
            let status = BackendStatus {
                kind,
                state: BackendState::Available,
                detail: String::new(),
            };
            assert_eq!(status.permission(), "Granted", "{kind}");
        }
    }

    #[test]
    fn non_atomic_replacement_requires_explicit_compatibility_even_when_verified() {
        let verified = InjectorCapabilities {
            replacement_guarantee: ReplacementGuarantee::VerifiedSurroundingText,
            ..InjectorCapabilities::default()
        };
        assert_eq!(
            verified.replacement_admission(crate::CompatibilityMode::Off),
            ReplacementAdmission::Refused
        );
        assert_eq!(
            verified.replacement_admission(crate::CompatibilityMode::TrackedFocus),
            ReplacementAdmission::ExplicitCompatibility
        );

        let atomic = InjectorCapabilities {
            atomic_replace: true,
            replacement_guarantee: ReplacementGuarantee::Atomic,
            ..InjectorCapabilities::default()
        };
        assert_eq!(
            atomic.replacement_admission(crate::CompatibilityMode::Off),
            ReplacementAdmission::Atomic
        );
        assert_eq!(
            InjectorCapabilities::default()
                .replacement_admission(crate::CompatibilityMode::UntrackedFocus),
            ReplacementAdmission::Refused
        );

        let inconsistent = InjectorCapabilities {
            atomic_replace: true,
            replacement_guarantee: ReplacementGuarantee::VerifiedSurroundingText,
            ..InjectorCapabilities::default()
        };
        assert!(!inconsistent.has_atomic_replacement_contract());
        assert_eq!(
            inconsistent.replacement_admission(crate::CompatibilityMode::Off),
            ReplacementAdmission::Refused
        );
    }
}
