/// Backend capability contracts — which features each backend supports.
///
/// Enables diagnostics, fleet configuration validation, and graceful degradation.
use std::fmt;

/// Feature capabilities supported by a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// Backend name (matches TextInjector::name())
    pub backend_name: &'static str,

    /// Can insert text containing newlines (multiline expansion)
    pub multiline: bool,

    /// Can clear undo history (supports undo/redo key binding)
    pub undo_history: bool,

    /// Text insertion method: UTF-8 direct or keyboard key synthesis
    pub text_method: TextMethod,

    /// Exclusive keyboard capture (input source can prevent other apps seeing keys)
    pub exclusive_capture: bool,

    /// Works on Wayland
    pub wayland: bool,

    /// Works on X11
    pub x11: bool,

    /// Supports app_filter window matching via dedicated protocol
    pub app_filter_native: bool,

    /// Maximum replacement size in bytes (0 = unlimited)
    pub max_replacement_size: usize,

    /// Human-readable feature list for diagnostics
    pub feature_summary: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextMethod {
    /// Direct UTF-8 text insertion (fast, layout-independent)
    DirectUtf8,
    /// Keyboard key synthesis (slow per-character, layout-dependent)
    KeySynthesis,
    /// Input method protocol (bidirectional with app)
    InputMethodProtocol,
}

impl fmt::Display for TextMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextMethod::DirectUtf8 => write!(f, "direct UTF-8"),
            TextMethod::KeySynthesis => write!(f, "key synthesis"),
            TextMethod::InputMethodProtocol => write!(f, "input method protocol"),
        }
    }
}

impl Capabilities {
    /// Get capability profile for a known backend
    pub fn for_backend(name: &str) -> Option<Self> {
        match name {
            "libei" => Some(Capabilities {
                backend_name: "libei",
                multiline: true,
                undo_history: false,
                text_method: TextMethod::DirectUtf8, // with ei_text fallback to key synthesis
                exclusive_capture: false,
                wayland: true,
                x11: false,
                app_filter_native: false,
                max_replacement_size: 1024 * 1024,
                feature_summary: "UTF-8 direct insertion, multiline, portal-based, layout-independent",
            }),
            "wlroots" => Some(Capabilities {
                backend_name: "wlroots",
                multiline: true,
                undo_history: false,
                text_method: TextMethod::DirectUtf8,
                exclusive_capture: false,
                wayland: true,
                x11: false,
                app_filter_native: true,
                max_replacement_size: 1024 * 1024,
                feature_summary: "UTF-8 direct insertion, multiline, wlr-virtual-keyboard, native app_filter for Sway/Hyprland/river",
            }),
            "input-method" => Some(Capabilities {
                backend_name: "input-method",
                multiline: true,
                undo_history: false,
                text_method: TextMethod::InputMethodProtocol,
                exclusive_capture: true,
                wayland: true,
                x11: false,
                app_filter_native: false,
                max_replacement_size: 1024 * 1024,
                feature_summary: "Exclusive keyboard capture, input method protocol, KDE Plasma + GNOME support",
            }),
            "evdev" => Some(Capabilities {
                backend_name: "evdev",
                multiline: false,
                undo_history: false,
                text_method: TextMethod::KeySynthesis,
                exclusive_capture: false,
                wayland: true,
                x11: true,
                app_filter_native: false,
                max_replacement_size: 65536, // larger replacements get dropped during key buffering
                feature_summary: "Input capture only (pairs with libei/wlroots output), no exclusive grab",
            }),
            "input-method-v2" => Some(Capabilities {
                backend_name: "input-method-v2",
                multiline: true,
                undo_history: false,
                text_method: TextMethod::InputMethodProtocol,
                exclusive_capture: true,
                wayland: true,
                x11: false,
                app_filter_native: false,
                max_replacement_size: 1024 * 1024,
                feature_summary: "Input method protocol, exclusive keyboard capture, bidirectional state tracking",
            }),
            _ => None,
        }
    }

    /// Check if this backend can handle a replacement of given size
    pub fn can_handle_size(&self, bytes: usize) -> bool {
        self.max_replacement_size == 0 || bytes <= self.max_replacement_size
    }

    /// Check if this backend works in the current environment
    pub fn works_in_environment(&self) -> bool {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let x11 = std::env::var_os("DISPLAY").is_some();
        (wayland && self.wayland) || (x11 && self.x11)
    }

    /// Get recommended max replacement size for this backend
    pub fn recommended_max_size(&self) -> usize {
        match self.text_method {
            TextMethod::DirectUtf8 | TextMethod::InputMethodProtocol => 1024 * 1024,
            TextMethod::KeySynthesis => 65536, // evdev key synthesis gets slow/drops chars above this
        }
    }
}

/// All known backends with their capabilities
pub fn all_capabilities() -> Vec<Capabilities> {
    vec![
        Capabilities::for_backend("libei"),
        Capabilities::for_backend("wlroots"),
        Capabilities::for_backend("input-method"),
        Capabilities::for_backend("input-method-v2"),
        Capabilities::for_backend("evdev"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libei_has_multiline_support() {
        let caps = Capabilities::for_backend("libei").unwrap();
        assert!(caps.multiline);
        assert!(caps.wayland);
    }

    #[test]
    fn evdev_has_smaller_size_limit() {
        let caps = Capabilities::for_backend("evdev").unwrap();
        let libei = Capabilities::for_backend("libei").unwrap();
        assert!(caps.max_replacement_size < libei.max_replacement_size);
    }

    #[test]
    fn input_method_has_exclusive_capture() {
        let caps = Capabilities::for_backend("input-method").unwrap();
        assert!(caps.exclusive_capture);
    }

    #[test]
    fn unknown_backend_returns_none() {
        assert_eq!(Capabilities::for_backend("unknown"), None);
    }

    #[test]
    fn all_backends_accessible() {
        let all = all_capabilities();
        assert!(all.len() >= 4);
        assert!(all.iter().any(|c| c.backend_name == "libei"));
        assert!(all.iter().any(|c| c.backend_name == "wlroots"));
    }
}
