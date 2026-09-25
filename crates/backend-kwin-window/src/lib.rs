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
    sync::{mpsc, Mutex},
    thread,
    time::Duration,
};
use thiserror::Error;
use wayexpand_core::{WindowContext, WindowTracker, WindowTrackerError};
use zbus::{blocking::Connection, interface};

const BACKEND_NAME: &str = "kwin-window";
const SCRIPT_TEMPLATE: &str = include_str!("window-tracker.js");
const LOAD_RETRY_ATTEMPTS: u32 = 15;
const LOAD_RETRY_DELAY: Duration = Duration::from_millis(150);
/// Upper bound on how long `probe()` waits for the session bus / KWin to
/// answer before giving up. A local D-Bus round trip normally completes in
/// well under this; this exists specifically for the case where it does
/// not (see `probe()`'s doc comment).
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Error)]
pub enum KwinWindowError {
    #[error("D-Bus call failed: {0}")]
    DBus(#[from] zbus::Error),
    #[error("could not write the KWin tracker script: {0}")]
    ScriptWrite(#[source] std::io::Error),
    #[error("KWin did not finish registering the loaded script in time")]
    ScriptNotReady,
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
}

impl KwinWindowTracker {
    /// A side-effect-free check for whether this session is even worth
    /// trying: confirms `org.kde.KWin` answers on the session bus and
    /// advertises the scripting interface, without loading or running
    /// anything.
    ///
    /// Called synchronously from the daemon's `main()` before its event
    /// loop starts (see `spawn_window_tracker`), so this must never be
    /// allowed to block indefinitely: `zbus::blocking::connection::Connection::session()`
    /// has no timeout of its own, and a session bus or KWin left in a bad
    /// state (observed in practice after a KWin script/D-Bus name from a
    /// prior, forcibly-killed daemon instance was not cleaned up) can hang
    /// it forever, which previously meant the *entire daemon* never
    /// reached its main loop -- no expansions worked, and no log line
    /// even indicated why, since the hang happened before any logging
    /// past this call. Bounded by running the real check on its own
    /// thread and abandoning it (not joining) if it doesn't answer in
    /// time, the same pattern already used for the output injector's
    /// drop on shutdown.
    pub fn probe() -> Result<(), KwinWindowError> {
        let (sender, receiver) = mpsc::channel();
        // Intentionally not joined: if probe_blocking() is itself stuck in
        // a hung D-Bus call, this thread may never finish. Leaking it here
        // is preferable to letting that hang propagate to the caller --
        // the OS reclaims it when the process exits either way.
        thread::Builder::new()
            .name("wayexpand-kwin-probe".into())
            .spawn(move || {
                let _ = sender.send(Self::probe_blocking());
            })
            .map_err(|_| KwinWindowError::NotAvailable)?;
        match receiver.recv_timeout(PROBE_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err(KwinWindowError::NotAvailable),
        }
    }

    fn probe_blocking() -> Result<(), KwinWindowError> {
        let connection = Connection::session()?;
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
        let pid = process::id();
        let bus_name = format!("org.wayexpand.WindowTracker.pid{pid}");
        let (sender, receiver) = mpsc::channel();
        let service = WindowTrackerService {
            sender: Mutex::new(sender),
        };
        let connection = zbus::blocking::connection::Builder::session()?
            .name(bus_name.clone())?
            .serve_at("/WindowTracker", service)?
            .build()?;

        let plugin_name = format!("wayexpand-window-tracker-{pid}");
        // The path is otherwise predictable (PID plus a fixed prefix, under
        // world-writable /tmp), so a local attacker who guesses this
        // process's upcoming PID could pre-place a symlink here pointing at
        // a file this user owns elsewhere; `fs::write` follows symlinks and
        // would overwrite that target. A cryptographically random suffix makes
        // the exact path unguessable, and `create_new` (O_CREAT|O_EXCL) refuses
        // to open through anything already there -- symlink or not -- as defense
        // in depth against symlink attacks.
        let mut nonce_bytes = [0_u8; 8];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|error| KwinWindowError::Nonce(error.to_string()))?;
        let nonce = u64::from_le_bytes(nonce_bytes);
        let script_path = std::env::temp_dir().join(format!("{plugin_name}-{nonce:x}.js"));
        let script_contents = SCRIPT_TEMPLATE.replace("__WAYEXPAND_BUS_NAME__", &bus_name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script_path)
            .map_err(KwinWindowError::ScriptWrite)?;
        file.write_all(script_contents.as_bytes())
            .map_err(KwinWindowError::ScriptWrite)?;

        if let Err(error) = Self::load_and_run(&connection, &script_path, &plugin_name) {
            let _ = fs::remove_file(&script_path);
            return Err(error);
        }

        Ok(Self {
            connection,
            receiver,
            plugin_name,
            script_path,
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
    ) -> Result<(), KwinWindowError> {
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
            match connection.call_method(
                Some("org.kde.KWin"),
                script_object_path.as_str(),
                Some("org.kde.kwin.Script"),
                "run",
                &(),
            ) {
                Ok(_) => return Ok(()),
                Err(_) if attempt + 1 < LOAD_RETRY_ATTEMPTS => {
                    std::thread::sleep(LOAD_RETRY_DELAY);
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
        match self.receiver.recv_timeout(timeout) {
            Ok(window) => Ok(Some(window)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(WindowTrackerError {
                backend: BACKEND_NAME,
                message: "the KWin script's D-Bus callback service stopped".into(),
                retryable: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::window_context_from_signal;

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
}
