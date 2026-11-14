//! Backend initialization and lifecycle management.
//!
//! Handles:
//! - Window tracker spawning and event draining
//! - KWin scripting bridge setup
//! - App-filter race prevention by draining pending window changes

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tracing::{info, warn};
use wayexpand_backend_gnome_window::GnomeWindowTracker;
use wayexpand_backend_kwin_window::KwinWindowTracker;
use wayexpand_backend_selection::WindowTrackerKind;
use wayexpand_core::{
    window_update_channel, FocusStamp, ProcessIdentityState, WindowContext, WindowIdentitySource,
    WindowTracker, WindowUpdateReceiver, WindowUpdateSender,
};

/// How long one tracker poll may block. It bounds how quickly the supervisor
/// notices shutdown; KWin health checks run on their own interval.
const TRACKER_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TRACKER_INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const TRACKER_MAX_BACKOFF: Duration = Duration::from_secs(30);
const TRACKER_STABLE_INTERVAL: Duration = Duration::from_secs(30);

const TRACKER_STATE_UNAVAILABLE: u64 = 0;
const TRACKER_STATE_STARTING: u64 = 1;
const TRACKER_STATE_CONNECTED: u64 = 2;
const TRACKER_STATE_RECONNECTING: u64 = 3;
const TRACKER_STATE_STOPPED: u64 = 4;

const TRACKER_FAILURE_PROBE_UNAVAILABLE: u64 = 1;
const TRACKER_FAILURE_CONNECT_FAILED: u64 = 2;
const TRACKER_FAILURE_DISCONNECTED: u64 = 3;
const TRACKER_FAILURE_CONFIGURATION_CHANGED: u64 = 4;
const TRACKER_FAILURE_COMPOSITOR_UNSUPPORTED: u64 = 5;
const TRACKER_FAILURE_SUPERVISOR_START_FAILED: u64 = 6;

/// Sanitized compositor focus-tracker lifecycle counters. Failure reasons are
/// stable categories only; D-Bus errors, app IDs, and window titles stay out
/// of the status contract.
#[derive(Default)]
struct WindowTrackerDiagnostics {
    state: std::sync::atomic::AtomicU64,
    reconnect_attempts_total: std::sync::atomic::AtomicU64,
    disconnects_total: std::sync::atomic::AtomicU64,
    last_failure_reason: std::sync::atomic::AtomicU64,
    last_connected_unix: std::sync::atomic::AtomicU64,
    reconnect_backoff_ms: std::sync::atomic::AtomicU64,
    identity_source: std::sync::atomic::AtomicU64,
    process_identity_state: std::sync::atomic::AtomicU64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowTrackerDiagnosticsSnapshot {
    pub state: &'static str,
    pub reconnect_attempts_total: u64,
    pub disconnects_total: u64,
    pub last_failure_reason: &'static str,
    pub last_connected_unix: u64,
    pub reconnect_backoff_ms: u64,
    pub identity_source: &'static str,
    pub process_identity_state: &'static str,
}

impl WindowTrackerDiagnostics {
    fn snapshot(&self) -> WindowTrackerDiagnosticsSnapshot {
        let state = match self.state.load(Ordering::Relaxed) {
            TRACKER_STATE_STARTING => "starting",
            TRACKER_STATE_CONNECTED => "connected",
            TRACKER_STATE_RECONNECTING => "reconnecting",
            TRACKER_STATE_STOPPED => "stopped",
            _ => "unavailable",
        };
        let last_failure_reason = match self.last_failure_reason.load(Ordering::Relaxed) {
            TRACKER_FAILURE_PROBE_UNAVAILABLE => "probe_unavailable",
            TRACKER_FAILURE_CONNECT_FAILED => "connect_failed",
            TRACKER_FAILURE_DISCONNECTED => "tracker_disconnected",
            TRACKER_FAILURE_CONFIGURATION_CHANGED => "configuration_changed",
            TRACKER_FAILURE_COMPOSITOR_UNSUPPORTED => "compositor_unsupported",
            TRACKER_FAILURE_SUPERVISOR_START_FAILED => "supervisor_start_failed",
            _ => "none",
        };
        let identity_source = WindowIdentitySource::from_code(
            self.identity_source
                .load(Ordering::Relaxed)
                .min(u64::from(u8::MAX)) as u8,
        )
        .as_str();
        let process_identity_state = ProcessIdentityState::from_code(
            self.process_identity_state
                .load(Ordering::Relaxed)
                .min(u64::from(u8::MAX)) as u8,
        )
        .as_str();
        WindowTrackerDiagnosticsSnapshot {
            state,
            reconnect_attempts_total: self.reconnect_attempts_total.load(Ordering::Relaxed),
            disconnects_total: self.disconnects_total.load(Ordering::Relaxed),
            last_failure_reason,
            last_connected_unix: self.last_connected_unix.load(Ordering::Relaxed),
            reconnect_backoff_ms: self.reconnect_backoff_ms.load(Ordering::Relaxed),
            identity_source,
            process_identity_state,
        }
    }

    fn set_failure(&self, reason: u64) {
        self.last_failure_reason.store(reason, Ordering::Relaxed);
        self.state
            .store(TRACKER_STATE_RECONNECTING, Ordering::Release);
    }

    fn set_backoff(&self, delay: Duration) {
        self.reconnect_backoff_ms.store(
            delay.as_millis().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );
    }

    fn mark_connected(&self) {
        self.identity_source.store(
            u64::from(WindowIdentitySource::Unavailable.code()),
            Ordering::Relaxed,
        );
        self.process_identity_state.store(
            u64::from(ProcessIdentityState::NotApplicable.code()),
            Ordering::Relaxed,
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.last_connected_unix.store(now, Ordering::Relaxed);
        self.reconnect_backoff_ms.store(0, Ordering::Relaxed);
        self.state.store(TRACKER_STATE_CONNECTED, Ordering::Release);
    }

    fn mark_stopped(&self) {
        self.reconnect_backoff_ms.store(0, Ordering::Relaxed);
        self.state.store(TRACKER_STATE_STOPPED, Ordering::Release);
    }

    fn mark_unavailable(&self, reason: u64) {
        self.last_failure_reason.store(reason, Ordering::Relaxed);
        self.reconnect_backoff_ms.store(0, Ordering::Relaxed);
        self.state
            .store(TRACKER_STATE_UNAVAILABLE, Ordering::Release);
    }
}

fn shared_tracker_diagnostics() -> Arc<WindowTrackerDiagnostics> {
    use std::sync::OnceLock;
    static DIAGNOSTICS: OnceLock<Arc<WindowTrackerDiagnostics>> = OnceLock::new();
    Arc::clone(DIAGNOSTICS.get_or_init(|| Arc::new(WindowTrackerDiagnostics::default())))
}

pub(crate) fn window_tracker_diagnostics_snapshot() -> WindowTrackerDiagnosticsSnapshot {
    shared_tracker_diagnostics().snapshot()
}

struct TrackerConnectionState {
    connected: Arc<AtomicBool>,
    diagnostics: Arc<WindowTrackerDiagnostics>,
}

#[derive(Clone, Default)]
struct TrackerRequirements {
    title_matching: Option<Arc<AtomicBool>>,
    process_identity: Option<Arc<AtomicBool>>,
}

impl TrackerRequirements {
    fn title_matching(&self) -> bool {
        self.title_matching
            .as_ref()
            .is_some_and(|value| value.load(Ordering::Acquire))
    }

    fn process_identity(&self) -> bool {
        self.process_identity
            .as_ref()
            .is_some_and(|value| value.load(Ordering::Acquire))
    }

    fn changed_since(&self, titles: bool, process_identity: bool) -> bool {
        self.title_matching() != titles || self.process_identity() != process_identity
    }
}

pub struct WindowTrackerHandle {
    pub receiver: WindowUpdateReceiver,
    connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    requirements: TrackerRequirements,
    supervisor: Option<JoinHandle<()>>,
}

impl WindowTrackerHandle {
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// Enable title delivery only when the active config explicitly needs
    /// `title_contains` matching. The supervisor reconnects the GNOME bridge
    /// to apply changes; KWin never sends titles through this contract.
    pub fn set_title_matching_required(&self, required: bool) {
        if let Some(enabled) = &self.requirements.title_matching {
            if enabled.load(Ordering::Acquire) != required {
                enabled.store(required, Ordering::Release);
            }
        }
    }

    /// Resolve focused process identity only while the active configuration
    /// needs it for an explicit process filter or terminal-only scope. The
    /// tracker prefers an exact executable path and reports a mutable process
    /// name only as a lower-assurance fallback; neither is inferred into an
    /// application-ID filter.
    pub fn set_process_identity_required(&self, required: bool) {
        if let Some(enabled) = &self.requirements.process_identity {
            enabled.store(required, Ordering::Release);
        }
    }

    /// A tracker fed by a test through the sender side of `receiver`.
    #[cfg(test)]
    pub(crate) fn for_test(receiver: WindowUpdateReceiver) -> Self {
        Self {
            receiver,
            connected: Arc::new(AtomicBool::new(true)),
            stop: Arc::new(AtomicBool::new(false)),
            requirements: TrackerRequirements::default(),
            supervisor: None,
        }
    }
}

impl Drop for WindowTrackerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(supervisor) = self.supervisor.take() {
            let _ = supervisor.join();
        }
    }
}

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
/// The returned receiver exists even when the desktop's bridge (KWin's
/// scripting interface, or the WayExpand GNOME Shell extension) is initially
/// unavailable; the supervisor keeps probing and reconnects with bounded
/// exponential backoff. Until it sends a fresh focused-window snapshot,
/// app-filtered expansions remain fail-closed. Desktops without a bridge get
/// no tracker. The wlroots toplevel prototype is deliberately not part of the
/// production daemon until its event-loop, ownership, and compositor test
/// coverage are complete.
pub fn spawn_window_tracker(
    requires_titles: bool,
    requires_process_identity: bool,
) -> Option<WindowTrackerHandle> {
    let Some(kind) = WindowTrackerKind::for_session() else {
        shared_tracker_diagnostics().mark_unavailable(TRACKER_FAILURE_COMPOSITOR_UNSUPPORTED);
        return None;
    };

    let (sender, receiver) = window_update_channel();
    let connected = Arc::new(AtomicBool::new(false));
    let supervisor_connected = Arc::clone(&connected);
    let stop = Arc::new(AtomicBool::new(false));
    let supervisor_stop = Arc::clone(&stop);
    let diagnostics = shared_tracker_diagnostics();
    diagnostics
        .state
        .store(TRACKER_STATE_STARTING, Ordering::Release);
    let supervisor_diagnostics = Arc::clone(&diagnostics);
    let requirements = TrackerRequirements {
        title_matching: (kind == WindowTrackerKind::GnomeShell)
            .then(|| Arc::new(AtomicBool::new(requires_titles))),
        process_identity: (kind == WindowTrackerKind::Kwin)
            .then(|| Arc::new(AtomicBool::new(requires_process_identity))),
    };
    let supervisor_requirements = requirements.clone();
    let kwin_process_identity = supervisor_requirements
        .process_identity
        .as_ref()
        .map(Arc::clone);
    match thread::Builder::new()
        .name("wayexpand-window-tracker-supervisor".into())
        .spawn(move || match kind {
            WindowTrackerKind::Kwin => supervise_window_tracker(
                sender,
                supervisor_stop,
                kind.label(),
                KwinWindowTracker::probe,
                move |_| {
                    KwinWindowTracker::new_with_process_identity(Arc::clone(
                        kwin_process_identity
                            .as_ref()
                            .expect("KWin tracker has process-identity state"),
                    ))
                },
                supervisor_requirements,
                TrackerConnectionState {
                    connected: supervisor_connected,
                    diagnostics: supervisor_diagnostics,
                },
            ),
            WindowTrackerKind::GnomeShell => supervise_window_tracker(
                sender,
                supervisor_stop,
                kind.label(),
                GnomeWindowTracker::probe,
                GnomeWindowTracker::new_with_titles,
                supervisor_requirements,
                TrackerConnectionState {
                    connected: supervisor_connected,
                    diagnostics: supervisor_diagnostics,
                },
            ),
        }) {
        Ok(supervisor) => Some(WindowTrackerHandle {
            receiver,
            connected,
            stop,
            requirements,
            supervisor: Some(supervisor),
        }),
        Err(error) => {
            diagnostics.mark_unavailable(TRACKER_FAILURE_SUPERVISOR_START_FAILED);
            warn!(%error, tracker = kind.label(), "could not start window tracker supervisor");
            None
        }
    }
}

fn supervise_window_tracker<T, Probe, Connect, ProbeError, ConnectError>(
    sender: WindowUpdateSender,
    stop: Arc<AtomicBool>,
    label: &'static str,
    mut probe: Probe,
    mut connect: Connect,
    requirements: TrackerRequirements,
    lifecycle: TrackerConnectionState,
) where
    T: WindowTracker,
    Probe: FnMut() -> Result<(), ProbeError>,
    ProbeError: std::fmt::Display,
    Connect: FnMut(bool) -> Result<T, ConnectError>,
    ConnectError: std::fmt::Display,
{
    let mut backoff = ReconnectBackoff::default();
    let mut attempted = false;
    loop {
        if stop.load(Ordering::Acquire) {
            lifecycle.diagnostics.mark_stopped();
            return;
        }
        if attempted {
            lifecycle
                .diagnostics
                .reconnect_attempts_total
                .fetch_add(1, Ordering::Relaxed);
            lifecycle
                .diagnostics
                .state
                .store(TRACKER_STATE_RECONNECTING, Ordering::Release);
        }
        attempted = true;
        lifecycle.connected.store(false, Ordering::Release);
        if sender.send(None).is_err() {
            lifecycle.diagnostics.mark_stopped();
            return;
        }

        if let Err(error) = probe() {
            lifecycle
                .diagnostics
                .set_failure(TRACKER_FAILURE_PROBE_UNAVAILABLE);
            warn!(%error, tracker = label, "window tracking unavailable; will reprobe");
            let delay = backoff.after_failure(None);
            lifecycle.diagnostics.set_backoff(delay);
            if !wait_for_tracker_retry(&stop, &sender, delay) {
                lifecycle.diagnostics.mark_stopped();
                return;
            }
            continue;
        }

        let current_title_requirement = requirements.title_matching();
        let current_process_identity_requirement = requirements.process_identity();
        let mut tracker = match connect(current_title_requirement) {
            Ok(tracker) => tracker,
            Err(error) => {
                lifecycle
                    .diagnostics
                    .set_failure(TRACKER_FAILURE_CONNECT_FAILED);
                warn!(%error, tracker = label, "window tracker failed to connect; will retry");
                let delay = backoff.after_failure(None);
                lifecycle.diagnostics.set_backoff(delay);
                if !wait_for_tracker_retry(&stop, &sender, delay) {
                    lifecycle.diagnostics.mark_stopped();
                    return;
                }
                continue;
            }
        };

        // Clear any previously cached app identity before accepting the new
        // script's initial active-window snapshot.
        lifecycle.connected.store(true, Ordering::Release);
        if sender.send(None).is_err() {
            lifecycle.connected.store(false, Ordering::Release);
            lifecycle.diagnostics.mark_stopped();
            return;
        }
        lifecycle.diagnostics.mark_connected();
        info!(tracker = label, "window tracker connected");
        let connected_at = Instant::now();
        loop {
            // The handle's Drop sets `stop` and joins this thread while the
            // receiver is still alive, so a failed send cannot be relied on
            // to end this loop. Without this check, daemon shutdown hung on
            // a connected tracker until systemd killed it.
            if stop.load(Ordering::Acquire) {
                lifecycle.connected.store(false, Ordering::Release);
                lifecycle.diagnostics.mark_stopped();
                return;
            }
            if requirements.changed_since(
                current_title_requirement,
                current_process_identity_requirement,
            ) {
                lifecycle.connected.store(false, Ordering::Release);
                lifecycle
                    .diagnostics
                    .set_failure(TRACKER_FAILURE_CONFIGURATION_CHANGED);
                if sender.send(None).is_err() {
                    lifecycle.diagnostics.mark_stopped();
                    return;
                }
                break;
            }
            match tracker.next_window_timeout(TRACKER_POLL_INTERVAL) {
                Ok(Some(window)) => {
                    let identity_source = if window.is_none() {
                        WindowIdentitySource::Unavailable
                    } else {
                        tracker.identity_source()
                    };
                    lifecycle
                        .diagnostics
                        .identity_source
                        .store(u64::from(identity_source.code()), Ordering::Relaxed);
                    let process_identity_state = if window.is_none() {
                        ProcessIdentityState::NotApplicable
                    } else {
                        tracker.process_identity_state()
                    };
                    lifecycle
                        .diagnostics
                        .process_identity_state
                        .store(u64::from(process_identity_state.code()), Ordering::Relaxed);
                    if sender.send(window).is_err() {
                        lifecycle.connected.store(false, Ordering::Release);
                        lifecycle.diagnostics.mark_stopped();
                        return;
                    }
                }
                // No focus change within the timeout is expected.
                Ok(None) => {}
                Err(error) => {
                    lifecycle.connected.store(false, Ordering::Release);
                    lifecycle
                        .diagnostics
                        .disconnects_total
                        .fetch_add(1, Ordering::Relaxed);
                    lifecycle
                        .diagnostics
                        .set_failure(TRACKER_FAILURE_DISCONNECTED);
                    warn!(%error, tracker = label, "window tracker disconnected; will reconnect");
                    if sender.send(None).is_err() {
                        lifecycle.diagnostics.mark_stopped();
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
            tracker = label,
            "waiting before reconnecting the window tracker"
        );
        lifecycle.diagnostics.set_backoff(delay);
        if !wait_for_tracker_retry(&stop, &sender, delay) {
            lifecycle.diagnostics.mark_stopped();
            return;
        }
    }
}

fn wait_for_tracker_retry(stop: &AtomicBool, sender: &WindowUpdateSender, delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    while !stop.load(Ordering::Acquire) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(remaining.min(Duration::from_millis(250)));
    }
    if stop.load(Ordering::Acquire) {
        return false;
    }
    // Sending also detects daemon shutdown: once its receiver is dropped,
    // the supervisor exits instead of continuing background probes.
    sender.send(None).is_ok()
}

/// Drain any pending window-change events from the tracker's receiver.
/// This prevents app-filter races where a focus change arrives between
/// input-event wait and processing.
/// Returns the latest window context if any changes were pending, with the
/// focus identity it was published under.
pub fn drain_pending_window_events(
    window_tracker: Option<&WindowUpdateReceiver>,
) -> Option<(Option<WindowContext>, FocusStamp)> {
    if let Some(receiver) = window_tracker {
        return match receiver.try_recv_stamped() {
            Ok(update) => Some(update),
            Err(mpsc::TryRecvError::Empty) => None,
            // The witness already reports a stopped tracker as changed.
            Err(mpsc::TryRecvError::Disconnected) => Some((None, FocusStamp::default())),
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct ScriptedTracker {
        events: VecDeque<Result<Option<WindowContext>, &'static str>>,
        failure_delay: Duration,
    }

    struct GatedTracker {
        ready: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        first_poll: bool,
    }

    impl WindowTracker for GatedTracker {
        fn name(&self) -> &'static str {
            "gated-window-tracker"
        }

        fn next_window_timeout(
            &mut self,
            timeout: Duration,
        ) -> Result<Option<Option<WindowContext>>, wayexpand_core::WindowTrackerError> {
            if self.first_poll {
                self.first_poll = false;
                self.ready.send(()).unwrap();
                let _ = self.release.recv_timeout(timeout);
                Ok(None)
            } else {
                Err(wayexpand_core::WindowTrackerError {
                    backend: self.name(),
                    message: "script stopped".into(),
                    retryable: true,
                })
            }
        }
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
                Err(message) => {
                    thread::sleep(self.failure_delay);
                    Err(wayexpand_core::WindowTrackerError {
                        backend: self.name(),
                        message: message.into(),
                        retryable: true,
                    })
                }
            }
        }
    }

    fn window(app_id: &str) -> WindowContext {
        WindowContext {
            app_id: Some(app_id.into()),
            title: None,
            instance_id: None,
        }
    }

    #[test]
    fn tracker_diagnostics_expose_only_stable_lifecycle_categories() {
        let diagnostics = WindowTrackerDiagnostics::default();
        diagnostics.set_failure(TRACKER_FAILURE_DISCONNECTED);
        diagnostics.set_backoff(Duration::from_millis(750));
        diagnostics
            .reconnect_attempts_total
            .fetch_add(2, Ordering::Relaxed);
        diagnostics
            .disconnects_total
            .fetch_add(1, Ordering::Relaxed);

        let reconnecting = diagnostics.snapshot();
        assert_eq!(reconnecting.state, "reconnecting");
        assert_eq!(reconnecting.last_failure_reason, "tracker_disconnected");
        assert_eq!(reconnecting.reconnect_attempts_total, 2);
        assert_eq!(reconnecting.disconnects_total, 1);
        assert_eq!(reconnecting.reconnect_backoff_ms, 750);

        diagnostics.mark_connected();
        let connected = diagnostics.snapshot();
        assert_eq!(connected.state, "connected");
        assert_eq!(connected.reconnect_backoff_ms, 0);
        assert!(connected.last_connected_unix > 0);
        // The retained failure is a sanitized category, useful for explaining
        // why the last reconnect happened.
        assert_eq!(connected.last_failure_reason, "tracker_disconnected");

        diagnostics.mark_stopped();
        assert_eq!(diagnostics.snapshot().state, "stopped");
        diagnostics.mark_unavailable(TRACKER_FAILURE_COMPOSITOR_UNSUPPORTED);
        let unavailable = diagnostics.snapshot();
        assert_eq!(unavailable.state, "unavailable");
        assert_eq!(unavailable.last_failure_reason, "compositor_unsupported");

        diagnostics.mark_unavailable(TRACKER_FAILURE_SUPERVISOR_START_FAILED);
        assert_eq!(
            diagnostics.snapshot().last_failure_reason,
            "supervisor_start_failed"
        );
    }

    #[test]
    fn supervisor_reprobes_reconnects_and_publishes_fresh_context() {
        let (sender, receiver) = window_update_channel();
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
                failure_delay: Duration::ZERO,
            },
            ScriptedTracker {
                events: VecDeque::from([Ok(Some(window("fresh.app"))), Err("stop test")]),
                failure_delay: Duration::from_millis(50),
            },
        ]);
        let connect = move |_| Ok::<_, &'static str>(trackers.pop_front().unwrap());
        let connected = Arc::new(AtomicBool::new(false));
        let supervisor_connected = Arc::clone(&connected);
        let stop = Arc::new(AtomicBool::new(false));
        let supervisor_stop = Arc::clone(&stop);
        let diagnostics = Arc::new(WindowTrackerDiagnostics::default());
        let supervisor_diagnostics = Arc::clone(&diagnostics);
        let supervisor = thread::spawn(move || {
            supervise_window_tracker(
                sender,
                supervisor_stop,
                "test tracker",
                probe,
                connect,
                TrackerRequirements::default(),
                TrackerConnectionState {
                    connected: supervisor_connected,
                    diagnostics: supervisor_diagnostics,
                },
            );
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
        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.state, "stopped");
        assert_eq!(snapshot.reconnect_attempts_total, 2);
        assert_eq!(snapshot.disconnects_total, 2);
        assert_eq!(snapshot.last_failure_reason, "tracker_disconnected");
    }

    #[test]
    fn supervisor_publishes_tracker_health_and_clears_it_on_disconnect() {
        let (sender, receiver) = window_update_channel();
        let (ready_sender, ready_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let tracker = GatedTracker {
            ready: ready_sender,
            release: release_receiver,
            first_poll: true,
        };
        let connected = Arc::new(AtomicBool::new(false));
        let supervisor_connected = Arc::clone(&connected);
        let stop = Arc::new(AtomicBool::new(false));
        let supervisor_stop = Arc::clone(&stop);
        let mut tracker = Some(tracker);
        let supervisor = thread::spawn(move || {
            supervise_window_tracker(
                sender,
                supervisor_stop,
                "test tracker",
                || Ok::<_, &'static str>(()),
                |_| Ok::<_, &'static str>(tracker.take().unwrap()),
                TrackerRequirements::default(),
                TrackerConnectionState {
                    connected: supervisor_connected,
                    diagnostics: Arc::new(WindowTrackerDiagnostics::default()),
                },
            );
        });

        ready_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(connected.load(Ordering::Acquire));
        release_sender.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while connected.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(!connected.load(Ordering::Acquire));
        drop(receiver);
        supervisor.join().unwrap();
    }

    struct IdleTracker;

    impl WindowTracker for IdleTracker {
        fn name(&self) -> &'static str {
            "idle-window-tracker"
        }

        fn next_window_timeout(
            &mut self,
            timeout: Duration,
        ) -> Result<Option<Option<WindowContext>>, wayexpand_core::WindowTrackerError> {
            thread::sleep(timeout);
            Ok(None)
        }
    }

    #[test]
    fn connected_supervisor_stops_while_the_receiver_is_alive() {
        let (sender, receiver) = window_update_channel();
        let connected = Arc::new(AtomicBool::new(false));
        let supervisor_connected = Arc::clone(&connected);
        let stop = Arc::new(AtomicBool::new(false));
        let supervisor_stop = Arc::clone(&stop);
        let supervisor = thread::spawn(move || {
            supervise_window_tracker(
                sender,
                supervisor_stop,
                "test tracker",
                || Ok::<_, &'static str>(()),
                |_| Ok::<_, &'static str>(IdleTracker),
                TrackerRequirements::default(),
                TrackerConnectionState {
                    connected: supervisor_connected,
                    diagnostics: Arc::new(WindowTrackerDiagnostics::default()),
                },
            );
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while !connected.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(connected.load(Ordering::Acquire));

        // Mirror WindowTrackerHandle::drop: stop, then join with the
        // receiver still in scope.
        let stopping = Instant::now();
        stop.store(true, Ordering::Release);
        supervisor.join().unwrap();
        assert!(
            stopping.elapsed() < Duration::from_secs(1),
            "supervisor took {:?} to stop",
            stopping.elapsed()
        );
        drop(receiver);
    }

    #[test]
    fn title_requirement_changes_reconnect_the_tracker_with_new_privacy_mode() {
        let (sender, receiver) = window_update_channel();
        let connected = Arc::new(AtomicBool::new(false));
        let supervisor_connected = Arc::clone(&connected);
        let stop = Arc::new(AtomicBool::new(false));
        let supervisor_stop = Arc::clone(&stop);
        let title_requirement = Arc::new(AtomicBool::new(false));
        let supervisor_title_requirement = Arc::clone(&title_requirement);
        let (mode_sender, mode_receiver) = mpsc::channel();
        let supervisor = thread::spawn(move || {
            supervise_window_tracker(
                sender,
                supervisor_stop,
                "test tracker",
                || Ok::<_, &'static str>(()),
                move |include_titles| {
                    mode_sender.send(include_titles).unwrap();
                    Ok::<_, &'static str>(IdleTracker)
                },
                TrackerRequirements {
                    title_matching: Some(supervisor_title_requirement),
                    process_identity: None,
                },
                TrackerConnectionState {
                    connected: supervisor_connected,
                    diagnostics: Arc::new(WindowTrackerDiagnostics::default()),
                },
            );
        });

        assert!(!mode_receiver.recv_timeout(Duration::from_secs(1)).unwrap());
        title_requirement.store(true, Ordering::Release);
        assert!(mode_receiver.recv_timeout(Duration::from_secs(2)).unwrap());

        stop.store(true, Ordering::Release);
        supervisor.join().unwrap();
        drop(receiver);
    }

    #[test]
    fn process_identity_requirement_changes_refresh_the_focused_window() {
        let (sender, receiver) = window_update_channel();
        let connected = Arc::new(AtomicBool::new(false));
        let supervisor_connected = Arc::clone(&connected);
        let stop = Arc::new(AtomicBool::new(false));
        let supervisor_stop = Arc::clone(&stop);
        let process_requirement = Arc::new(AtomicBool::new(false));
        let supervisor_process_requirement = Arc::clone(&process_requirement);
        let (mode_sender, mode_receiver) = mpsc::channel();
        let requirements = TrackerRequirements {
            title_matching: None,
            process_identity: Some(Arc::clone(&process_requirement)),
        };
        let supervisor = thread::spawn(move || {
            supervise_window_tracker(
                sender,
                supervisor_stop,
                "test KWin tracker",
                || Ok::<_, &'static str>(()),
                move |_| {
                    mode_sender
                        .send(supervisor_process_requirement.load(Ordering::Acquire))
                        .unwrap();
                    Ok::<_, &'static str>(IdleTracker)
                },
                requirements,
                TrackerConnectionState {
                    connected: supervisor_connected,
                    diagnostics: Arc::new(WindowTrackerDiagnostics::default()),
                },
            );
        });

        assert!(!mode_receiver.recv_timeout(Duration::from_secs(1)).unwrap());
        process_requirement.store(true, Ordering::Release);
        assert!(mode_receiver.recv_timeout(Duration::from_secs(2)).unwrap());

        stop.store(true, Ordering::Release);
        supervisor.join().unwrap();
        drop(receiver);
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
    fn tracker_retry_stops_without_waiting_for_the_full_backoff() {
        let (sender, _receiver) = window_update_channel();
        let stop = AtomicBool::new(true);
        let started = Instant::now();
        assert!(!wait_for_tracker_retry(
            &stop,
            &sender,
            Duration::from_secs(30)
        ));
        assert!(started.elapsed() < Duration::from_millis(100));
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
        let (sender, receiver) = window_update_channel();
        sender
            .send(Some(WindowContext {
                app_id: Some("old.app".into()),
                title: Some("old".into()),
                instance_id: None,
            }))
            .unwrap();
        sender
            .send(Some(WindowContext {
                app_id: Some("new.app".into()),
                title: Some("new".into()),
                instance_id: None,
            }))
            .unwrap();

        let latest = drain_pending_window_events(Some(&receiver)).map(|(window, _)| window);
        assert_eq!(
            latest,
            Some(Some(WindowContext {
                app_id: Some("new.app".into()),
                title: Some("new".into()),
                instance_id: None,
            }))
        );
    }

    #[test]
    fn draining_a_disconnected_tracker_fails_closed_to_no_window() {
        let (sender, receiver) = window_update_channel();
        drop(sender);

        assert_eq!(
            drain_pending_window_events(Some(&receiver)).map(|(window, _)| window),
            Some(None)
        );
    }
}
