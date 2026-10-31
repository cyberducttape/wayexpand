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
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    process,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use wayexpand_core::{
    window_update_channel, WindowContext, WindowTracker, WindowTrackerError, WindowUpdateReceiver,
    WindowUpdateSender,
};
use zbus::{blocking::Connection, interface};

const BACKEND_NAME: &str = "kwin-window";
const SCRIPT_TEMPLATE: &str = include_str!("window-tracker.js");
const LOAD_RETRY_ATTEMPTS: u32 = 15;
const LOAD_RETRY_DELAY: Duration = Duration::from_millis(150);
const TRACKER_SETUP_TIMEOUT: Duration = Duration::from_secs(12);
const DBUS_CONNECTION_TIMEOUT: Duration = Duration::from_secs(3);
const TRACKER_HEALTH_CHECK_INTERVAL: Duration = Duration::from_millis(500);
/// How long health checks may keep failing to get an answer before the
/// tracker is treated as lost. KWin answers D-Bus on its main thread, which
/// stalls for seconds under memory pressure; a check that times out is not
/// evidence that the script stopped, and tearing the tracker down and
/// reloading the script under the same pressure only failed again. A
/// definite "not loaded" answer still ends the tracker at once.
const TRACKER_HEALTH_GRACE: Duration = Duration::from_secs(10);
/// Upper bound on how long `probe()` waits for the session bus / KWin to
/// answer before giving up. A local D-Bus round trip normally completes in
/// well under this; this exists specifically for the case where it does
/// not (see `probe()`'s doc comment).
const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(1);
/// Keep orphan cleanup from turning a daemon start into an unbounded scan or
/// a series of slow synchronous compositor round trips.
const ORPHAN_SCAN_ENTRY_LIMIT: usize = 512;
const ORPHAN_CHECK_LIMIT: usize = 4;

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
    #[error("session D-Bus connection setup exceeded its time limit")]
    ConnectionTimedOut,
    #[error("KWin no longer reports the WayExpand window-tracker script as loaded")]
    ScriptStopped,
    #[error("org.kde.KWin's scripting interface is not reachable on the session bus")]
    NotAvailable,
    #[error("could not generate a unique KWin tracker nonce: {0}")]
    Nonce(String),
}

struct WindowTrackerService {
    sender: Mutex<WindowUpdateSender>,
    // Session-bus methods are callable by other local clients; only accept
    // focus reports from the KWin process that owns org.kde.KWin at setup.
    kwin_owner: String,
}

fn is_kwin_sender(sender: Option<&str>, kwin_owner: &str) -> bool {
    sender == Some(kwin_owner)
}

fn window_context_from_signal(
    app_id: String,
    title: String,
    instance_id: String,
) -> Option<WindowContext> {
    if app_id.is_empty() && title.is_empty() && instance_id.is_empty() {
        None
    } else {
        Some(WindowContext {
            app_id: (!app_id.is_empty()).then_some(app_id),
            title: (!title.is_empty()).then_some(title),
            instance_id: (!instance_id.is_empty()).then_some(instance_id),
        })
    }
}

#[interface(name = "org.wayexpand.WindowTracker1")]
impl WindowTrackerService {
    fn window_changed(
        &self,
        app_id: String,
        title: String,
        instance_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) {
        if !is_kwin_sender(
            header.sender().map(|sender| sender.as_str()),
            &self.kwin_owner,
        ) {
            return;
        }
        let context = window_context_from_signal(app_id, title, instance_id);
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
    receiver: WindowUpdateReceiver,
    #[cfg(test)]
    service_name: String,
    plugin_name: String,
    script_path: std::path::PathBuf,
    next_health_check: Instant,
    /// When KWin last answered a health check.
    last_healthy: Instant,
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
        let connection = bounded_session_connection(
            zbus::connection::Builder::session()?.method_timeout(DBUS_METHOD_TIMEOUT),
            DBUS_CONNECTION_TIMEOUT,
        )?;
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
        let (sender, receiver) = window_update_channel();
        let owner_connection = bounded_session_connection(
            zbus::connection::Builder::session()?.method_timeout(DBUS_METHOD_TIMEOUT),
            DBUS_CONNECTION_TIMEOUT,
        )?;
        let kwin_owner: String = owner_connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &("org.kde.KWin",),
            )?
            .body()
            .deserialize()?;
        drop(owner_connection);

        // Register the callback object on the builder before the connection is
        // exposed on the bus. KWin reports workspace.activeWindow immediately
        // when the script starts; registering through object_server().at()
        // after build() leaves a race where that first callback can arrive
        // before the interface is ready and the initial focus is lost.
        let connection = bounded_session_connection(
            zbus::connection::Builder::session()?
                .method_timeout(DBUS_METHOD_TIMEOUT)
                .name(bus_name.clone())?
                .serve_at(
                    "/WindowTracker",
                    WindowTrackerService {
                        sender: Mutex::new(sender),
                        kwin_owner,
                    },
                )?,
            DBUS_CONNECTION_TIMEOUT,
        )?;

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
            .mode(0o600)
            .open(&script_path)
            .map_err(KwinWindowError::ScriptWrite)?;
        let write_result = file
            .set_permissions(fs::Permissions::from_mode(0o600))
            .and_then(|()| file.write_all(script_contents.as_bytes()));
        if let Err(error) = write_result {
            drop(file);
            let _ = fs::remove_file(&script_path);
            return Err(KwinWindowError::ScriptWrite(error));
        }
        drop(file);

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

        cleanup_orphan_tracker_scripts(
            &std::env::temp_dir(),
            |old_pid| Path::new("/proc").join(old_pid.to_string()).exists(),
            |old_plugin| {
                connection
                    .call_method(
                        Some("org.kde.KWin"),
                        "/Scripting",
                        Some("org.kde.kwin.Scripting"),
                        "isScriptLoaded",
                        &(old_plugin,),
                    )
                    .and_then(|reply| reply.body().deserialize())
                    .ok()
            },
        );

        Ok(Self {
            connection,
            receiver,
            #[cfg(test)]
            service_name: bus_name,
            plugin_name,
            script_path,
            next_health_check: Instant::now() + TRACKER_HEALTH_CHECK_INTERVAL,
            last_healthy: Instant::now(),
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

/// Remove temporary files left by WayExpand tracker processes that exited
/// without running `Drop` (for example, after SIGKILL or a process crash).
/// This deliberately does not unload scripts or touch files owned by another
/// process: it removes only an exact generated file for a dead PID after KWin
/// confirms that the corresponding script is not loaded.
fn cleanup_orphan_tracker_scripts(
    directory: &Path,
    mut process_is_alive: impl FnMut(u32) -> bool,
    mut script_is_loaded: impl FnMut(&str) -> Option<bool>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let current_uid = unsafe { libc::geteuid() };

    let mut orphan_checks = 0;
    for entry in entries.take(ORPHAN_SCAN_ENTRY_LIMIT).flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some((pid, nonce)) = parse_tracker_script_filename(name) else {
            continue;
        };
        if process_is_alive(pid) {
            continue;
        }

        orphan_checks += 1;
        if orphan_checks > ORPHAN_CHECK_LIMIT {
            break;
        }

        // Pin the file without following a last-moment symlink swap, then
        // validate the opened inode rather than path metadata. Bound the read
        // to the exact generated script length so a forged file cannot cause
        // an unbounded allocation during daemon startup.
        let Ok(mut file) = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
        else {
            continue;
        };
        let Ok(metadata) = file.metadata() else {
            continue;
        };
        let bus_name = format!("org.wayexpand.WindowTracker.pid{pid}.n{nonce}");
        let expected_contents = SCRIPT_TEMPLATE.replace("__WAYEXPAND_BUS_NAME__", &bus_name);
        if !metadata.is_file()
            || metadata.uid() != current_uid
            || metadata.nlink() != 1
            || metadata.mode() & 0o777 != 0o600
            || metadata.len() != expected_contents.len() as u64
        {
            continue;
        }

        let mut contents = String::with_capacity(expected_contents.len());
        if Read::by_ref(&mut file)
            .take(expected_contents.len() as u64 + 1)
            .read_to_string(&mut contents)
            .is_err()
            || contents != expected_contents
        {
            continue;
        }

        let plugin_name = format!("wayexpand-window-tracker-{pid}-{nonce}");
        if script_is_loaded(&plugin_name) == Some(false) {
            // Remove only if the directory entry still names the inode that
            // was inspected. This avoids unlinking a replacement file after
            // the descriptor check above.
            let Ok(current) = fs::symlink_metadata(&path) else {
                continue;
            };
            if current.dev() == metadata.dev()
                && current.ino() == metadata.ino()
                && !current.file_type().is_symlink()
            {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// Parse only the exact filename shape emitted by `KwinWindowTracker::new`.
fn parse_tracker_script_filename(name: &str) -> Option<(u32, String)> {
    let name = name.strip_prefix("wayexpand-window-tracker-")?;
    let (pid, suffix) = name.split_once('-')?;
    let (nonce, repeated_nonce) = suffix.strip_suffix(".js")?.split_once('-')?;
    let pid = pid.parse::<u32>().ok()?;
    if pid == 0
        || nonce.is_empty()
        || nonce.len() > 16
        || !nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || nonce != repeated_nonce
    {
        return None;
    }
    Some((pid, nonce.to_owned()))
}

#[derive(Debug)]
enum HealthOutcome {
    /// KWin answered whether the script is loaded.
    Answered(bool),
    /// No answer yet, but still within the grace period.
    Unanswered,
    /// No answer for longer than the grace period.
    Lost(zbus::Error),
}

fn health_outcome(
    answer: Result<bool, zbus::Error>,
    last_healthy: Instant,
    now: Instant,
) -> HealthOutcome {
    match answer {
        Ok(loaded) => HealthOutcome::Answered(loaded),
        Err(_) if now.saturating_duration_since(last_healthy) < TRACKER_HEALTH_GRACE => {
            HealthOutcome::Unanswered
        }
        Err(error) => HealthOutcome::Lost(error),
    }
}

/// Build the session-bus connection without allowing authentication or socket
/// setup to strand the tracker supervisor/UI worker indefinitely. `race`
/// drops the unfinished connection future when the deadline wins.
fn bounded_session_connection<'a>(
    builder: zbus::connection::Builder<'a>,
    timeout: Duration,
) -> Result<Connection, KwinWindowError> {
    let connect = async move {
        builder
            .build()
            .await
            .map(Connection::from)
            .map_err(KwinWindowError::DBus)
    };
    let timeout = async {
        async_io::Timer::after(timeout).await;
        Err(KwinWindowError::ConnectionTimedOut)
    };
    futures_lite::future::block_on(futures_lite::future::race(connect, timeout))
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
                let answer: Result<bool, zbus::Error> = self
                    .connection
                    .call_method(
                        Some("org.kde.KWin"),
                        "/Scripting",
                        Some("org.kde.kwin.Scripting"),
                        "isScriptLoaded",
                        &(self.plugin_name.as_str(),),
                    )
                    .and_then(|reply| reply.body().deserialize());
                let loaded = match health_outcome(answer, self.last_healthy, Instant::now()) {
                    HealthOutcome::Answered(loaded) => {
                        self.last_healthy = Instant::now();
                        loaded
                    }
                    HealthOutcome::Unanswered => {
                        self.next_health_check = Instant::now() + TRACKER_HEALTH_CHECK_INTERVAL;
                        continue;
                    }
                    HealthOutcome::Lost(error) => {
                        return Err(WindowTrackerError {
                            backend: BACKEND_NAME,
                            message: format!(
                                "checking KWin tracker health failed for {} s: {error}",
                                TRACKER_HEALTH_GRACE.as_secs()
                            ),
                            retryable: true,
                        })
                    }
                };
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
    use super::{
        bounded_session_connection, cleanup_orphan_tracker_scripts, is_kwin_sender,
        parse_tracker_script_filename, window_context_from_signal, KwinWindowError,
        KwinWindowTracker, ORPHAN_CHECK_LIMIT, SCRIPT_TEMPLATE, TRACKER_HEALTH_CHECK_INTERVAL,
    };
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::{
        fs,
        io::{Read, Write},
        os::unix::net::UnixListener,
        path::Path,
        process, thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use wayexpand_core::WindowTracker;

    #[test]
    fn unanswered_health_checks_are_tolerated_for_the_grace_period() {
        use super::{health_outcome, HealthOutcome, TRACKER_HEALTH_GRACE};
        let healthy = Instant::now();
        let timeout = || zbus::Error::Failure("timed out".into());
        assert!(matches!(
            health_outcome(Ok(true), healthy, healthy + Duration::from_secs(60)),
            HealthOutcome::Answered(true)
        ));
        // A definite "not loaded" is reported at once, grace or not.
        assert!(matches!(
            health_outcome(Ok(false), healthy, healthy),
            HealthOutcome::Answered(false)
        ));
        // KWin stalls (memory pressure) for a few seconds: keep tracking.
        assert!(matches!(
            health_outcome(Err(timeout()), healthy, healthy + Duration::from_secs(4)),
            HealthOutcome::Unanswered
        ));
        assert!(matches!(
            health_outcome(Err(timeout()), healthy, healthy + TRACKER_HEALTH_GRACE),
            HealthOutcome::Lost(_)
        ));
    }

    #[test]
    fn empty_signal_represents_window_disappearance() {
        assert!(window_context_from_signal(String::new(), String::new(), String::new()).is_none());
    }

    #[test]
    fn missing_app_id_fails_closed_but_preserves_title() {
        let context =
            window_context_from_signal(String::new(), "Terminal".into(), String::new()).unwrap();
        assert_eq!(context.app_id, None);
        assert_eq!(context.title.as_deref(), Some("Terminal"));
    }

    #[test]
    fn missing_title_preserves_app_id() {
        let context =
            window_context_from_signal("org.example.Editor".into(), String::new(), String::new())
                .unwrap();
        assert_eq!(context.app_id.as_deref(), Some("org.example.Editor"));
        assert_eq!(context.title, None);
    }

    #[test]
    fn both_fields_are_preserved() {
        let context = window_context_from_signal(
            "org.example.Editor".into(),
            "Document".into(),
            "window-1".into(),
        )
        .unwrap();
        assert_eq!(context.app_id.as_deref(), Some("org.example.Editor"));
        assert_eq!(context.title.as_deref(), Some("Document"));
        assert_eq!(context.instance_id.as_deref(), Some("window-1"));
    }

    #[test]
    fn window_tracker_accepts_only_the_current_kwin_bus_owner() {
        let kwin_owner = ":1.42";
        assert!(is_kwin_sender(Some(kwin_owner), kwin_owner));
        assert!(!is_kwin_sender(Some(":1.43"), kwin_owner));
        assert!(!is_kwin_sender(None, kwin_owner));
    }

    #[test]
    fn orphan_tracker_cleanup_removes_only_exact_unloaded_private_files() {
        use std::os::unix::fs::symlink;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-kwin-orphan-cleanup-{}-{suffix}",
            process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();

        let stale = write_tracker_file(&directory, 410_001, "aabbccddeeff0011");
        let active = write_tracker_file(&directory, 410_002, "aabbccddeeff0022");
        let loaded = write_tracker_file(&directory, 410_003, "aabbccddeeff0033");
        let forged = write_tracker_file(&directory, 410_004, "aabbccddeeff0044");
        fs::write(&forged, "not the embedded WayExpand tracker script").unwrap();
        let target = directory.join("keep-me");
        fs::write(&target, "leave target unchanged").unwrap();
        let linked =
            directory.join("wayexpand-window-tracker-410005-aabbccddeeff0055-aabbccddeeff0055.js");
        symlink(&target, &linked).unwrap();

        cleanup_orphan_tracker_scripts(
            &directory,
            |pid| pid == 410_002,
            |plugin| {
                if plugin.contains("410003") {
                    Some(true)
                } else {
                    Some(false)
                }
            },
        );

        assert!(!stale.exists(), "a confirmed orphan should be removed");
        assert!(active.exists(), "a live process's file must be preserved");
        assert!(loaded.exists(), "a loaded KWin script must be preserved");
        assert!(
            forged.exists(),
            "unrecognized file contents must be preserved"
        );
        assert!(linked.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(
            fs::read_to_string(target).unwrap(),
            "leave target unchanged"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn tracker_script_filename_parser_rejects_ambiguous_names() {
        assert_eq!(
            parse_tracker_script_filename(
                "wayexpand-window-tracker-42-aabbccddeeff0011-aabbccddeeff0011.js"
            ),
            Some((42, "aabbccddeeff0011".into()))
        );
        assert_eq!(
            parse_tracker_script_filename("wayexpand-window-tracker-42-a-a.js"),
            Some((42, "a".into()))
        );
        for invalid in [
            "wayexpand-window-tracker-0-aabb-aabb.js",
            "wayexpand-window-tracker-42-aabb-bbcc.js",
            "wayexpand-window-tracker-42-not-hex-not-hex.js",
            "wayexpand-window-tracker-42-1234567890abcdef0-1234567890abcdef0.js",
            "wayexpand-window-tracker-42-aabb-aabb.js.bak",
        ] {
            assert_eq!(parse_tracker_script_filename(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn orphan_cleanup_bounds_synchronous_compositor_checks() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-kwin-orphan-limit-{}-{suffix}",
            process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        for index in 0..8 {
            write_tracker_file(
                &directory,
                420_000 + index,
                &format!("aabbccddeeff{index:04x}"),
            );
        }

        let checks = std::cell::Cell::new(0);
        cleanup_orphan_tracker_scripts(
            &directory,
            |_| false,
            |_| {
                checks.set(checks.get() + 1);
                Some(false)
            },
        );

        assert_eq!(checks.get(), ORPHAN_CHECK_LIMIT);
        assert_eq!(
            fs::read_dir(&directory).unwrap().count(),
            8 - ORPHAN_CHECK_LIMIT
        );
        fs::remove_dir_all(directory).unwrap();
    }

    fn write_tracker_file(directory: &Path, pid: u32, nonce: &str) -> std::path::PathBuf {
        let plugin = format!("wayexpand-window-tracker-{pid}-{nonce}");
        let path = directory.join(format!("{plugin}-{nonce}.js"));
        let bus_name = format!("org.wayexpand.WindowTracker.pid{pid}.n{nonce}");
        let contents = SCRIPT_TEMPLATE.replace("__WAYEXPAND_BUS_NAME__", &bus_name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn session_bus_authentication_is_cancelled_at_connection_deadline() {
        let socket_path = std::env::temp_dir().join(format!(
            "wayexpand-kwin-timeout-{}-{}.sock",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listener = UnixListener::bind(&socket_path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut ignored = Vec::new();
            let _ = stream.read_to_end(&mut ignored);
        });
        let address = format!("unix:path={}", socket_path.display());
        let builder = zbus::connection::Builder::address(address.as_str()).unwrap();
        let started = Instant::now();
        let result = bounded_session_connection(builder, Duration::from_millis(100));

        assert!(matches!(result, Err(KwinWindowError::ConnectionTimedOut)));
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
        std::fs::remove_file(socket_path).unwrap();
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
        let initial_window = initial
            .flatten()
            .expect("KWin should identify its initially focused window");
        assert!(
            initial_window
                .instance_id
                .as_deref()
                .is_some_and(|identity| !identity.is_empty()),
            "KWin should provide a strong identity for the exact focused window"
        );
        assert_eq!(
            std::fs::metadata(&tracker.script_path).unwrap().mode() & 0o777,
            0o600,
            "tracker script files must be private regardless of the process umask"
        );

        let untrusted_client =
            zbus::blocking::Connection::session().expect("connect a second session-bus client");
        untrusted_client
            .call_method(
                Some(tracker.service_name.as_str()),
                "/WindowTracker",
                Some("org.wayexpand.WindowTracker1"),
                "WindowChanged",
                &("org.example.Spoof", "fake-window", "not-a-kwin-id"),
            )
            .expect("the tracker method should reject an untrusted caller without disconnecting");
        assert!(
            tracker
                .next_window_timeout(std::time::Duration::from_millis(100))
                .expect("the tracker should stay responsive after a spoof attempt")
                .is_none(),
            "an untrusted D-Bus caller must not publish a focus event"
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
