use std::fmt;
use std::{fs::OpenOptions, path::Path};

use crate::InputEvent;

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
pub trait TextInjector {
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
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendStatus {
    pub kind: BackendKind,
    pub state: BackendState,
    pub detail: String,
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
            detail: "clipboard mutation is not implemented".into(),
        },
    ]
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
            format!("{readable}/{total} input device(s) readable; keyboard filtering happens at connect time"),
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
}
