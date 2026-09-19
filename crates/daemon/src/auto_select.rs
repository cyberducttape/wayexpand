/// Intelligent backend auto-selection strategy.
///
/// Implements capability-based selection with safety-first fallback:
/// 1. Probe for actual protocol and device availability (not desktop name)
/// 2. Prefer safer options (input-method-v2 over evdev for password field safety)
/// 3. Fall back to conservative options (stdin) when capabilities unavailable
/// 4. Desktop environment strings influence preference ordering only
use std::env;
use std::fs::File;
use tracing::{debug, warn};
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_wlroots::WlrootsInjector;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectorBackend {
    None,
    Libei,
    Wlroots,
}

impl InjectorBackend {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Libei => "libei",
            Self::Wlroots => "wlroots",
        }
    }
}

/// A source/output combination that the daemon can actually start.
///
/// Input-method-v2 is deliberately its own variant because it captures and
/// injects text through the same protocol object. It cannot be combined with
/// a separate output injector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBackendPair {
    InputMethod,
    Evdev(InjectorBackend),
    Stdin(InjectorBackend),
}

impl ResolvedBackendPair {
    pub fn source(self) -> &'static str {
        match self {
            Self::InputMethod => "input-method",
            Self::Evdev(_) => "evdev",
            Self::Stdin(_) => "stdin",
        }
    }

    pub fn backend(self) -> &'static str {
        match self {
            Self::InputMethod => "none",
            Self::Evdev(backend) | Self::Stdin(backend) => backend.name(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSelectionError {
    UnknownSource(String),
    UnknownBackend(String),
    Incompatible { source: String, backend: String },
}

impl std::fmt::Display for BackendSelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSource(source) => write!(
                formatter,
                "unknown source {source:?}; expected stdin, input-method, or evdev"
            ),
            Self::UnknownBackend(backend) => write!(
                formatter,
                "unknown backend {backend:?}; expected none, wlroots, or libei"
            ),
            Self::Incompatible { source, backend } => write!(
                formatter,
                "source {source:?} cannot be combined with output backend {backend:?}"
            ),
        }
    }
}

impl std::error::Error for BackendSelectionError {}

#[derive(Debug, Clone)]
pub struct BackendSelection {
    pub pair: ResolvedBackendPair,
    /// Why this selection was made (for logging/documentation)
    pub reason: String,
}

/// Probed capabilities for the current session
#[derive(Debug, Clone, Default)]
struct Capabilities {
    has_input_method_v2: bool,
    #[allow(dead_code)]
    has_virtual_keyboard: bool,
    has_direct_libei_socket: bool,
    has_dev_input: bool,
    #[allow(dead_code)]
    has_window_tracker: bool,
}

/// Probe for actual capabilities available in the session
fn probe_capabilities() -> Capabilities {
    let is_wayland =
        env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("WAYLAND_SOCKET").is_some();
    let has_dev_input = has_readable_input_device();

    let has_input_method_v2 = is_wayland && InputMethodSource::probe().is_ok();
    let has_virtual_keyboard = is_wayland && WlrootsInjector::probe().is_ok();
    // Portal probing would show a consent dialog. An existing direct EIS
    // socket is safe to recognize; desktop portal availability remains an
    // explicit startup decision and is reported as such by `doctor`.
    let has_direct_libei_socket = env::var_os("LIBEI_SOCKET").is_some();
    let has_window_tracker = false;

    let caps = Capabilities {
        has_input_method_v2,
        has_virtual_keyboard,
        has_direct_libei_socket,
        has_dev_input,
        has_window_tracker,
    };

    debug!("probed capabilities: {:?}", caps);
    caps
}

fn has_readable_input_device() -> bool {
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return false;
    };

    entries.flatten().any(|entry| {
        entry.file_name().to_string_lossy().starts_with("event") && File::open(entry.path()).is_ok()
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Compositor {
    KdePlasma,
    Sway,
    #[allow(dead_code)]
    Hyprland,
    #[allow(dead_code)]
    River,
    Gnome,
    X11,
    Unknown,
}

impl Compositor {
    fn detect() -> Self {
        // Check XDG_CURRENT_DESKTOP first
        if let Ok(desktop) = env::var("XDG_CURRENT_DESKTOP") {
            match desktop.as_str() {
                "KDE" => return Compositor::KdePlasma,
                "GNOME" => return Compositor::Gnome,
                _ => {}
            }
        }

        // Check WAYLAND_DISPLAY and WAYLAND_SOCKET for wlroots compositors
        let is_wayland =
            env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("WAYLAND_SOCKET").is_some();

        if is_wayland {
            // Try to detect specific wlroots compositor
            if let Ok(session_type) = env::var("XDG_SESSION_TYPE") {
                if session_type == "wayland" {
                    // Additional detection could go here (socket path, etc.)
                    // For now, assume generic wlroots
                    return Compositor::Sway; // Sway is most common, represents all wlroots
                }
            }
        }

        // Check for X11
        if env::var_os("DISPLAY").is_some() {
            return Compositor::X11;
        }

        Compositor::Unknown
    }
}

/// Pure policy: select backend given capabilities.
///
/// Decoupled from probing so logic is deterministically testable.
/// Takes explicit capabilities to avoid environment dependencies in tests.
fn select_backend(
    capabilities: &Capabilities,
    _compositor: Compositor,
    explicit_source: Option<&str>,
    explicit_backend: Option<&str>,
) -> Result<BackendSelection, BackendSelectionError> {
    // Resolve an explicit source/backend pair first. Once this returns, the
    // daemon receives a typed pair and never has to combine the two options.
    if let (Some(source), Some(backend)) = (explicit_source, explicit_backend) {
        let pair = match source {
            "input-method" if backend == "none" => ResolvedBackendPair::InputMethod,
            "input-method" => {
                return Err(BackendSelectionError::Incompatible {
                    source: source.to_string(),
                    backend: backend.to_string(),
                })
            }
            "evdev" => ResolvedBackendPair::Evdev(parse_backend(backend)?),
            "stdin" => ResolvedBackendPair::Stdin(parse_backend(backend)?),
            other => return Err(BackendSelectionError::UnknownSource(other.to_string())),
        };
        return Ok(BackendSelection {
            pair,
            reason: "user-specified".to_string(),
        });
    }

    // If the user specified just a source, pick its default output backend.
    if let Some(source) = explicit_source {
        let pair = match source {
            "input-method" => ResolvedBackendPair::InputMethod,
            "evdev" => {
                if !capabilities.has_dev_input {
                    warn!("evdev requested but /dev/input not readable - using stdin instead");
                    return Ok(BackendSelection {
                        pair: ResolvedBackendPair::Stdin(InjectorBackend::Libei),
                        reason: "evdev requested but /dev/input not readable".to_string(),
                    });
                }
                ResolvedBackendPair::Evdev(InjectorBackend::Libei)
            }
            "stdin" => ResolvedBackendPair::Stdin(InjectorBackend::Libei),
            other => return Err(BackendSelectionError::UnknownSource(other.to_string())),
        };
        return Ok(BackendSelection {
            pair,
            reason: format!("user-specified source: {}", source),
        });
    }

    // If the user specified just a backend, pick a compatible source. An
    // explicit injector request must never select input-method-v2 because
    // input-method-v2 is already its own source and injector.
    if let Some(backend) = explicit_backend {
        let backend = parse_backend(backend)?;
        let pair = match backend {
            InjectorBackend::None => ResolvedBackendPair::Stdin(InjectorBackend::None),
            InjectorBackend::Libei if capabilities.has_dev_input => {
                ResolvedBackendPair::Evdev(InjectorBackend::Libei)
            }
            InjectorBackend::Wlroots if capabilities.has_dev_input => {
                ResolvedBackendPair::Evdev(InjectorBackend::Wlroots)
            }
            InjectorBackend::Libei | InjectorBackend::Wlroots => {
                ResolvedBackendPair::Stdin(backend)
            }
        };
        return Ok(BackendSelection {
            pair,
            reason: format!("user-specified backend: {}", backend.name()),
        });
    }

    // Auto-select based on capabilities (SAFETY-FIRST STRATEGY)

    // Prefer input-method-v2 when available (password field safety)
    if capabilities.has_input_method_v2 {
        return Ok(BackendSelection {
            pair: ResolvedBackendPair::InputMethod,
            reason: "auto-selected input-method-v2: safest option (password field protection)"
                .to_string(),
        });
    }

    // Fall back to evdev + libei only if /dev/input is readable
    if capabilities.has_dev_input {
        let libei_reason = if capabilities.has_direct_libei_socket {
            ", direct EIS socket available"
        } else {
            "; portal consent will be requested when libei starts"
        };
        return Ok(BackendSelection {
            pair: ResolvedBackendPair::Evdev(InjectorBackend::Libei),
            reason: format!(
                "auto-selected evdev + libei: input-method-v2 unavailable{}",
                libei_reason
            ),
        });
    }

    // Most conservative: stdin + libei (no special permissions needed)
    warn!(
        "no safe input sources available (input-method-v2, evdev, portal) - falling back to stdin"
    );
    Ok(BackendSelection {
        pair: ResolvedBackendPair::Stdin(InjectorBackend::Libei),
        reason: "conservative fallback: stdin + libei (no other input sources available)"
            .to_string(),
    })
}

fn parse_backend(backend: &str) -> Result<InjectorBackend, BackendSelectionError> {
    match backend {
        "none" => Ok(InjectorBackend::None),
        "libei" => Ok(InjectorBackend::Libei),
        "wlroots" => Ok(InjectorBackend::Wlroots),
        other => Err(BackendSelectionError::UnknownBackend(other.to_string())),
    }
}

/// Capability-based backend auto-selection: safety first.
///
/// Strategy:
/// 1. Probe for actual capabilities
/// 2. Delegate decision logic to select_backend() (deterministically testable)
/// 3. Respect user overrides (explicit source/backend)
/// 4. Prefer safer input sources (input-method-v2 over evdev)
pub fn auto_select(
    explicit_source: Option<&str>,
    explicit_backend: Option<&str>,
) -> Result<BackendSelection, BackendSelectionError> {
    let capabilities = probe_capabilities();
    let compositor = Compositor::detect();
    debug!(
        "detected compositor: {:?}, capabilities: {:?}",
        compositor, capabilities
    );
    select_backend(&capabilities, compositor, explicit_source, explicit_backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Deterministic tests of decision logic (no environment dependencies)

    fn capabilities(
        has_input_method_v2: bool,
        has_dev_input: bool,
        has_direct_libei_socket: bool,
    ) -> Capabilities {
        Capabilities {
            has_input_method_v2,
            has_virtual_keyboard: true,
            has_direct_libei_socket,
            has_dev_input,
            has_window_tracker: false,
        }
    }

    #[test]
    fn decision_prefers_input_method_v2_when_available() {
        let result = select_backend(
            &capabilities(true, true, false),
            Compositor::Gnome,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result.pair, ResolvedBackendPair::InputMethod);
        assert!(result.reason.contains("input-method-v2"));
    }

    #[test]
    fn decision_falls_back_to_evdev_when_input_method_unavailable() {
        let result = select_backend(
            &capabilities(false, true, true),
            Compositor::Sway,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Evdev(InjectorBackend::Libei)
        );
    }

    #[test]
    fn decision_falls_back_to_stdin_when_nothing_available() {
        let result = select_backend(
            &capabilities(false, false, false),
            Compositor::X11,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Stdin(InjectorBackend::Libei)
        );
    }

    #[test]
    fn policy_uses_evdev_on_x11_if_available() {
        let result = select_backend(
            &capabilities(false, true, false),
            Compositor::X11,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Evdev(InjectorBackend::Libei)
        );
    }

    #[test]
    fn decision_user_explicit_selection_overrides_all() {
        let result = select_backend(
            &capabilities(true, true, false),
            Compositor::Gnome,
            Some("stdin"),
            Some("libei"),
        )
        .unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Stdin(InjectorBackend::Libei)
        );
        assert!(result.reason.contains("user-specified"));
    }

    #[test]
    fn explicit_libei_never_selects_input_method() {
        let result = select_backend(
            &capabilities(true, true, false),
            Compositor::Gnome,
            None,
            Some("libei"),
        )
        .unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Evdev(InjectorBackend::Libei)
        );
    }

    #[test]
    fn explicit_wlroots_resolves_a_compatible_source() {
        let result = select_backend(
            &capabilities(true, true, false),
            Compositor::Gnome,
            None,
            Some("wlroots"),
        )
        .unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Evdev(InjectorBackend::Wlroots)
        );
    }

    #[test]
    fn explicit_source_resolves_its_default_output() {
        let result = select_backend(
            &capabilities(false, true, false),
            Compositor::Sway,
            Some("evdev"),
            None,
        )
        .unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Evdev(InjectorBackend::Libei)
        );
    }

    #[test]
    fn input_method_cannot_be_combined_with_an_output_backend() {
        let result = select_backend(
            &Capabilities::default(),
            Compositor::Unknown,
            Some("input-method"),
            Some("libei"),
        );
        assert!(matches!(
            result,
            Err(BackendSelectionError::Incompatible { .. })
        ));
    }

    #[test]
    fn unknown_source_and_backend_are_rejected() {
        assert!(matches!(
            select_backend(
                &Capabilities::default(),
                Compositor::Unknown,
                Some("bogus"),
                None,
            ),
            Err(BackendSelectionError::UnknownSource(_))
        ));
        assert!(matches!(
            select_backend(
                &Capabilities::default(),
                Compositor::Unknown,
                None,
                Some("bogus"),
            ),
            Err(BackendSelectionError::UnknownBackend(_))
        ));
    }

    // Integration tests of the full auto_select flow

    #[test]
    fn explicit_selection_takes_priority() {
        let result = auto_select(Some("stdin"), Some("libei")).unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Stdin(InjectorBackend::Libei)
        );
        assert!(result.reason.contains("user-specified"));
    }

    #[test]
    fn input_method_source_has_no_backend() {
        let result = auto_select(Some("input-method"), None).unwrap();
        assert_eq!(result.pair, ResolvedBackendPair::InputMethod);
    }

    #[test]
    fn reason_field_is_always_populated() {
        let result = auto_select(None, None).unwrap();
        assert!(
            !result.reason.is_empty(),
            "reason should explain the selection"
        );
    }

    #[test]
    fn selection_is_deterministic() {
        let result1 = auto_select(None, None).unwrap();
        let result2 = auto_select(None, None).unwrap();
        assert_eq!(result1.pair, result2.pair);
    }
}
