//! Backend initialization and lifecycle management.
//!
//! Handles:
//! - Window tracker spawning and event draining
//! - KWin scripting bridge setup
//! - App-filter race prevention by draining pending window changes

use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tracing::{info, warn};
use wayexpand_backend_kwin_window::KwinWindowTracker;
use wayexpand_core::{WindowContext, WindowTracker};

const TRACKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
const TRACKER_INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const TRACKER_MAX_BACKOFF: Duration = Duration::from_secs(30);
const TRACKER_STABLE_INTERVAL: Duration = Duration::from_secs(30);

struct ReconnectBackoff {
    current: Duration,
}

impl Default for ReconnectBackoff {
    fn default() -> Self {
        Self {
            current: TRACKER_INITIAL_BACKOFF,
        }
    }
}

impl ReconnectBackoff {
    fn after_failure(&mut self, connected_for: Option<Duration>) -> Duration {
        if connected_for.is_some_and(|duration| duration >= TRACKER_STABLE_INTERVAL) {
            self.current = TRACKER_INITIAL_BACKOFF;
        }
        let delay = self.current;
        self.current = self.current.saturating_mul(2).min(TRACKER_MAX_BACKOFF);
        delay
    }
}

/// Spawn a supervised background window tracker.
///
/// The returned receiver exists even when KWin is initially unavailable;
/// the supervisor keeps probing and reconnects with bounded exponential
/// backoff. Until it sends a fresh focused-window snapshot, app-filtered
/// expansions remain fail-closed.
pub fn spawn_window_tracker() -> Option<mpsc::Receiver<Option<WindowContext>>> {
    // Try the integrated KDE Plasma backend. The wlroots toplevel prototype is
    // deliberately not part of the production daemon until its event-loop,
    // ownership, and compositor test coverage are complete.

    let (sender, receiver) = mpsc::channel();
    match thread::Builder::new()
        .name("wayexpand-window-tracker-supervisor".into())
        .spawn(move || supervise_kwin_window_tracker(sender))
    {
        Ok(_) => Some(receiver),
        Err(error) => {
            warn!(%error, "could not start KWin window tracker supervisor");
            None
        }
    }
}

fn supervise_kwin_window_tracker(sender: mpsc::Sender<Option<WindowContext>>) {
    supervise_window_tracker(sender, KwinWindowTracker::probe, KwinWindowTracker::new);
}

fn supervise_window_tracker<T, Probe, Connect, ProbeError, ConnectError>(
    sender: mpsc::Sender<Option<WindowContext>>,
    mut probe: Probe,
    mut connect: Connect,
) where
    T: WindowTracker,
    Probe: FnMut() -> Result<(), ProbeError>,
    ProbeError: std::fmt::Display,
    Connect: FnMut() -> Result<T, ConnectError>,
    ConnectError: std::fmt::Display,
{
    let mut backoff = ReconnectBackoff::default();
    loop {
        if sender.send(None).is_err() {
            return;
        }

        if let Err(error) = probe() {
            warn!(%error, "KWin window tracking unavailable; will reprobe");
            if !wait_for_tracker_retry(&sender, backoff.after_failure(None)) {
                return;
            }
            continue;
        }

        let mut tracker = match connect() {
            Ok(tracker) => tracker,
            Err(error) => {
                warn!(%error, "KWin window tracker failed to connect; will retry");
                if !wait_for_tracker_retry(&sender, backoff.after_failure(None)) {
                    return;
                }
                continue;
            }
        };

        // Clear any previously cached app identity before accepting the new
        // script's initial active-window snapshot.
        if sender.send(None).is_err() {
            return;
        }
        info!("window tracker connected (KWin scripting bridge)");
        let connected_at = Instant::now();
        loop {
            match tracker.next_window_timeout(TRACKER_POLL_INTERVAL) {
                Ok(Some(window)) => {
                    if sender.send(window).is_err() {
                        return;
                    }
                }
                // No focus change within the timeout is expected.
                Ok(None) => {}
                Err(error) => {
                    warn!(%error, "KWin window tracker disconnected; will reconnect");
                    if sender.send(None).is_err() {
                        return;
                    }
                    break;
                }
            }
        }
        drop(tracker);

        let delay = backoff.after_failure(Some(connected_at.elapsed()));
        info!(
            ?delay,
            "waiting before reconnecting the KWin window tracker"
        );
        if !wait_for_tracker_retry(&sender, delay) {
            return;
        }
    }
}

fn wait_for_tracker_retry(sender: &mpsc::Sender<Option<WindowContext>>, delay: Duration) -> bool {
    thread::sleep(delay);
    // Sending also detects daemon shutdown: once its receiver is dropped,
    // the supervisor exits instead of continuing background probes.
    sender.send(None).is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct ScriptedTracker {
        events: VecDeque<Result<Option<WindowContext>, &'static str>>,
    }

    impl WindowTracker for ScriptedTracker {
        fn name(&self) -> &'static str {
            "scripted-window-tracker"
        }

        fn next_window_timeout(
            &mut self,
            _: Duration,
        ) -> Result<Option<Option<WindowContext>>, wayexpand_core::WindowTrackerError> {
            match self.events.pop_front().unwrap_or(Ok(None)) {
                Ok(window) => Ok(Some(window)),
                Err(message) => Err(wayexpand_core::WindowTrackerError {
                    backend: self.name(),
                    message: message.into(),
                    retryable: true,
                }),
            }
        }
    }

    fn window(app_id: &str) -> WindowContext {
        WindowContext {
            app_id: Some(app_id.into()),
            title: None,
        }
    }

    #[test]
    fn supervisor_reprobes_reconnects_and_publishes_fresh_context() {
        let (sender, receiver) = mpsc::channel();
        let mut probe_count = 0;
        let probe = move || {
            probe_count += 1;
            if probe_count == 1 {
                Err("temporary KWin interruption")
            } else {
                Ok(())
            }
        };
        let mut trackers = VecDeque::from([
            ScriptedTracker {
                events: VecDeque::from([Ok(Some(window("old.app"))), Err("script stopped")]),
            },
            ScriptedTracker {
                events: VecDeque::from([Ok(Some(window("fresh.app"))), Err("stop test")]),
            },
        ]);
        let connect = move || Ok::<_, &'static str>(trackers.pop_front().unwrap());
        let supervisor = thread::spawn(move || {
            supervise_window_tracker(sender, probe, connect);
        });

        let mut refreshed = false;
        while let Ok(snapshot) = receiver.recv_timeout(Duration::from_secs(2)) {
            if snapshot
                .as_ref()
                .and_then(|context| context.app_id.as_deref())
                == Some("fresh.app")
            {
                refreshed = true;
                break;
            }
        }
        drop(receiver);
        supervisor.join().unwrap();
        assert!(refreshed, "reconnected tracker must publish a fresh window");
    }

    #[test]
    fn tracker_backoff_doubles_and_is_capped() {
        let mut backoff = ReconnectBackoff::default();
        assert_eq!(backoff.after_failure(None), Duration::from_millis(250));
        assert_eq!(backoff.after_failure(None), Duration::from_millis(500));
        assert_eq!(backoff.after_failure(None), Duration::from_secs(1));
        for _ in 0..16 {
            backoff.after_failure(None);
        }
        assert_eq!(backoff.current, TRACKER_MAX_BACKOFF);
    }

    #[test]
    fn stable_tracker_connection_resets_backoff() {
        let mut backoff = ReconnectBackoff::default();
        backoff.after_failure(None);
        backoff.after_failure(None);
        assert_eq!(
            backoff.after_failure(Some(TRACKER_STABLE_INTERVAL)),
            TRACKER_INITIAL_BACKOFF
        );
        assert_eq!(backoff.current, TRACKER_INITIAL_BACKOFF * 2);
    }

    #[test]
    fn draining_focus_events_keeps_the_latest_snapshot() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Some(WindowContext {
                app_id: Some("old.app".into()),
                title: Some("old".into()),
            }))
            .unwrap();
        sender
            .send(Some(WindowContext {
                app_id: Some("new.app".into()),
                title: Some("new".into()),
            }))
            .unwrap();

        let latest = drain_pending_window_events(&Some(receiver));
        assert_eq!(
            latest,
            Some(Some(WindowContext {
                app_id: Some("new.app".into()),
                title: Some("new".into()),
            }))
        );
    }

    #[test]
    fn draining_a_disconnected_tracker_fails_closed_to_no_window() {
        let (sender, receiver) = mpsc::channel::<Option<WindowContext>>();
        drop(sender);

        assert_eq!(drain_pending_window_events(&Some(receiver)), Some(None));
    }
}
