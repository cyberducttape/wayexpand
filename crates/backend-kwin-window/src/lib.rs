//! Focused-window tracking for KDE Plasma (KWin).
//!
//! KWin exposes no Wayland protocol for reading or subscribing to the
//! focused window's identity -- unlike wlroots compositors
//! (`wlr-foreign-toplevel-management-unstable-v1`), this is a deliberate
//! KDE privacy stance. The only bridge is KWin's scripting engine, reached
//! over the session D-Bus (`org.kde.kwin.Scripting`): we load a small
//! bundled script that watches `workspace.windowActivated` and calls back
//! into a D-Bus service this process hosts for exactly that purpose. This
//! is the same mechanism community tools like `kdotool` use, since no
//! public API exists for it.

use std::{
    fs,
    io::Write,
    process,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use wayexpand_core::{WindowContext, WindowTracker, WindowTrackerError};
use zbus::{blocking::Connection, interface};

const BACKEND_NAME: &str = "kwin-window";
const SCRIPT_TEMPLATE: &str = include_str!("window-tracker.js");
const LOAD_RETRY_ATTEMPTS: u32 = 15;
const LOAD_RETRY_DELAY: Duration = Duration::from_millis(150);
const TRACKER_SETUP_TIMEOUT: Duration = Duration::from_secs(12);
const TRACKER_HEALTH_CHECK_INTERVAL: Duration = Duration::from_millis(500);
/// Upper bound on how long `probe()` waits for the session bus / KWin to
/// answer before giving up. A local D-Bus round trip normally completes in
/// well under this; this exists specifically for the case where it does
/// not (see `probe()`'s doc comment).
const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Error)]
pub enum KwinWindowError {
    #[error("D-Bus call failed: {0}")]
    DBus(#[from] zbus::Error),
    #[error("could not write the KWin tracker script: {0}")]
    ScriptWrite(#[source] std::io::Error),
    #[error("KWin did not finish registering the loaded script in time")]
    ScriptNotReady,
    #[error("KWin tracker setup exceeded its time limit")]
    SetupTimedOut,
    #[error("KWin no longer reports the WayExpand window-tracker script as loaded")]
    ScriptStopped,
    #[error("org.kde.KWin's scripting interface is not reachable on the session bus")]
    NotAvailable,
    #[error("could not generate a unique KWin tracker nonce: {0}")]
    Nonce(String),
}

struct WindowTrackerService {
    sender: Mutex<mpsc::Sender<Option<WindowContext>>>,
}

fn window_context_from_signal(app_id: String, title: String) -> Option<WindowContext> {
    if app_id.is_empty() && title.is_empty() {
        None
    } else {
        Some(WindowContext {
            app_id: (!app_id.is_empty()).then_some(app_id),
            title: (!title.is_empty()).then_some(title),
        })
    }
}

#[interface(name = "org.wayexpand.WindowTracker1")]
impl WindowTrackerService {
    fn window_changed(&self, app_id: String, title: String) {
        let context = window_context_from_signal(app_id, title);
        // The receiver may already be gone if the tracker was dropped
        // between the script firing and this call landing; that is not an
        // error, there is simply nothing left to notify. A poisoned mutex
        // (some other panic while holding the lock) is treated the same way
        // rather than propagating the panic here: this callback runs on
        // every D-Bus dispatch, so unwrap()-ing would permanently break
        // app_filter-scoped snippets on the first poisoning instead of just
        // this one window-change notification.
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(context);
        }
    }
}

/// A `WindowTracker` backed by a KWin script + a private D-Bus service.
/// Each instance owns one uniquely-named bus connection and one uniquely
/// named loaded script (both suffixed with this process's PID), so running
/// more than one WayExpand daemon concurrently does not collide.
pub struct KwinWindowTracker {
    // Kept alive for the object's lifetime: dropping it stops serving the
    // callback interface the loaded script calls into.
    connection: Connection,
    receiver: mpsc::Receiver<Option<WindowContext>>,
    plugin_name: String,
    script_path: std::path::PathBuf,
    next_health_check: Instant,
}

impl KwinWindowTracker {
    /// A side-effect-free check for whether this session is even worth
    /// trying: confirms `org.kde.KWin` answers on the session bus and
    /// advertises the scripting interface, without loading or running
    /// anything.
    ///
    /// The supervisor calls this from its own worker, never from the daemon's
    /// input loop. The D-Bus introspection call has a finite method timeout;
    /// keeping the probe in the supervisor thread also ensures a hung
    /// connection attempt cannot create an unbounded series of abandoned
    /// probe threads across retries.
    pub fn probe() -> Result<(), KwinWindowError> {
        Self::probe_blocking()
    }

    fn probe_blocking() -> Result<(), KwinWindowError> {
        let connection = zbus::blocking::connection::Builder::session()?
            .method_timeout(DBUS_METHOD_TIMEOUT)
            .build()?;
        let reply = connection
            .call_method(
                Some("org.kde.KWin"),
                "/Scripting",
                Some("org.freedesktop.DBus.Introspectable"),
                "Introspect",
                &(),
            )
            .map_err(|_| KwinWindowError::NotAvailable)?;
        let xml: String = reply
            .body()
            .deserialize()
            .map_err(|_| KwinWindowError::NotAvailable)?;
        if xml.contains("org.kde.kwin.Scripting") {
            Ok(())
        } else {
            Err(KwinWindowError::NotAvailable)
        }
    }

    pub fn new() -> Result<Self, KwinWindowError> {
        Self::new_cancellable(None)
    }

    /// Like `new`, but allows an owning UI/task to stop the bounded script
    /// readiness retry loop. An in-flight D-Bus call is bounded by
    /// `DBUS_METHOD_TIMEOUT`.
    pub fn new_cancellable(cancelled: Option<&AtomicBool>) -> Result<Self, KwinWindowError> {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(KwinWindowError::ScriptNotReady);
        }
        let pid = process::id();
        // Reconnects may overlap cleanup of a failed tracker. Give each
        // attempt distinct D-Bus and KWin script identities so a delayed
        // unload cannot collide with the replacement instance.
        let mut nonce_bytes = [0_u8; 8];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|error| KwinWindowError::Nonce(error.to_string()))?;
        let nonce = u64::from_le_bytes(nonce_bytes);
        let bus_name = format!("org.wayexpand.WindowTracker.pid{pid}.n{nonce:x}");
        let (sender, receiver) = mpsc::channel();
        let service = WindowTrackerService {
            sender: Mutex::new(sender),
        };
        let connection = zbus::blocking::connection::Builder::session()?
            .method_timeout(DBUS_METHOD_TIMEOUT)
            .name(bus_name.clone())?
            .serve_at("/WindowTracker", service)?
            .build()?;

        let plugin_name = format!("wayexpand-window-tracker-{pid}-{nonce:x}");
        // The path is otherwise predictable (PID plus a fixed prefix, under
        // world-writable /tmp), so a local attacker who guesses this
        // process's upcoming PID could pre-place a symlink here pointing at
        // a file this user owns elsewhere; `fs::write` follows symlinks and
        // would overwrite that target. A cryptographically random suffix makes
        // the exact path unguessable, and `create_new` (O_CREAT|O_EXCL) refuses
        // to open through anything already there -- symlink or not -- as defense
        // in depth against symlink attacks.
        let script_path = std::env::temp_dir().join(format!("{plugin_name}-{nonce:x}.js"));
        let script_contents = SCRIPT_TEMPLATE.replace("__WAYEXPAND_BUS_NAME__", &bus_name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script_path)
            .map_err(KwinWindowError::ScriptWrite)?;
        file.write_all(script_contents.as_bytes())
            .map_err(KwinWindowError::ScriptWrite)?;

        if let Err(error) = Self::load_and_run(
            &connection,
            &script_path,
            &plugin_name,
            cancelled,
            Instant::now() + TRACKER_SETUP_TIMEOUT,
        ) {
            let _ = connection.call_method(
                Some("org.kde.KWin"),
                "/Scripting",
                Some("org.kde.kwin.Scripting"),
                "unloadScript",
                &(plugin_name.as_str(),),
            );
            let _ = fs::remove_file(&script_path);
            return Err(error);
        }

        Ok(Self {
            connection,
            receiver,
            plugin_name,
            script_path,
            next_health_check: Instant::now() + TRACKER_HEALTH_CHECK_INTERVAL,
        })
    }

    /// `loadScript` returns before the resulting `/Scripting/ScriptN`
    /// object is necessarily reachable yet -- observed directly against a
    /// live KWin 6.6 session, where `run()` immediately after `loadScript`
    /// reliably fails with "No such object path" for roughly the first
    /// second. There is no signal to wait on, so this retries `run()` with
    /// a short, bounded backoff instead of guessing a fixed delay.
    fn load_and_run(
        connection: &Connection,
        script_path: &std::path::Path,
        plugin_name: &str,
        cancelled: Option<&AtomicBool>,
        deadline: Instant,
    ) -> Result<(), KwinWindowError> {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(KwinWindowError::ScriptNotReady);
        }
        if Instant::now() >= deadline {
            return Err(KwinWindowError::SetupTimedOut);
        }
        let script_id: i32 = connection
            .call_method(
                Some("org.kde.KWin"),
                "/Scripting",
                Some("org.kde.kwin.Scripting"),
                "loadScript",
                &(script_path.to_string_lossy().into_owned(), plugin_name),
            )?
            .body()
            .deserialize()?;
        let script_object_path = format!("/Scripting/Script{script_id}");

        for attempt in 0..LOAD_RETRY_ATTEMPTS {
            if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                return Err(KwinWindowError::ScriptNotReady);
            }
            if Instant::now() >= deadline {
                return Err(KwinWindowError::SetupTimedOut);
            }
            match connection.call_method(
                Some("org.kde.KWin"),
                script_object_path.as_str(),
                Some("org.kde.kwin.Script"),
                "run",
                &(),
            ) {
                Ok(_) => return Ok(()),
                Err(_) if attempt + 1 < LOAD_RETRY_ATTEMPTS => {
                    std::thread::sleep(
                        LOAD_RETRY_DELAY.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(_) => return Err(KwinWindowError::ScriptNotReady),
            }
        }
        Err(KwinWindowError::ScriptNotReady)
    }
}

impl Drop for KwinWindowTracker {
    fn drop(&mut self) {
        let _ = self.connection.call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "unloadScript",
            &(self.plugin_name.as_str(),),
        );
        let _ = fs::remove_file(&self.script_path);
    }
}

impl WindowTracker for KwinWindowTracker {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn next_window_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<Option<WindowContext>>, WindowTrackerError> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            if now >= self.next_health_check {
                let loaded: bool = self
                    .connection
                    .call_method(
                        Some("org.kde.KWin"),
                        "/Scripting",
                        Some("org.kde.kwin.Scripting"),
                        "isScriptLoaded",
                        &(self.plugin_name.as_str(),),
                    )
                    .and_then(|reply| reply.body().deserialize())
                    .map_err(|error| WindowTrackerError {
                        backend: BACKEND_NAME,
                        message: format!("checking KWin tracker health failed: {error}"),
                        retryable: true,
                    })?;
                if !loaded {
                    return Err(WindowTrackerError {
                        backend: BACKEND_NAME,
                        message: KwinWindowError::ScriptStopped.to_string(),
                        retryable: true,
                    });
                }
                self.next_health_check = Instant::now() + TRACKER_HEALTH_CHECK_INTERVAL;
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let wait = deadline
                .min(self.next_health_check)
                .saturating_duration_since(now);
            match self.receiver.recv_timeout(wait) {
                Ok(window) => return Ok(Some(window)),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    return Ok(None)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(WindowTrackerError {
                        backend: BACKEND_NAME,
                        message: "the KWin script's D-Bus callback service stopped".into(),
                        retryable: true,
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{window_context_from_signal, KwinWindowTracker, TRACKER_HEALTH_CHECK_INTERVAL};
    use std::{thread, time::Instant};
    use wayexpand_core::WindowTracker;

    #[test]
    fn empty_signal_represents_window_disappearance() {
        assert!(window_context_from_signal(String::new(), String::new()).is_none());
    }

    #[test]
    fn missing_app_id_fails_closed_but_preserves_title() {
        let context = window_context_from_signal(String::new(), "Terminal".into()).unwrap();
        assert_eq!(context.app_id, None);
        assert_eq!(context.title.as_deref(), Some("Terminal"));
    }

    #[test]
    fn missing_title_preserves_app_id() {
        let context =
            window_context_from_signal("org.example.Editor".into(), String::new()).unwrap();
        assert_eq!(context.app_id.as_deref(), Some("org.example.Editor"));
        assert_eq!(context.title, None);
    }

    #[test]
    fn both_fields_are_preserved() {
        let context =
            window_context_from_signal("org.example.Editor".into(), "Document".into()).unwrap();
        assert_eq!(context.app_id.as_deref(), Some("org.example.Editor"));
        assert_eq!(context.title.as_deref(), Some("Document"));
    }

    #[test]
    #[ignore = "requires a live KDE Plasma Wayland session and briefly loads a KWin script"]
    fn live_kwin_tracker_health_probe_detects_script_unload() {
        let mut tracker = KwinWindowTracker::new().expect("connect to the live KWin scripting API");
        let initial = tracker
            .next_window_timeout(std::time::Duration::from_secs(2))
            .expect("receive the initial active-window snapshot");
        assert!(
            initial.is_some(),
            "KWin should report its initial active window"
        );
        assert!(
            initial.flatten().is_some(),
            "KWin should provide application or window identity for the focused window"
        );

        let deadline =
            Instant::now() + TRACKER_HEALTH_CHECK_INTERVAL + std::time::Duration::from_secs(1);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            tracker
                .next_window_timeout(remaining)
                .expect("the script should remain loaded during its health check");
            thread::yield_now();
        }

        let unloaded: bool = tracker
            .connection
            .call_method(
                Some("org.kde.KWin"),
                "/Scripting",
                Some("org.kde.kwin.Scripting"),
                "unloadScript",
                &(tracker.plugin_name.as_str(),),
            )
            .expect("unload only this test's uniquely named KWin script")
            .body()
            .deserialize()
            .expect("decode KWin unload result");
        assert!(unloaded, "the uniquely named test script should be loaded");

        let deadline =
            Instant::now() + TRACKER_HEALTH_CHECK_INTERVAL + std::time::Duration::from_secs(1);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "tracker health probe did not notice script unload"
            );
            match tracker.next_window_timeout(remaining) {
                Ok(_) => thread::yield_now(),
                Err(error) => {
                    assert!(error.message.contains("no longer reports"));
                    break;
                }
            }
        }
    }
}
