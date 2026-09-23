use std::fmt;
use std::time::Duration;
use std::{fs::OpenOptions, path::Path};

use crate::{InputEvent, WindowContext};

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
    fn next_event(&mut self) -> Result<InputEvent, InputSourceError>;
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

/// The platform-independent operation required by the expansion engine.
///
/// Requires `Send` so that a `Box<dyn TextInjector>` can be handed off to a
/// detached thread on shutdown (see the daemon's main loop): the libei
/// backend's teardown can hang against some portal implementations, and
/// moving that drop off the main thread is what lets shutdown proceed
/// without waiting on it.
pub trait TextInjector: Send {
    fn name(&self) -> &'static str;
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
    /// Move the text-insertion cursor left by `count` characters, for a
    /// `{{cursor}}` placement marker. Always best-effort: the default no-op
    /// implementation is a valid choice for a backend that cannot or does
    /// not synthesize a Left key, since failing to reposition the cursor
    /// does not mean the expansion itself failed -- the replacement text
    /// was already inserted successfully by `replace`/`insert`.
    fn move_cursor_left(&mut self, _count: usize) -> Result<(), InjectorError> {
        Ok(())
    }
}

impl<T: TextInjector + ?Sized> TextInjector for Box<T> {
    fn name(&self) -> &'static str {
        (**self).name()
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
                "implemented; direct EIS socket configured (explicit backend only)"
            } else if wayland {
                "implemented; portal connection requires explicit opt-in"
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
                 add your user to the `input` group and log in again. If this still fails after \
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
