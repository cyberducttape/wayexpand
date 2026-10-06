use std::fmt;
use std::time::Duration;
use std::{fs::OpenOptions, path::Path};

use crate::{InputEvent, Modifiers, WindowContext};

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
    /// Verified like `VerifiedSurroundingText`, and erase plus insert are one
    /// protocol transaction with no visible intermediate state.
    Atomic,
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
    /// means that the backend does not advertise a mode-specific limit.
    pub max_text_chars: usize,
    /// Approximate sustained output rate when the mode is paced.
    pub expected_throughput_chars_per_sec: Option<u32>,
    /// The backend can replace the trigger and replacement as one protocol
    /// transaction, without an externally visible erase-then-insert gap.
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
    /// Human-readable negotiated capability detail for diagnostics. Backends
    /// that have no additional runtime mode may use the stable default.
    fn status_detail(&self) -> &'static str {
        ""
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
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn status_detail(&self) -> &'static str {
        (**self).status_detail()
    }

    fn capabilities(&self) -> InjectorCapabilities {
        (**self).capabilities()
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
    /// This expansion cannot be represented by the current backend. The
    /// input session remains healthy and the expansion should be dropped.
    ExpansionRejected,
    /// The backend implementation itself cannot perform the requested
    /// operation. This is a daemon/backend failure, not bad snippet data.
    FatalBackendFailure,
}

impl InjectorError {
    pub fn kind(&self) -> InjectorErrorKind {
        if self.retryable {
            InjectorErrorKind::TransportFailure
        } else if self.message.starts_with("backend cannot synthesize") {
            InjectorErrorKind::FatalBackendFailure
        } else {
            InjectorErrorKind::ExpansionRejected
        }
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
            BackendState::NotImplemented
            | BackendState::Unavailable
            | BackendState::Implemented => "NotApplicable",
            BackendState::Available => "Granted",
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
    if desktop.to_lowercase().contains("kde") {
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
                 only KDE Plasma (KWin) is implemented so far (detected desktop: {})",
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
}
