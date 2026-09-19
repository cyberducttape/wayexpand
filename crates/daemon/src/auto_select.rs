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

#[derive(Debug, Clone)]
pub struct BackendSelection {
    /// Input source: "stdin", "input-method", or "evdev"
    pub source: String,
    /// Output backend: "none", "libei", or "wlroots"
    pub backend: String,
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
    is_wayland: bool,
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
        is_wayland,
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
) -> BackendSelection {
    // User override: always respect explicit selection
    if let (Some(source), Some(backend)) = (explicit_source, explicit_backend) {
        return BackendSelection {
            source: source.to_string(),
            backend: backend.to_string(),
            reason: "user-specified".to_string(),
        };
    }

    // If user specified just a source, pick best backend for it
    if let Some(source) = explicit_source {
        let backend = match source {
            "input-method" => "none", // input-method is also the injector
            "evdev" => {
                if !capabilities.has_dev_input {
                    warn!("evdev requested but /dev/input not readable - using stdin instead");
                    return BackendSelection {
                        source: "stdin".to_string(),
                        backend: "libei".to_string(),
                        reason: "evdev requested but /dev/input not readable".to_string(),
                    };
                }
                "libei" // libei for output with evdev
            }
            "stdin" => "libei", // libei is default output
            other => {
                return BackendSelection {
                    source: other.to_string(),
                    backend: "none".to_string(),
                    reason: format!("user-specified source: {}", other),
                }
            }
        };
        return BackendSelection {
            source: source.to_string(),
            backend: backend.to_string(),
            reason: format!("user-specified source: {}", source),
        };
    }

    // If user specified just a backend, pick best source for it
    if let Some(backend) = explicit_backend {
        let source = match backend {
            "libei" if capabilities.has_input_method_v2 => "input-method", // Safer than evdev
            "libei" if capabilities.has_dev_input => "evdev",
            "libei" => "stdin", // Conservative fallback
            "wlroots" if !capabilities.is_wayland => "stdin", // No wlroots on X11
            "wlroots" if capabilities.has_input_method_v2 => "input-method", // Safer
            "wlroots" if capabilities.has_dev_input => "evdev",
            "wlroots" => "stdin",
            "none" => "stdin",
            _ => "stdin",
        };
        return BackendSelection {
            source: source.to_string(),
            backend: backend.to_string(),
            reason: format!("user-specified backend: {}", backend),
        };
    }

    // Auto-select based on capabilities (SAFETY-FIRST STRATEGY)

    // Prefer input-method-v2 when available (password field safety)
    if capabilities.has_input_method_v2 {
        return BackendSelection {
            source: "input-method".to_string(),
            backend: "none".to_string(),
            reason: "auto-selected input-method-v2: safest option (password field protection)"
                .to_string(),
        };
    }

    // Fall back to evdev + libei only if /dev/input is readable
    if capabilities.has_dev_input {
        let libei_reason = if capabilities.has_direct_libei_socket {
            ", direct EIS socket available"
        } else {
            "; portal consent will be requested when libei starts"
        };
        return BackendSelection {
            source: "evdev".to_string(),
            backend: "libei".to_string(),
            reason: format!(
                "auto-selected evdev + libei: input-method-v2 unavailable{}",
                libei_reason
            ),
        };
    }

    // Most conservative: stdin + libei (no special permissions needed)
    warn!(
        "no safe input sources available (input-method-v2, evdev, portal) - falling back to stdin"
    );
    BackendSelection {
        source: "stdin".to_string(),
        backend: "libei".to_string(),
        reason: "conservative fallback: stdin + libei (no other input sources available)"
            .to_string(),
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
) -> BackendSelection {
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

    #[test]
    fn decision_prefers_input_method_v2_when_available() {
        let caps = Capabilities {
            has_input_method_v2: true,
            has_virtual_keyboard: true,
            has_direct_libei_socket: false,
            has_dev_input: true,
            has_window_tracker: false,
            is_wayland: true,
        };
        let result = select_backend(&caps, Compositor::Gnome, None, None);
        assert_eq!(result.source, "input-method");
        assert_eq!(result.backend, "none");
        assert!(result.reason.contains("input-method-v2"));
    }

    #[test]
    fn decision_falls_back_to_evdev_when_input_method_unavailable() {
        let caps = Capabilities {
            has_input_method_v2: false,
            has_virtual_keyboard: false,
            has_direct_libei_socket: true,
            has_dev_input: true,
            has_window_tracker: false,
            is_wayland: true,
        };
        let result = select_backend(&caps, Compositor::Sway, None, None);
        assert_eq!(result.source, "evdev");
        assert_eq!(result.backend, "libei");
    }

    #[test]
    fn decision_falls_back_to_stdin_when_nothing_available() {
        let caps = Capabilities {
            has_input_method_v2: false,
            has_virtual_keyboard: false,
            has_direct_libei_socket: false,
            has_dev_input: false,
            has_window_tracker: false,
            is_wayland: false,
        };
        let result = select_backend(&caps, Compositor::X11, None, None);
        assert_eq!(result.source, "stdin");
        assert_eq!(result.backend, "libei");
    }

    #[test]
    fn policy_uses_evdev_on_x11_if_available() {
        // X11 can use evdev if /dev/input is readable (no input-method-v2 available)
        let caps = Capabilities {
            has_input_method_v2: false,
            has_virtual_keyboard: false,
            has_direct_libei_socket: false,
            has_dev_input: true,
            has_window_tracker: false,
            is_wayland: false,
        };
        let result = select_backend(&caps, Compositor::X11, None, None);
        assert_eq!(result.source, "evdev");
        assert_eq!(result.backend, "libei");
    }

    #[test]
    fn decision_user_explicit_selection_overrides_all() {
        let caps = Capabilities {
            has_input_method_v2: true,
            has_virtual_keyboard: true,
            has_direct_libei_socket: false,
            has_dev_input: true,
            has_window_tracker: false,
            is_wayland: true,
        };
        let result = select_backend(&caps, Compositor::Gnome, Some("stdin"), Some("libei"));
        assert_eq!(result.source, "stdin");
        assert_eq!(result.backend, "libei");
        assert!(result.reason.contains("user-specified"));
    }

    #[test]
    fn decision_backend_libei_picks_safest_source() {
        let caps = Capabilities {
            has_input_method_v2: true,
            has_virtual_keyboard: true,
            has_direct_libei_socket: false,
            has_dev_input: true,
            has_window_tracker: false,
            is_wayland: true,
        };
        // When libei is explicitly requested, should prefer input-method over evdev
        let result = select_backend(&caps, Compositor::Gnome, None, Some("libei"));
        assert_eq!(result.source, "input-method");
        assert_eq!(result.backend, "libei");
    }

    #[test]
    fn decision_rejects_unsupported_input_method_on_x11() {
        let caps = Capabilities {
            has_input_method_v2: false, // Not supported on X11
            has_virtual_keyboard: false,
            has_direct_libei_socket: false,
            has_dev_input: false,
            has_window_tracker: false,
            is_wayland: false,
        };
        let result = select_backend(&caps, Compositor::X11, None, Some("libei"));
        assert_eq!(result.source, "stdin");
        assert_eq!(result.backend, "libei");
    }

    // Integration tests of the full auto_select flow

    #[test]
    fn explicit_selection_takes_priority() {
        let result = auto_select(Some("stdin"), Some("libei"));
        assert_eq!(result.source, "stdin");
        assert_eq!(result.backend, "libei");
        assert!(result.reason.contains("user-specified"));
    }

    #[test]
    fn input_method_source_has_no_backend() {
        let result = auto_select(Some("input-method"), None);
        assert_eq!(result.source, "input-method");
        assert_eq!(result.backend, "none");
    }

    #[test]
    fn reason_field_is_always_populated() {
        let result = auto_select(None, None);
        assert!(
            !result.reason.is_empty(),
            "reason should explain the selection"
        );
    }

    #[test]
    fn selection_is_deterministic() {
        let result1 = auto_select(None, None);
        let result2 = auto_select(None, None);
        assert_eq!(result1.source, result2.source);
        assert_eq!(result1.backend, result2.backend);
    }
}
