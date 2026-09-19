//! Wlroots window tracking via wlr-foreign-toplevel-management-v1 protocol.
//!
//! Implements WindowTracker for wlroots-based compositors (Sway, Hyprland, river).
//! Uses the wlr-foreign-toplevel-management protocol to receive window focus and
//! metadata changes, avoiding the need for D-Bus (KDE-only) or polling.
//!
//! **Status:** Phase 1 (Protocol Connection) - Foundation for focus tracking.
//! Phase 2 (Focus Tracking) and Phase 3 (Daemon Integration) implement
//! async notification and daemon integration.

use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tracing::debug;
use wayexpand_core::{WindowContext, WindowTracker, WindowTrackerError};

#[derive(Debug, Error)]
pub enum WlrootsToplevelError {
    #[error("wayland connection failed: {0}")]
    Connection(String),
    #[error("wlr-foreign-toplevel-management-v1 protocol not available")]
    ProtocolNotAvailable,
    #[error("window tracking probe timeout")]
    ProbeTimeout,
}

/// Phase 1: Protocol connection and toplevel list management.
///
/// Stores the set of known toplevels and which one is focused.
/// Phase 2 will add event-driven updates and callbacks.
#[allow(dead_code)]
struct ToplevelState {
    toplevels: Vec<ToplevelInfo>,
    focused_app_id: Option<String>,
    focused_title: Option<String>,
}

#[allow(dead_code)]
struct ToplevelInfo {
    app_id: Option<String>,
    title: Option<String>,
    focused: bool,
}

impl ToplevelState {
    fn new() -> Self {
        Self {
            toplevels: Vec::new(),
            focused_app_id: None,
            focused_title: None,
        }
    }

    fn current_window(&self) -> Option<WindowContext> {
        if self.focused_app_id.is_none() && self.focused_title.is_none() {
            return None;
        }
        Some(WindowContext {
            app_id: self.focused_app_id.clone().map(Into::into),
            title: self.focused_title.clone().map(Into::into),
        })
    }
}

/// Window tracker implementation for wlroots compositors.
///
/// **Phase 1:** Maintains a list of known toplevels and the currently focused window.
/// Connection is established but event handling is stubbed for Phase 2.
pub struct WlrootsToplevelTracker {
    state: Arc<Mutex<ToplevelState>>,
}

impl WlrootsToplevelTracker {
    /// Create a new wlroots toplevel tracker. Probes for protocol availability
    /// within the given timeout. Currently a stub for Phase 1.
    pub fn new(timeout: Duration) -> Result<Self, WlrootsToplevelError> {
        // Phase 1: Check if WAYLAND_DISPLAY is set (required for Wayland connection)
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(WlrootsToplevelError::Connection(
                "WAYLAND_DISPLAY not set".into(),
            ));
        }

        // TODO: Phase 1 - Connect to Wayland and bind to wlr-foreign-toplevel-manager
        // This requires:
        // 1. Connection::connect_to_env()
        // 2. Get registry and bind to zwlr_foreign_toplevel_manager_v1
        // 3. Iterate existing toplevels and listen for focus changes
        debug!(
            timeout_ms = timeout.as_millis(),
            "wlroots toplevel tracker initialization deferred to full Phase 1"
        );

        let state = Arc::new(Mutex::new(ToplevelState::new()));
        Ok(Self { state })
    }

    /// Get the currently focused window, if any.
    fn get_current_window(&self) -> Option<WindowContext> {
        self.state.lock().ok()?.current_window()
    }
}

impl WindowTracker for WlrootsToplevelTracker {
    fn name(&self) -> &'static str {
        "wlr-foreign-toplevel-management-v1"
    }

    fn next_window_timeout(
        &mut self,
        _timeout: Duration,
    ) -> Result<Option<Option<WindowContext>>, WindowTrackerError> {
        // TODO: Phase 2 - Implement async notification
        // Current stub returns the cached window, which doesn't wait for changes.
        match self.get_current_window() {
            Some(window) => Ok(Some(Some(window))),
            None => Ok(Some(None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_creation_result_is_reasonable() {
        // Protocol may or may not be available depending on Wayland server presence.
        // Either success or failure is acceptable at Phase 1.
        let result = WlrootsToplevelTracker::new(Duration::from_secs(5));
        // Result should be either Ok or Err, not panic
        let _ = result;
    }

    #[test]
    fn tracker_reports_correct_name() {
        // Verify the implementation identifies itself correctly
        let result = WlrootsToplevelTracker::new(Duration::from_secs(5));
        if let Ok(tracker) = result {
            assert_eq!(tracker.name(), "wlr-foreign-toplevel-management-v1");
        }
    }
}
