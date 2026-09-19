use std::sync::{Arc, Mutex};
use std::time::Duration;
use wayexpand_core::{WindowContext, WindowTracker, WindowTrackerError};

/// Wlroots foreign-toplevel-management protocol-based window tracker.
///
/// Provides focused-window identity for wlroots compositors (Sway, Hyprland,
/// river, etc.) via the `wlr_foreign_toplevel_management_unstable_v1` protocol.
/// This enables `app_filter`-scoped expansions on compositors lacking KWin or
/// input-method-v2 window tracking.
///
/// **Implementation Status:** Phase 1 (Wayland connection)
/// The protocol binding and event handling are being added incrementally.
///
/// Protocol: https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1
pub struct WlrootsTopLevelTracker {
    state: Arc<Mutex<TrackerState>>,
}

struct TrackerState {
    focused_app_id: Option<String>,
}

impl WlrootsTopLevelTracker {
    /// Create a new tracker and connect to Wayland.
    ///
    /// Returns an error if:
    /// - No Wayland display is available (WAYLAND_DISPLAY env var not set)
    /// - wlr_foreign_toplevel_manager_v1 global is not available
    pub fn new() -> Result<Self, WindowTrackerError> {
        // TODO: Connect to Wayland display via wayland-client
        // TODO: Get wl_registry
        // TODO: Bind to wlr_foreign_toplevel_manager_v1
        // For now, return a working placeholder that will be filled in Phase 2

        Ok(Self {
            state: Arc::new(Mutex::new(TrackerState {
                focused_app_id: None,
            })),
        })
    }

    /// Probe whether the wlr_foreign_toplevel_manager_v1 global is available.
    ///
    /// This is a lightweight check that doesn't maintain state.
    pub fn probe() -> Result<(), WindowTrackerError> {
        // TODO: Implement a non-state-maintaining probe
        // Connect, check for global, disconnect, return result
        Err(WindowTrackerError {
            backend: "wlroots-toplevel",
            message: "wlr-foreign-toplevel-management protocol not available on this compositor"
                .into(),
            retryable: false,
        })
    }
}

impl WindowTracker for WlrootsTopLevelTracker {
    fn name(&self) -> &'static str {
        "wlroots-toplevel"
    }

    fn next_window_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<Option<WindowContext>>, WindowTrackerError> {
        // TODO: Implement event queue polling with timeout
        // 1. Wait for wlr_foreign_toplevel events (app_id changes, activation)
        // 2. Parse app_id from events
        // 3. Return focused window or None if no focus change before timeout
        // 4. Handle errors gracefully (compositor disconnect, etc.)

        let state = self.state.lock().map_err(|_| WindowTrackerError {
            backend: "wlroots-toplevel",
            message: "internal state lock poisoned".into(),
            retryable: false,
        })?;

        // Placeholder: return the current focused app if any
        if let Some(ref app_id) = state.focused_app_id {
            Ok(Some(Some(WindowContext {
                app_id: Some(app_id.clone()),
                title: None, // TODO: Extract from toplevel if available
            })))
        } else {
            // Timeout reached without change
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_name_is_correct() {
        let state = TrackerState {
            focused_app_id: None,
        };
        let tracker = WlrootsTopLevelTracker {
            state: Arc::new(Mutex::new(state)),
        };
        assert_eq!(tracker.name(), "wlroots-toplevel");
    }

    #[test]
    fn probe_returns_unavailable_until_implemented() {
        let result = WlrootsTopLevelTracker::probe();
        assert!(result.is_err());
    }

    // TODO: Add integration tests for Sway/Hyprland/river
    // Test cases should verify:
    // 1. Compositor with wlr-foreign-toplevel-management available
    // 2. Correct app_id extraction from toplevel
    // 3. Focus change detection
    // 4. Timeout behavior on unavailable compositor
}
