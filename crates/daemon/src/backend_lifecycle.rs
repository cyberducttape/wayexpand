//! Backend initialization and lifecycle management.
//!
//! Handles:
//! - Window tracker spawning and event draining
//! - KWin scripting bridge setup
//! - App-filter race prevention by draining pending window changes

use std::{sync::mpsc, thread, time::Duration};
use tracing::{info, warn};
use wayexpand_backend_kwin_window::KwinWindowTracker;
use wayexpand_core::{WindowContext, WindowTracker};

/// Spawn a background window tracker thread if available.
/// Currently only KWin is supported in production.
pub fn spawn_window_tracker() -> Option<mpsc::Receiver<Option<WindowContext>>> {
    // Try the integrated KDE Plasma backend. The wlroots toplevel prototype is
    // deliberately not part of the production daemon until its event-loop,
    // ownership, and compositor test coverage are complete.

    // Try KWin first
    if KwinWindowTracker::probe().is_ok() {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut tracker = match KwinWindowTracker::new() {
                Ok(tracker) => tracker,
                Err(error) => {
                    warn!(%error, "KWin window tracker failed to start after a successful probe");
                    return;
                }
            };
            info!("window tracker active (KWin scripting bridge)");
            loop {
                match tracker.next_window_timeout(Duration::from_secs(2)) {
                    Ok(Some(window)) => {
                        if sender.send(window).is_err() {
                            break;
                        }
                    }
                    // Nothing changed within the timeout: expected and frequent.
                    Ok(None) => {}
                    Err(error) => {
                        warn!(%error, "KWin window tracker stopped");
                        break;
                    }
                }
            }
        });
        return Some(receiver);
    }

    info!("window tracking unavailable; app_filter-scoped expansions will not match");
    None
}

/// Drain any pending window-change events from the tracker's receiver.
/// This prevents app-filter races where a focus change arrives between
/// input-event wait and processing.
/// Returns the latest window context if any changes were pending.
pub fn drain_pending_window_events(
    window_tracker: &Option<mpsc::Receiver<Option<WindowContext>>>,
) -> Option<Option<WindowContext>> {
    if let Some(receiver) = window_tracker.as_ref() {
        let mut latest = None;
        loop {
            match receiver.try_recv() {
                Ok(window) => latest = Some(window),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    latest = Some(None);
                    break;
                }
            }
        }
        return latest;
    }
    None
}
