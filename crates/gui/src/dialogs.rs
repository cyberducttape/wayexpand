use wayexpand_core::WindowContext;

/// User action deferred while an unsaved draft confirmation dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingAction {
    Select(usize),
    New,
    Duplicate,
    Delete,
    Reload,
    Undo,
    Close,
}

/// Result of the asynchronous "Use current app" detection dialog action.
#[derive(Debug)]
pub(crate) enum AppDetection {
    Found(WindowContext),
    Unavailable,
    /// No other application's window was focused in time.
    TimedOut,
}

/// How long "Use current app" waits for the user to click the target app.
pub(crate) const APP_DETECTION_WINDOW: std::time::Duration = std::time::Duration::from_secs(15);

/// Whether a focused window is one of WayExpand's own (the editor, picker,
/// or form). "Use current app" is clicked inside the editor, so the editor is
/// always focused first and must be skipped.
pub(crate) fn is_wayexpand_window(window: &WindowContext) -> bool {
    window.app_id.as_deref().is_some_and(|app_id| {
        let app_id = app_id.to_ascii_lowercase();
        app_id == "wayexpand"
            || app_id == "wayexpand-gui"
            || app_id.starts_with("io.github.cyberducttape.wayexpand")
    })
}

/// Wait for the user to focus a window that is not WayExpand's, for at most
/// `window`. The desktop (no window) and WayExpand's own windows are
/// skipped. `next` waits up to the given time for the next focus report.
pub(crate) fn wait_for_other_app(
    window: std::time::Duration,
    cancelled: &std::sync::atomic::AtomicBool,
    mut next: impl FnMut(
        std::time::Duration,
    )
        -> Result<Option<Option<WindowContext>>, wayexpand_core::WindowTrackerError>,
) -> AppDetection {
    let deadline = std::time::Instant::now() + window;
    loop {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return AppDetection::Unavailable;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return AppDetection::TimedOut;
        }
        // Short waits so a cancel is noticed promptly.
        match next(remaining.min(std::time::Duration::from_millis(250))) {
            Ok(Some(Some(focused))) if !is_wayexpand_window(&focused) => {
                return AppDetection::Found(focused)
            }
            Ok(_) => {}
            Err(_) => return AppDetection::Unavailable,
        }
    }
}
