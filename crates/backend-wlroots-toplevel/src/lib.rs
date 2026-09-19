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
/// **Implementation Status:** Phase 2 (Wayland connection & protocol binding)
/// The protocol binding is being implemented incrementally.
///
/// Protocol: https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1
pub struct WlrootsTopLevelTracker {
    state: Arc<Mutex<TrackerState>>,
}

struct TrackerState {
    /// Focused window app_id: Some(id) if known, None if unknown or no window
    focused_app_id: Option<String>,
    /// Toplevels tracked by this compositor (app_id -> title)
    toplevels: Vec<ToplevelInfo>,
}

/// Information about a tracked window
struct ToplevelInfo {
    app_id: String,
    title: Option<String>,
    is_focused: bool,
}

impl WlrootsTopLevelTracker {
    /// Create a new tracker and connect to Wayland.
    ///
    /// Returns an error if:
    /// - No Wayland display is available (WAYLAND_DISPLAY env var not set)
    /// - wlr_foreign_toplevel_manager_v1 global is not available
    pub fn new() -> Result<Self, WindowTrackerError> {
        // Phase 2: Attempt to bind to wlr_foreign_toplevel_manager_v1
        // This validates that the compositor supports the protocol
        Self::probe()?;

        Ok(Self {
            state: Arc::new(Mutex::new(TrackerState {
                focused_app_id: None,
                toplevels: Vec::new(),
            })),
        })
    }

    /// Probe whether the wlr_foreign_toplevel_manager_v1 global is available.
    ///
    /// This is a lightweight check that doesn't maintain state.
    /// Returns Ok(()) if the protocol is available, Err otherwise.
    pub fn probe() -> Result<(), WindowTrackerError> {
        // Check if Wayland display is available
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(WindowTrackerError {
                backend: "wlroots-toplevel",
                message: "WAYLAND_DISPLAY not set; not a Wayland session".into(),
                retryable: false,
            });
        }

        // TODO: Phase 2 continuation
        // Implement actual Wayland connection:
        // 1. Connect to wl_display via wayland-client
        // 2. Get wl_registry global
        // 3. Bind to wlr_foreign_toplevel_manager_v1
        // 4. Verify binding succeeds
        // 5. Disconnect and return

        // For now, assume protocol is available on wlroots compositors
        // (Sway, Hyprland, river all implement it)
        Ok(())
    }

    /// Add a toplevel to tracking state
    fn add_toplevel(state: &mut TrackerState, app_id: String, title: Option<String>) {
        state.toplevels.push(ToplevelInfo {
            app_id,
            title,
            is_focused: false,
        });
    }

    /// Mark a toplevel as focused and update focused_app_id
    fn set_focused(state: &mut TrackerState, app_id: &str) {
        for toplevel in &mut state.toplevels {
            toplevel.is_focused = toplevel.app_id == app_id;
        }
        state.focused_app_id = Some(app_id.to_string());
    }

    /// Remove all toplevels from tracking
    fn clear_toplevels(state: &mut TrackerState) {
        state.toplevels.clear();
        state.focused_app_id = None;
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
            // Find the full window context (with title if available)
            let title = state
                .toplevels
                .iter()
                .find(|t| t.app_id == *app_id)
                .and_then(|t| t.title.clone());

            Ok(Some(Some(WindowContext {
                app_id: Some(app_id.clone()),
                title,
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
            toplevels: Vec::new(),
        };
        let tracker = WlrootsTopLevelTracker {
            state: Arc::new(Mutex::new(state)),
        };
        assert_eq!(tracker.name(), "wlroots-toplevel");
    }

    #[test]
    fn probe_requires_wayland_display() {
        // Save original value
        let original = std::env::var_os("WAYLAND_DISPLAY");

        // Unset WAYLAND_DISPLAY
        std::env::remove_var("WAYLAND_DISPLAY");

        let result = WlrootsTopLevelTracker::probe();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .message
            .contains("WAYLAND_DISPLAY not set"));

        // Restore original value
        if let Some(val) = original {
            std::env::set_var("WAYLAND_DISPLAY", val);
        }
    }

    #[test]
    fn probe_succeeds_with_wayland_display() {
        // Set WAYLAND_DISPLAY for this test
        std::env::set_var("WAYLAND_DISPLAY", "wayland-0");

        let result = WlrootsTopLevelTracker::probe();
        // Should succeed (or fail gracefully if not in Wayland session)
        // but WAYLAND_DISPLAY check should pass
        assert!(result.is_ok() || result.unwrap_err().message.contains("protocol"));
    }

    #[test]
    fn add_toplevel_updates_state() {
        let mut state = TrackerState {
            focused_app_id: None,
            toplevels: Vec::new(),
        };

        WlrootsTopLevelTracker::add_toplevel(
            &mut state,
            "org.example.App".to_string(),
            Some("Example App".to_string()),
        );

        assert_eq!(state.toplevels.len(), 1);
        assert_eq!(state.toplevels[0].app_id, "org.example.App");
        assert_eq!(state.toplevels[0].title, Some("Example App".to_string()));
        assert!(!state.toplevels[0].is_focused);
    }

    #[test]
    fn set_focused_updates_state() {
        let mut state = TrackerState {
            focused_app_id: None,
            toplevels: vec![
                ToplevelInfo {
                    app_id: "org.example.App1".to_string(),
                    title: Some("App 1".to_string()),
                    is_focused: false,
                },
                ToplevelInfo {
                    app_id: "org.example.App2".to_string(),
                    title: Some("App 2".to_string()),
                    is_focused: false,
                },
            ],
        };

        WlrootsTopLevelTracker::set_focused(&mut state, "org.example.App2");

        assert_eq!(state.focused_app_id, Some("org.example.App2".to_string()));
        assert!(!state.toplevels[0].is_focused);
        assert!(state.toplevels[1].is_focused);
    }

    #[test]
    fn clear_toplevels_resets_state() {
        let mut state = TrackerState {
            focused_app_id: Some("org.example.App".to_string()),
            toplevels: vec![ToplevelInfo {
                app_id: "org.example.App".to_string(),
                title: Some("App".to_string()),
                is_focused: true,
            }],
        };

        WlrootsTopLevelTracker::clear_toplevels(&mut state);

        assert!(state.focused_app_id.is_none());
        assert!(state.toplevels.is_empty());
    }

    // TODO: Add integration tests for Sway/Hyprland/river
    // Test cases should verify:
    // 1. Compositor with wlr-foreign-toplevel-management available
    // 2. Correct app_id extraction from toplevel
    // 3. Focus change detection
    // 4. Timeout behavior on unavailable compositor
}
