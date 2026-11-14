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
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc, Mutex,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use tracing::info;
use wayexpand_core::{
    window_update_channel, ProcessIdentityState, WindowContext, WindowIdentitySource,
    WindowTracker, WindowTrackerError, WindowUpdateReceiver, WindowUpdateSender,
};
use zbus::{blocking::Connection, interface};

const BACKEND_NAME: &str = "kwin-window";
const SCRIPT_TEMPLATE: &str = include_str!("window-tracker.js");
const TRACKER_SETUP_TIMEOUT: Duration = Duration::from_secs(12);
/// Give KWin a short opportunity to publish its initial focus state. Missing
/// this snapshot must not tear down the script: it remains subscribed and can
/// report the next focus change while app-scoped matching stays fail-closed.
const INITIAL_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(4);
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
static REJECTED_KWIN_CALLBACKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn identity_source_from_label(source: &str) -> WindowIdentitySource {
    match source {
        "resource-class" => WindowIdentitySource::ResourceClass,
        "desktop-file-name" => WindowIdentitySource::DesktopFileName,
        "resource-name" => WindowIdentitySource::ResourceName,
        "compositor-identity-only" => WindowIdentitySource::CompositorIdentityOnly,
        "process-executable" => WindowIdentitySource::ProcessExecutable,
        "process-name" => WindowIdentitySource::ProcessName,
        _ => WindowIdentitySource::Unavailable,
    }
}

fn effective_identity_source(
    compositor_source: WindowIdentitySource,
    process_identity_requested: bool,
    process_executable_resolved: bool,
    process_name_resolved: bool,
) -> WindowIdentitySource {
    let needs_process_fallback = matches!(
        compositor_source,
        WindowIdentitySource::Unavailable | WindowIdentitySource::CompositorIdentityOnly
    );
    if process_identity_requested && needs_process_fallback {
        if process_executable_resolved {
            WindowIdentitySource::ProcessExecutable
        } else if process_name_resolved {
            WindowIdentitySource::ProcessName
        } else {
            WindowIdentitySource::ProcessExecutableUnavailable
        }
    } else {
        compositor_source
    }
}

#[derive(Debug, Error)]
pub enum KwinWindowError {
    #[error("D-Bus call failed: {0}")]
    DBus(#[from] zbus::Error),
    #[error("could not write the KWin tracker script: {0}")]
    ScriptWrite(#[source] std::io::Error),
    #[error("KWin did not finish registering the loaded script in time")]
    ScriptNotReady,
    #[error(
        "KWin loaded the tracker script but did not publish an initial focus snapshot in time"
    )]
    InitialSnapshotTimedOut,
    #[error("KWin failed before publishing its initial focus snapshot: {0}")]
    InitialSnapshotFailed(String),
    #[error("KWin tracker setup exceeded its time limit")]
    SetupTimedOut,
    #[error("session D-Bus connection setup exceeded its time limit")]
    ConnectionTimedOut,
    #[error("KWin no longer reports the WayExpand window-tracker script as loaded")]
    ScriptStopped,
    #[error("org.kde.KWin's scripting interface is not reachable on the session bus")]
    NotAvailable,
    #[error("could not generate a unique KWin tracker ID: {0}")]
    TrackerId(String),
    #[error("XDG_RUNTIME_DIR is missing or is not a private directory owned by this user")]
    InvalidRuntimeDirectory,
}

struct WindowTrackerService {
    sender: Mutex<WindowUpdateSender>,
    identity_source: std::sync::Arc<AtomicU8>,
    process_identity_state: std::sync::Arc<AtomicU8>,
    // KWin's callDBus() helper sends script callbacks over a distinct D-Bus
    // connection, so the message sender is not necessarily the connection
    // that owns org.kde.KWin. Authenticate either the owner name or a sender
    // whose bus credentials identify the same KWin process.
    kwin_owner: String,
    kwin_pid: u32,
    process_identity_enabled: std::sync::Arc<AtomicBool>,
}

fn is_kwin_sender(
    sender: Option<&str>,
    kwin_owner: &str,
    sender_pid: Option<u32>,
    kwin_pid: u32,
) -> bool {
    sender == Some(kwin_owner) || sender_pid == Some(kwin_pid)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExecutableLookupFailure {
    ProcessIdUnavailable,
    ProcessMetadataUnavailable,
    DifferentUser,
    ExecutablePermissionDenied,
    ExecutableInvalid,
    ProcessChanged,
    ExecutableLinkUnavailable,
}

impl ExecutableLookupFailure {
    fn diagnostic_state(self) -> ProcessIdentityState {
        match self {
            Self::ProcessIdUnavailable => ProcessIdentityState::ProcessIdUnavailable,
            Self::ProcessMetadataUnavailable => ProcessIdentityState::ProcessMetadataUnavailable,
            Self::DifferentUser => ProcessIdentityState::DifferentUser,
            Self::ExecutablePermissionDenied => ProcessIdentityState::ExecutablePermissionDenied,
            Self::ExecutableInvalid => ProcessIdentityState::ExecutableInvalid,
            Self::ProcessChanged => ProcessIdentityState::ProcessChanged,
            Self::ExecutableLinkUnavailable => ProcessIdentityState::ExecutableLinkUnavailable,
        }
    }
}

fn window_context_from_signal(
    app_id: String,
    title: String,
    instance_id: String,
    identity_source: WindowIdentitySource,
    process_executable: Option<String>,
    process_name: Option<String>,
) -> Option<WindowContext> {
    let identity_source_is_application = matches!(
        identity_source,
        WindowIdentitySource::ResourceClass
            | WindowIdentitySource::DesktopFileName
            | WindowIdentitySource::ResourceName
    );
    let mut app_id = (identity_source_is_application
        && !app_id.is_empty()
        && !is_kwin_compositor_app_id(&app_id)
        && !wayexpand_core::is_reserved_process_app_id(&app_id))
    .then_some(app_id);
    if app_id.is_none() {
        if let Some(path) = process_executable.filter(|path| is_valid_executable_path(path)) {
            app_id = Some(format!(
                "{}{}",
                wayexpand_core::PROCESS_EXECUTABLE_APP_ID_PREFIX,
                path
            ));
        }
    }
    if app_id.is_none() {
        if let Some(name) = process_name.filter(|name| is_valid_process_name(name)) {
            app_id = Some(format!(
                "{}{}",
                wayexpand_core::PROCESS_NAME_APP_ID_PREFIX,
                name.to_ascii_lowercase()
            ));
        }
    }
    if app_id.is_none() && title.is_empty() && instance_id.is_empty() {
        None
    } else {
        Some(WindowContext {
            app_id,
            title: (!title.is_empty()).then_some(title),
            instance_id: (!instance_id.is_empty()).then_some(instance_id),
        })
    }
}

/// Resolve an executable only after the callback has been authenticated as
/// KWin. PID and path values from any other D-Bus caller are never consulted.
/// The before/after start-time check rejects PID reuse during the lookup.
fn process_executable_path(pid: i32, expected_uid: u32) -> Result<String, ExecutableLookupFailure> {
    if pid <= 0 {
        return Err(ExecutableLookupFailure::ProcessIdUnavailable);
    }
    let process_dir = Path::new("/proc").join(pid.to_string());
    let before = process_start_time(&process_dir)
        .ok_or(ExecutableLookupFailure::ProcessMetadataUnavailable)?;
    let metadata = fs::metadata(&process_dir)
        .map_err(|_| ExecutableLookupFailure::ProcessMetadataUnavailable)?;
    if metadata.uid() != expected_uid {
        return Err(ExecutableLookupFailure::DifferentUser);
    }
    let executable = fs::read_link(process_dir.join("exe")).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            ExecutableLookupFailure::ExecutablePermissionDenied
        } else {
            ExecutableLookupFailure::ExecutableLinkUnavailable
        }
    })?;
    if !executable.is_absolute() {
        return Err(ExecutableLookupFailure::ExecutableInvalid);
    }
    let path = executable
        .to_str()
        .ok_or(ExecutableLookupFailure::ExecutableInvalid)?;
    // A package upgrade replaces the binary of every running instance, and
    // the kernel then reports "<path> (deleted)". The process was still
    // started from that path, so keep the identity while a valid executable
    // is installed there again; otherwise terminal-scoped expansion would stop
    // in every open terminal until it is restarted.
    let path = path.strip_suffix(" (deleted)").unwrap_or(path);
    if !is_valid_executable_path(path) {
        return Err(ExecutableLookupFailure::ExecutableInvalid);
    }
    if process_start_time(&process_dir) != Some(before) {
        return Err(ExecutableLookupFailure::ProcessChanged);
    }
    Ok(path.to_owned())
}

/// Read Linux's bounded task name only as a lower-assurance fallback. The
/// process can change this value; callers must require an explicit
/// `process_name_exact:` allowlist entry before it can authorize matching.
fn process_name(pid: i32, expected_uid: u32) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    let process_dir = Path::new("/proc").join(pid.to_string());
    let before = process_start_time(&process_dir)?;
    let metadata = fs::metadata(&process_dir).ok()?;
    if metadata.uid() != expected_uid {
        return None;
    }
    let name = fs::read_to_string(process_dir.join("comm")).ok()?;
    let name = name.trim_end_matches(['\r', '\n']);
    if !is_valid_process_name(name) || process_start_time(&process_dir)? != before {
        return None;
    }
    Some(name.to_ascii_lowercase())
}

/// Resolve a KWin-reported PID only when it can identify the client process.
/// Some compositor-owned surfaces report KWin's own PID; that PID must never
/// be turned into an application identity, including the lower-assurance
/// process-name fallback.
fn window_process_identity(
    process_id: i32,
    kwin_pid: u32,
    expected_uid: u32,
) -> (Option<String>, Option<String>, ProcessIdentityState) {
    if process_id > 0 && process_id as u32 == kwin_pid {
        return (None, None, ProcessIdentityState::CompositorProcessId);
    }
    match process_executable_path(process_id, expected_uid) {
        Ok(executable) => (
            Some(executable),
            None,
            ProcessIdentityState::ProcessExecutable,
        ),
        Err(failure) => (
            None,
            process_name(process_id, expected_uid),
            failure.diagnostic_state(),
        ),
    }
}

fn is_valid_process_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= wayexpand_core::limits::MAX_PROCESS_NAME_BYTES
        && name.is_ascii()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn process_start_time(process_dir: &Path) -> Option<u64> {
    let stat = fs::read_to_string(process_dir.join("stat")).ok()?;
    // comm is parenthesized and may itself contain spaces or ')'.
    let close = stat.rfind(')')?;
    let fields = stat
        .get(close + 1..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    fields.get(19)?.parse().ok() // field 22 (starttime), with field 3 at index 0
}

fn is_valid_executable_path(path: &str) -> bool {
    let candidate = Path::new(path);
    if !candidate.is_absolute()
        || path.chars().any(char::is_control)
        || candidate.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || path
            .split('/')
            .skip(1)
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        || path.chars().count() > wayexpand_core::limits::MAX_PROCESS_EXECUTABLE_PATH_CHARS
    {
        return false;
    }
    fs::metadata(candidate)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn is_kwin_compositor_app_id(app_id: &str) -> bool {
    let basename = app_id
        .rsplit('/')
        .next()
        .unwrap_or(app_id)
        .to_ascii_lowercase();
    let basename = basename.strip_suffix(".desktop").unwrap_or(&basename);
    basename == "kwin_wayland" || basename == "kwin_x11"
}

#[interface(name = "org.wayexpand.WindowTracker1")]
impl WindowTrackerService {
    // D-Bus method arguments must remain positional and independently typed
    // to match the bundled KWin script's callback signature.
    #[allow(clippy::too_many_arguments)]
    async fn window_changed(
        &self,
        app_id: String,
        title: String,
        instance_id: String,
        identity_source: String,
        process_id: i32,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> bool {
        let sender = header.sender().map(|sender| sender.to_string());
        let sender_is_owner = sender.as_deref() == Some(self.kwin_owner.as_str());
        let sender_pid = if !sender_is_owner {
            if let Some(sender) = sender.as_deref() {
                connection
                    .call_method(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        Some("org.freedesktop.DBus"),
                        "GetConnectionUnixProcessID",
                        &(sender,),
                    )
                    .await
                    .ok()
                    .and_then(|reply| reply.body().deserialize::<u32>().ok())
            } else {
                None
            }
        } else {
            None
        };
        let sender_is_kwin = is_kwin_sender(
            sender.as_deref(),
            &self.kwin_owner,
            sender_pid,
            self.kwin_pid,
        );
        if !sender_is_kwin {
            // Emit one sanitized identity diagnostic per process. KWin may
            // dispatch script D-Bus calls through a helper connection; this
            // distinguishes that case from a missing script callback without
            // logging window titles or application IDs.
            if REJECTED_KWIN_CALLBACKS.fetch_add(1, Ordering::Relaxed) == 0 {
                tracing::warn!(
                    sender_present = sender.is_some(),
                    sender_is_owner,
                    sender_pid,
                    kwin_pid = self.kwin_pid,
                    "ignored an unauthenticated KWin window-tracker callback"
                );
            }
            return false;
        }
        let identity_source = identity_source_from_label(&identity_source);
        let process_identity_requested = self.process_identity_enabled.load(Ordering::Acquire);
        let process_identity_needed = process_identity_requested
            && matches!(
                identity_source,
                WindowIdentitySource::Unavailable | WindowIdentitySource::CompositorIdentityOnly
            );
        let (process_executable, process_name, process_identity_state) =
            if !process_identity_requested {
                (None, None, ProcessIdentityState::NotRequested)
            } else if !process_identity_needed {
                (None, None, ProcessIdentityState::CompositorApplicationId)
            } else {
                window_process_identity(process_id, self.kwin_pid, unsafe { libc::geteuid() })
            };
        let effective_identity_source = effective_identity_source(
            identity_source,
            process_identity_requested,
            process_executable.is_some(),
            process_name.is_some(),
        );
        self.identity_source
            .store(effective_identity_source.code(), Ordering::Release);
        self.process_identity_state
            .store(process_identity_state.code(), Ordering::Release);
        let context = window_context_from_signal(
            app_id,
            title,
            instance_id,
            identity_source,
            process_executable,
            process_name,
        );
        info!(
            has_context = context.is_some(),
            has_app_id = context
                .as_ref()
                .is_some_and(|window| window.app_id.is_some()),
            has_title = context
                .as_ref()
                .is_some_and(|window| window.title.is_some()),
            has_instance_id = context
                .as_ref()
                .is_some_and(|window| window.instance_id.is_some()),
            identity_source = effective_identity_source.as_str(),
            process_identity_state = process_identity_state.as_str(),
            "accepted a KWin focus update"
        );
        // The receiver may already be gone if the tracker was dropped
        // between the script firing and this call landing; that is not an
        // error, there is simply nothing left to notify. A poisoned mutex
        // (some other panic while holding the lock) is treated the same way
        // rather than propagating the panic here: this callback runs on
        // every D-Bus dispatch, so unwrap()-ing would permanently break
        // app_filter-scoped snippets on the first poisoning instead of just
        // this one window-change notification.
        if let Ok(sender) = self.sender.lock() {
            return sender.send(context).is_ok();
        }
        false
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
    identity_source: std::sync::Arc<AtomicU8>,
    process_identity_state: std::sync::Arc<AtomicU8>,
    receiver: WindowUpdateReceiver,
    #[cfg(test)]
    service_name: String,
    plugin_name: String,
    script_path: std::path::PathBuf,
    initial_snapshot: Option<WindowContext>,
    next_health_check: Instant,
    /// When KWin last answered a health check.
    last_healthy: Instant,
}

impl KwinWindowTracker {
    /// Return the content-free KWin metadata source for the latest focused
    /// window identity report. This is diagnostic only and never changes
    /// fail-closed matching behavior.
    pub fn last_identity_source(&self) -> WindowIdentitySource {
        WindowIdentitySource::from_code(self.identity_source.load(Ordering::Acquire))
    }

    /// Return the sanitized outcome of KWin's latest optional `/proc`
    /// identity lookup. No process identifiers or names are retained here.
    pub fn last_process_identity_state(&self) -> ProcessIdentityState {
        ProcessIdentityState::from_code(self.process_identity_state.load(Ordering::Acquire))
    }

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

    pub fn new_with_process_identity(
        process_identity_enabled: std::sync::Arc<AtomicBool>,
    ) -> Result<Self, KwinWindowError> {
        Self::new_cancellable_with_process_identity(None, process_identity_enabled)
    }

    /// Like `new`, but allows an owning UI/task to stop the bounded script
    /// readiness retry loop. An in-flight D-Bus call is bounded by
    /// `DBUS_METHOD_TIMEOUT`.
    pub fn new_cancellable(cancelled: Option<&AtomicBool>) -> Result<Self, KwinWindowError> {
        Self::new_cancellable_with_process_identity(
            cancelled,
            std::sync::Arc::new(AtomicBool::new(false)),
        )
    }

    fn new_cancellable_with_process_identity(
        cancelled: Option<&AtomicBool>,
        process_identity_enabled: std::sync::Arc<AtomicBool>,
    ) -> Result<Self, KwinWindowError> {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(KwinWindowError::ScriptNotReady);
        }
        // KWin itself must be able to read this script. On systems that
        // enforce PrivateTmp=yes, a file under std::env::temp_dir() is
        // invisible to the compositor even though loadScript() can create a
        // script object for the path. XDG_RUNTIME_DIR is shared with the
        // compositor and remains private to this user.
        let script_directory = tracker_script_directory()?;
        let pid = process::id();
        // Reconnects may overlap cleanup of a failed tracker. Give each
        // attempt distinct D-Bus and KWin script identities so a delayed
        // unload cannot collide with the replacement instance.
        let mut tracker_id_bytes = [0_u8; 8];
        getrandom::fill(&mut tracker_id_bytes)
            .map_err(|error| KwinWindowError::TrackerId(error.to_string()))?;
        let tracker_id = u64::from_le_bytes(tracker_id_bytes);
        let bus_name = format!("org.wayexpand.WindowTracker.pid{pid}.n{tracker_id:x}");
        let (sender, receiver) = window_update_channel();
        let identity_source =
            std::sync::Arc::new(AtomicU8::new(WindowIdentitySource::Unavailable.code()));
        let process_identity_state =
            std::sync::Arc::new(AtomicU8::new(ProcessIdentityState::NotApplicable.code()));
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
        let kwin_pid: u32 = owner_connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetConnectionUnixProcessID",
                &(kwin_owner.as_str(),),
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
                        identity_source: std::sync::Arc::clone(&identity_source),
                        process_identity_state: std::sync::Arc::clone(&process_identity_state),
                        kwin_owner,
                        kwin_pid,
                        process_identity_enabled,
                    },
                )?,
            DBUS_CONNECTION_TIMEOUT,
        )?;

        let plugin_name = format!("wayexpand-window-tracker-{pid}-{tracker_id:x}");
        // The path is otherwise predictable (PID plus a fixed prefix, under
        // the user runtime directory), so another local process that guesses this
        // process's upcoming PID could pre-place a symlink here pointing at
        // a file this user owns elsewhere; `fs::write` follows symlinks and
        // would overwrite that target. A cryptographically random suffix makes
        // the exact path unguessable, and `create_new` (O_CREAT|O_EXCL) refuses
        // to open through anything already there -- symlink or not -- as defense
        // in depth against symlink attacks.
        let script_path = script_directory.join(format!("{plugin_name}-{tracker_id:x}.js"));
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
            &script_directory,
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
            |old_plugin| {
                connection
                    .call_method(
                        Some("org.kde.KWin"),
                        "/Scripting",
                        Some("org.kde.kwin.Scripting"),
                        "unloadScript",
                        &(old_plugin,),
                    )
                    .and_then(|reply| reply.body().deserialize())
                    .unwrap_or(false)
            },
        );

        let mut tracker = Self {
            connection,
            identity_source,
            process_identity_state,
            receiver,
            #[cfg(test)]
            service_name: bus_name,
            plugin_name,
            script_path,
            initial_snapshot: None,
            next_health_check: Instant::now() + TRACKER_HEALTH_CHECK_INTERVAL,
            last_healthy: Instant::now(),
        };
        // `loadScript()` and `run()` only prove that KWin accepted the script.
        // Preserve a usable initial context when available, but keep the
        // subscription alive if KWin has no snapshot yet: unloading it on a
        // timeout can miss the next focus transition and create repeated
        // gaps. The daemon still fails closed while focus is unknown.
        tracker.initial_snapshot = wait_for_initial_window(INITIAL_SNAPSHOT_TIMEOUT, |timeout| {
            tracker.next_window_timeout(timeout)
        })?;
        Ok(tracker)
    }

    /// Start the script through `/Scripting.start()`, never through the
    /// `/Scripting/ScriptN` object that `loadScript` names. KWin derives that
    /// ID from its list size, so after any unload it can repeat the ID of a
    /// script that is still loaded (observed on KWin 6.6: a load returned 7
    /// while another script held 7). `run()` then reaches the older script and
    /// reports success while the new one never starts, leaving a tracker that
    /// looks connected but never reports focus. `start()` runs every loaded
    /// script that is not already running, regardless of ID.
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
        // KWin answers -1 when this plugin name is already loaded.
        if script_id < 0 {
            return Err(KwinWindowError::ScriptNotReady);
        }
        connection.call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "start",
            &(),
        )?;
        Ok(())
    }
}

fn tracker_script_directory() -> Result<std::path::PathBuf, KwinWindowError> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .ok_or(KwinWindowError::InvalidRuntimeDirectory)?;
    validate_tracker_script_directory(&runtime_dir)?;
    Ok(runtime_dir)
}

fn validate_tracker_script_directory(path: &Path) -> Result<(), KwinWindowError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| KwinWindowError::InvalidRuntimeDirectory)?;
    if !path.is_absolute()
        || metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.mode() & 0o700 != 0o700
    {
        return Err(KwinWindowError::InvalidRuntimeDirectory);
    }
    Ok(())
}

/// Remove temporary files left by WayExpand tracker processes that exited
/// without running `Drop` (for example, after SIGKILL or a process crash).
/// It unloads only an exact generated script whose owner PID is dead and whose
/// private file still matches the generated contents, then removes that file
/// only after KWin confirms the script is no longer loaded.
fn cleanup_orphan_tracker_scripts(
    directory: &Path,
    mut process_is_alive: impl FnMut(u32) -> bool,
    mut script_is_loaded: impl FnMut(&str) -> Option<bool>,
    mut unload_script: impl FnMut(&str) -> bool,
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
        let Some((pid, tracker_id)) = parse_tracker_script_filename(name) else {
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
        let bus_name = format!("org.wayexpand.WindowTracker.pid{pid}.n{tracker_id}");
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

        let plugin_name = format!("wayexpand-window-tracker-{pid}-{tracker_id}");
        let unloaded = match script_is_loaded(&plugin_name) {
            Some(false) => true,
            // KWin's unloadScript return value acknowledges whether the
            // requested script was successfully unloaded. Do not add another
            // synchronous round trip to verify the same operation.
            Some(true) => unload_script(&plugin_name),
            None => false,
        };
        if unloaded {
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
    let (tracker_id, repeated_tracker_id) = suffix.strip_suffix(".js")?.split_once('-')?;
    let pid = pid.parse::<u32>().ok()?;
    if pid == 0
        || tracker_id.is_empty()
        || tracker_id.len() > 16
        || !tracker_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || tracker_id != repeated_tracker_id
    {
        return None;
    }
    Some((pid, tracker_id.to_owned()))
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

/// Wait for a usable focused-window context. KWin may first report an empty
/// focus transition while its startup state is settling; accepting that as
/// readiness leaves app-scoped matching unavailable even though the tracker
/// claims to be connected. Keep waiting within the same bounded startup
/// deadline until a real window context arrives.
fn wait_for_initial_window(
    timeout: Duration,
    mut next: impl FnMut(Duration) -> Result<Option<Option<WindowContext>>, WindowTrackerError>,
) -> Result<Option<WindowContext>, KwinWindowError> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        match next(remaining)
            .map_err(|error| KwinWindowError::InitialSnapshotFailed(error.to_string()))?
        {
            Some(Some(window)) => return Ok(Some(window)),
            Some(None) => continue,
            None => return Ok(None),
        }
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

    fn identity_source(&self) -> WindowIdentitySource {
        self.last_identity_source()
    }

    fn process_identity_state(&self) -> ProcessIdentityState {
        self.last_process_identity_state()
    }

    fn next_window_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<Option<WindowContext>>, WindowTrackerError> {
        if let Some(initial_snapshot) = self.initial_snapshot.take() {
            return Ok(Some(Some(initial_snapshot)));
        }
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
        bounded_session_connection, cleanup_orphan_tracker_scripts, effective_identity_source,
        is_kwin_compositor_app_id, is_kwin_sender, is_valid_executable_path,
        parse_tracker_script_filename, validate_tracker_script_directory, wait_for_initial_window,
        window_context_from_signal, KwinWindowError, KwinWindowTracker, ORPHAN_CHECK_LIMIT,
        SCRIPT_TEMPLATE, TRACKER_HEALTH_CHECK_INTERVAL,
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
    use wayexpand_core::WindowIdentitySource;
    use wayexpand_core::{WindowContext, WindowTracker};

    #[test]
    fn process_identity_failure_is_distinct_from_unrequested_lookup() {
        assert_eq!(
            effective_identity_source(
                WindowIdentitySource::CompositorIdentityOnly,
                true,
                false,
                false,
            ),
            WindowIdentitySource::ProcessExecutableUnavailable
        );
        assert_eq!(
            effective_identity_source(
                WindowIdentitySource::CompositorIdentityOnly,
                true,
                true,
                false,
            ),
            WindowIdentitySource::ProcessExecutable
        );
        assert_eq!(
            effective_identity_source(
                WindowIdentitySource::CompositorIdentityOnly,
                true,
                false,
                true,
            ),
            WindowIdentitySource::ProcessName
        );
        assert_eq!(
            effective_identity_source(
                WindowIdentitySource::CompositorIdentityOnly,
                false,
                false,
                false,
            ),
            WindowIdentitySource::CompositorIdentityOnly
        );
        assert_eq!(
            effective_identity_source(WindowIdentitySource::DesktopFileName, true, false, false,),
            WindowIdentitySource::DesktopFileName
        );
    }

    #[test]
    fn initial_empty_focus_report_does_not_mark_tracker_ready() {
        let expected = WindowContext {
            app_id: Some("org.kde.konsole".into()),
            title: Some("Terminal".into()),
            instance_id: Some("window-1".into()),
        };
        let mut reports = [Some(None), Some(Some(expected.clone()))].into_iter();

        let actual = wait_for_initial_window(Duration::from_secs(1), |_| {
            Ok(reports.next().expect("the initial focus report sequence"))
        })
        .unwrap();

        assert_eq!(actual, Some(expected));
    }

    #[test]
    fn missing_initial_focus_does_not_fail_the_tracker_connection() {
        let result = wait_for_initial_window(Duration::from_secs(1), |_| Ok(Some(None)));

        assert_eq!(result.unwrap(), None);
    }

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
        assert!(window_context_from_signal(
            String::new(),
            String::new(),
            String::new(),
            WindowIdentitySource::Unavailable,
            None,
            None,
        )
        .is_none());
    }

    #[test]
    fn client_app_id_cannot_claim_a_reserved_process_identity() {
        for spoof in [
            "process-executable:/usr/bin/konsole",
            "Process-Executable:/usr/bin/konsole",
            "PROCESS-NAME:konsole",
        ] {
            let context = window_context_from_signal(
                spoof.into(),
                "Terminal".into(),
                "window-1".into(),
                WindowIdentitySource::ResourceClass,
                None,
                None,
            )
            .unwrap();
            assert_eq!(context.app_id, None, "{spoof}");
        }
    }

    #[test]
    fn missing_app_id_fails_closed_but_preserves_title() {
        let context = window_context_from_signal(
            String::new(),
            "Terminal".into(),
            String::new(),
            WindowIdentitySource::Unavailable,
            None,
            None,
        )
        .unwrap();
        assert_eq!(context.app_id, None);
        assert_eq!(context.title.as_deref(), Some("Terminal"));
    }

    #[test]
    fn executable_identity_is_used_only_when_compositor_app_identity_is_unavailable() {
        let executable = std::env::current_exe().unwrap().display().to_string();
        let context = window_context_from_signal(
            String::new(),
            "Terminal".into(),
            "window-1".into(),
            WindowIdentitySource::Unavailable,
            Some(executable.clone()),
            None,
        )
        .unwrap();
        assert_eq!(
            context.app_id.as_deref(),
            Some(
                format!(
                    "{}{}",
                    wayexpand_core::PROCESS_EXECUTABLE_APP_ID_PREFIX,
                    executable
                )
                .as_str()
            )
        );

        let context = window_context_from_signal(
            "org.kde.konsole".into(),
            "Terminal".into(),
            "window-1".into(),
            WindowIdentitySource::DesktopFileName,
            Some("/usr/bin/other-terminal".into()),
            None,
        )
        .unwrap();
        assert_eq!(context.app_id.as_deref(), Some("org.kde.konsole"));
    }

    #[test]
    fn invalid_or_untrusted_executable_paths_do_not_create_identity() {
        for path in ["relative/program", "/tmp/../bin/sh", "/definitely/missing"] {
            assert!(!super::is_valid_executable_path(path), "{path}");
        }
        let context = window_context_from_signal(
            String::new(),
            "Terminal".into(),
            String::new(),
            WindowIdentitySource::Unavailable,
            Some("relative/program".into()),
            None,
        )
        .unwrap();
        assert_eq!(context.app_id, None);
    }

    #[test]
    fn proc_executable_resolution_is_same_uid_and_pid_stable() {
        let uid = unsafe { libc::geteuid() };
        let path = super::process_executable_path(process::id() as i32, uid)
            .expect("the current test process has an executable identity");
        assert!(super::is_valid_executable_path(&path));
        assert_eq!(
            super::process_executable_path(process::id() as i32, uid.saturating_add(1)),
            Err(super::ExecutableLookupFailure::DifferentUser)
        );
        assert_eq!(
            super::process_executable_path(0, uid),
            Err(super::ExecutableLookupFailure::ProcessIdUnavailable)
        );
    }

    #[test]
    fn process_executable_failure_categories_are_content_free_and_stable() {
        use wayexpand_core::ProcessIdentityState;

        let cases = [
            (
                super::ExecutableLookupFailure::ProcessIdUnavailable,
                ProcessIdentityState::ProcessIdUnavailable,
            ),
            (
                super::ExecutableLookupFailure::ProcessMetadataUnavailable,
                ProcessIdentityState::ProcessMetadataUnavailable,
            ),
            (
                super::ExecutableLookupFailure::DifferentUser,
                ProcessIdentityState::DifferentUser,
            ),
            (
                super::ExecutableLookupFailure::ExecutablePermissionDenied,
                ProcessIdentityState::ExecutablePermissionDenied,
            ),
            (
                super::ExecutableLookupFailure::ExecutableLinkUnavailable,
                ProcessIdentityState::ExecutableLinkUnavailable,
            ),
            (
                super::ExecutableLookupFailure::ExecutableInvalid,
                ProcessIdentityState::ExecutableInvalid,
            ),
            (
                super::ExecutableLookupFailure::ProcessChanged,
                ProcessIdentityState::ProcessChanged,
            ),
        ];
        for (failure, expected) in cases {
            let state = failure.diagnostic_state();
            assert_eq!(state, expected);
            assert_eq!(ProcessIdentityState::from_code(state.code()), expected);
            assert!(!state.as_str().contains('/'));
        }
    }

    #[test]
    fn compositor_pid_is_never_used_as_application_identity() {
        use wayexpand_core::ProcessIdentityState;

        let pid = process::id();
        let (executable, name, state) =
            super::window_process_identity(pid as i32, pid, unsafe { libc::geteuid() });
        assert_eq!(executable, None);
        assert_eq!(name, None);
        assert_eq!(state, ProcessIdentityState::CompositorProcessId);
        assert_eq!(
            ProcessIdentityState::from_code(state.code()),
            ProcessIdentityState::CompositorProcessId
        );
        assert_eq!(state.as_str(), "compositor-process-id");
    }

    #[test]
    fn proc_name_resolution_is_same_uid_and_pid_stable() {
        let uid = unsafe { libc::geteuid() };
        let name = super::process_name(process::id() as i32, uid)
            .expect("the current test process has a kernel task name");
        assert!(super::is_valid_process_name(&name));
        assert_eq!(
            super::process_name(process::id() as i32, uid.saturating_add(1)),
            None
        );
        assert_eq!(super::process_name(0, uid), None);
        for invalid in ["", "bad/name", "bad name", "é", "1234567890123456"] {
            assert!(!super::is_valid_process_name(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn process_name_is_used_only_as_explicit_lower_assurance_identity() {
        let context = window_context_from_signal(
            String::new(),
            "Terminal".into(),
            "window-1".into(),
            WindowIdentitySource::CompositorIdentityOnly,
            None,
            Some("konsole".into()),
        )
        .unwrap();
        assert_eq!(context.app_id.as_deref(), Some("process-name:konsole"));

        let context = window_context_from_signal(
            "org.kde.konsole".into(),
            "Terminal".into(),
            "window-1".into(),
            WindowIdentitySource::DesktopFileName,
            None,
            Some("other-terminal".into()),
        )
        .unwrap();
        assert_eq!(context.app_id.as_deref(), Some("org.kde.konsole"));

        let context = window_context_from_signal(
            String::new(),
            "Terminal".into(),
            "window-1".into(),
            WindowIdentitySource::CompositorIdentityOnly,
            None,
            Some("bad/name".into()),
        )
        .unwrap();
        assert_eq!(context.app_id, None);
    }

    #[test]
    fn compositor_identity_never_becomes_a_focused_application_id() {
        for compositor in [
            "kwin_wayland",
            "KWIN_X11",
            "/usr/share/applications/kwin_wayland.desktop",
        ] {
            let context = window_context_from_signal(
                compositor.into(),
                "Terminal window".into(),
                "window-kwin".into(),
                WindowIdentitySource::CompositorIdentityOnly,
                None,
                None,
            )
            .expect("the title and exact window identity remain available");
            assert_eq!(context.app_id, None, "{compositor} is not an app identity");
            assert_eq!(context.title.as_deref(), Some("Terminal window"));
            assert_eq!(context.instance_id.as_deref(), Some("window-kwin"));
        }
        assert!(is_kwin_compositor_app_id("KWin_Wayland"));
        assert!(is_kwin_compositor_app_id("kwin_x11.desktop"));
        assert!(!is_kwin_compositor_app_id("org.kde.konsole"));
    }

    #[test]
    fn untrusted_or_missing_identity_provenance_never_authorizes_an_app_id() {
        for source in [
            WindowIdentitySource::Unavailable,
            WindowIdentitySource::CompositorIdentityOnly,
        ] {
            let context = window_context_from_signal(
                "org.kde.konsole".into(),
                "Terminal window".into(),
                "window-kwin".into(),
                source,
                None,
                None,
            )
            .expect("exact window context remains useful for non-app scoped diagnostics");
            assert_eq!(context.app_id, None, "{source:?} cannot authorize app ID");
            assert_eq!(context.instance_id.as_deref(), Some("window-kwin"));
        }
    }

    #[test]
    fn missing_title_preserves_app_id() {
        let context = window_context_from_signal(
            "org.example.Editor".into(),
            String::new(),
            String::new(),
            WindowIdentitySource::DesktopFileName,
            None,
            None,
        )
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
            WindowIdentitySource::ResourceClass,
            None,
            None,
        )
        .unwrap();
        assert_eq!(context.app_id.as_deref(), Some("org.example.Editor"));
        assert_eq!(context.title.as_deref(), Some("Document"));
        assert_eq!(context.instance_id.as_deref(), Some("window-1"));
    }

    #[test]
    fn window_tracker_accepts_only_the_current_kwin_process() {
        let kwin_owner = ":1.42";
        assert!(is_kwin_sender(Some(kwin_owner), kwin_owner, None, 42));
        assert!(is_kwin_sender(Some(":1.43"), kwin_owner, Some(42), 42));
        assert!(!is_kwin_sender(Some(":1.43"), kwin_owner, Some(99), 42));
        assert!(!is_kwin_sender(None, kwin_owner, None, 42));
    }

    #[test]
    fn tracker_script_directory_must_be_private_and_user_owned() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-kwin-runtime-dir-{}-{suffix}",
            process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(validate_tracker_script_directory(&directory).is_ok());

        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            validate_tracker_script_directory(&directory),
            Err(KwinWindowError::InvalidRuntimeDirectory)
        ));
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();

        let file = directory.join("not-a-directory");
        fs::write(&file, "x").unwrap();
        assert!(matches!(
            validate_tracker_script_directory(&file),
            Err(KwinWindowError::InvalidRuntimeDirectory)
        ));

        let symlink = directory.join("directory-link");
        std::os::unix::fs::symlink(&directory, &symlink).unwrap();
        assert!(matches!(
            validate_tracker_script_directory(&symlink),
            Err(KwinWindowError::InvalidRuntimeDirectory)
        ));
        fs::remove_dir_all(directory).unwrap();
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
            |_| false,
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
    fn orphan_tracker_cleanup_unloads_exact_script_for_a_dead_process() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-kwin-orphan-unload-{}-{suffix}",
            process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let stale = write_tracker_file(&directory, 410_006, "aabbccddeeff0066");
        let loaded = std::cell::Cell::new(true);
        let unload_calls = std::cell::Cell::new(0);

        cleanup_orphan_tracker_scripts(
            &directory,
            |_| false,
            |_| Some(loaded.get()),
            |_| {
                unload_calls.set(unload_calls.get() + 1);
                loaded.set(false);
                true
            },
        );

        assert_eq!(unload_calls.get(), 1);
        assert!(!loaded.get());
        assert!(
            !stale.exists(),
            "confirmed unloaded orphan should be removed"
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
            |_| false,
        );

        assert_eq!(checks.get(), ORPHAN_CHECK_LIMIT);
        assert_eq!(
            fs::read_dir(&directory).unwrap().count(),
            8 - ORPHAN_CHECK_LIMIT
        );
        fs::remove_dir_all(directory).unwrap();
    }

    fn write_tracker_file(directory: &Path, pid: u32, tracker_id: &str) -> std::path::PathBuf {
        let plugin = format!("wayexpand-window-tracker-{pid}-{tracker_id}");
        let path = directory.join(format!("{plugin}-{tracker_id}.js"));
        let bus_name = format!("org.wayexpand.WindowTracker.pid{pid}.n{tracker_id}");
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
        let accepted: bool = untrusted_client
            .call_method(
                Some(tracker.service_name.as_str()),
                "/WindowTracker",
                Some("org.wayexpand.WindowTracker1"),
                "WindowChanged",
                &(
                    "org.example.Spoof",
                    "fake-window",
                    "not-a-kwin-id",
                    "resource-class",
                    0_i32,
                ),
            )
            .expect("the tracker method should reject an untrusted caller without disconnecting")
            .body()
            .deserialize()
            .expect("the tracker should return an authorization result");
        assert!(
            !accepted,
            "an untrusted caller must receive an explicit refusal"
        );
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

    #[test]
    #[ignore = "requires a live KDE Plasma Wayland session and a focused application window"]
    fn live_kwin_tracker_requires_trustworthy_application_identity() {
        let process_identity_enabled =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut tracker = KwinWindowTracker::new_with_process_identity(process_identity_enabled)
            .expect("connect to the live KWin scripting API");
        let initial = tracker
            .next_window_timeout(std::time::Duration::from_secs(2))
            .expect("receive the initial active-window snapshot");
        assert!(
            initial.is_some(),
            "KWin should report its initial active-window snapshot"
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
        match tracker.last_identity_source() {
            WindowIdentitySource::ProcessExecutable => {
                let path = initial_window
                    .app_id
                    .as_deref()
                    .and_then(|app_id| {
                        app_id.strip_prefix(wayexpand_core::PROCESS_EXECUTABLE_APP_ID_PREFIX)
                    })
                    .expect("process identity source must carry its exact executable path");
                assert!(
                    is_valid_executable_path(path),
                    "KWin fallback must resolve a live same-user executable path"
                );
            }
            WindowIdentitySource::ProcessName => {
                panic!(
                    "KWin exposed only a mutable Linux process name (executable lookup: {}); this is insufficient for the trusted-identity certification gate",
                    tracker.last_process_identity_state().as_str()
                );
            }
            WindowIdentitySource::ProcessExecutableUnavailable
                if tracker.last_process_identity_state()
                    == wayexpand_core::ProcessIdentityState::CompositorProcessId =>
            {
                panic!(
                    "KWin reported its own PID for the focused compositor surface; focus a real application window and rerun the identity test"
                );
            }
            source => {
                assert!(
                    initial_window
                        .app_id
                        .as_deref()
                        .is_some_and(|app_id| !app_id.is_empty()),
                    "KWin must provide an app ID or verified process executable; metadata source: {}",
                    source.as_str()
                );
            }
        }
    }
}
