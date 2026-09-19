/// Intelligent backend auto-selection strategy.
///
/// Implements "libei-first + smart fallback" to reduce user friction:
/// 1. Detect compositor and available protocols
/// 2. Choose optimal source and backend combination
/// 3. Allow user to override if needed
use std::env;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct BackendSelection {
    /// Input source: "stdin", "input-method", or "evdev"
    pub source: String,
    /// Output backend: "none", "libei", or "wlroots"
    pub backend: String,
    /// Why this selection was made (for logging/documentation)
    pub reason: String,
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

/// Libei-first strategy: try libei portal, fall back based on compositor
pub fn auto_select(
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

    // Detect compositor
    let compositor = Compositor::detect();
    debug!("detected compositor: {:?}", compositor);

    // If user specified just a source, pick best backend for it
    if let Some(source) = explicit_source {
        let backend = match source {
            "input-method" => "none", // input-method is also the injector
            "evdev" => "libei",       // libei for output with evdev
            "stdin" => "libei",       // libei is default output
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
            "libei" => "evdev", // Use evdev with libei output
            "wlroots" if compositor == Compositor::X11 => "stdin", // No wlroots on X11
            "wlroots" => "evdev", // Use evdev with wlroots output
            "none" => "stdin",
            _ => "stdin",
        };
        return BackendSelection {
            source: source.to_string(),
            backend: backend.to_string(),
            reason: format!("user-specified backend: {}", backend),
        };
    }

    // Auto-select based on compositor (LIBEI-FIRST STRATEGY)
    match compositor {
        Compositor::KdePlasma => {
            // KDE: Use input-method-v2 (best KDE integration)
            // Falls back to evdev + wlroots if needed
            BackendSelection {
                source: "input-method".to_string(),
                backend: "none".to_string(),
                reason: "auto-detected KDE Plasma: using input-method-v2".to_string(),
            }
        }
        Compositor::Sway | Compositor::Hyprland | Compositor::River => {
            // wlroots: Try evdev + libei (libei-first strategy)
            // Better compatibility: evdev works everywhere, libei has portal consent
            BackendSelection {
                source: "evdev".to_string(),
                backend: "libei".to_string(),
                reason:
                    "auto-detected wlroots compositor: using evdev + libei (libei-first strategy)"
                        .to_string(),
            }
        }
        Compositor::Gnome => {
            // GNOME: Use input-method-v2 (no wlr-foreign-toplevel support)
            // No window tracking, but best GNOME integration
            BackendSelection {
                source: "input-method".to_string(),
                backend: "none".to_string(),
                reason: "auto-detected GNOME: using input-method-v2 (no window tracking available)"
                    .to_string(),
            }
        }
        Compositor::X11 => {
            // X11: Use evdev with fallback to stdin
            // X11 doesn't have exclusive keyboard grab protocol
            BackendSelection {
                source: "evdev".to_string(),
                backend: "none".to_string(),
                reason: "auto-detected X11: using evdev (X11 compatibility mode)".to_string(),
            }
        }
        Compositor::Unknown => {
            // Conservative fallback: stdin (no special permissions needed)
            BackendSelection {
                source: "stdin".to_string(),
                backend: "libei".to_string(),
                reason: "unknown compositor: using stdin + libei (conservative fallback)"
                    .to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_selection_takes_priority() {
        let result = auto_select(Some("stdin"), Some("libei"));
        assert_eq!(result.source, "stdin");
        assert_eq!(result.backend, "libei");
        assert!(result.reason.contains("user-specified"));
    }

    #[test]
    fn respects_source_only() {
        let result = auto_select(Some("evdev"), None);
        assert_eq!(result.source, "evdev");
        assert_eq!(result.backend, "libei"); // Default backend for evdev
    }

    #[test]
    fn respects_backend_only() {
        let result = auto_select(None, Some("libei"));
        assert_eq!(result.backend, "libei");
        assert_eq!(result.source, "evdev"); // Default source for libei
    }

    #[test]
    fn input_method_source_has_no_backend() {
        let result = auto_select(Some("input-method"), None);
        assert_eq!(result.source, "input-method");
        assert_eq!(result.backend, "none");
    }
}
