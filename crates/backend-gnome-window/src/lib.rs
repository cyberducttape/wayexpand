//! Focused-window tracking for GNOME (Mutter).
//!
//! Mutter exposes no Wayland protocol or public D-Bus API for the focused
//! window: `org.gnome.Shell.Introspect` answers only allowlisted callers,
//! and `org.gnome.Shell.Eval` is disabled outside unsafe mode. The bridge is
//! WayExpand's own GNOME Shell extension
//! (`gnome-extension/wayexpand-window-tracker@cyberducttape.github.io`),
//! which runs inside GNOME Shell. A tracker calls its `Subscribe()` method
//! and receives the current window in the reply; the extension then calls
//! `WindowChanged` on this process's private bus connection for each focus
//! or title change. Calls are accepted only from the process that owns
//! `org.gnome.Shell`, so another local client cannot forge window identity.

use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use wayexpand_core::{
    window_update_channel, WindowContext, WindowTracker, WindowTrackerError, WindowUpdateReceiver,
    WindowUpdateSender,
};
use zbus::{blocking::Connection, interface, zvariant::OwnedValue};

const BACKEND_NAME: &str = "gnome-window";
const SHELL_NAME: &str = "org.gnome.Shell";
const TRACKER_PATH: &str = "/org/wayexpand/ShellWindowTracker";
const TRACKER_IFACE: &str = "org.wayexpand.ShellWindowTracker1";
const EXTENSIONS_PATH: &str = "/org/gnome/Shell/Extensions";
const EXTENSIONS_IFACE: &str = "org.gnome.Shell.Extensions";
/// The extension protocol this tracker speaks.
const PROTOCOL_VERSION: u32 = 2;
const INPUT_SOURCE_CONTROL_VERSION: u32 = 1;
const DBUS_CONNECTION_TIMEOUT: Duration = Duration::from_secs(3);
const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(1);
const TRACKER_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// GNOME Shell answers D-Bus on its main thread, which can stall under load;
/// tolerate unanswered checks this long before treating the tracker as lost.
/// A definite "no such object" (extension disabled) ends it at once.
const TRACKER_HEALTH_GRACE: Duration = Duration::from_secs(10);

/// The extension's UUID (also its directory name).
pub const EXTENSION_UUID: &str = "wayexpand-window-tracker@cyberducttape.github.io";
const EXTENSION_METADATA: &str = include_str!(
    "../../../gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/metadata.json"
);
const EXTENSION_SCRIPT: &str = include_str!(
    "../../../gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/extension.js"
);
const EXTENSION_NOTIFICATION_QUEUE: &str = include_str!(
    "../../../gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/notification-queue.js"
);

/// The extension's files as `(file name, contents)`, so any installation
/// method can install the exact bundled version.
pub fn extension_files() -> [(&'static str, &'static str); 3] {
    [
        ("metadata.json", EXTENSION_METADATA),
        ("extension.js", EXTENSION_SCRIPT),
        ("notification-queue.js", EXTENSION_NOTIFICATION_QUEUE),
    ]
}

#[derive(Debug, Error)]
pub enum GnomeWindowError {
    #[error("D-Bus call failed: {0}")]
    DBus(#[from] zbus::Error),
    #[error("session D-Bus connection setup exceeded its time limit")]
    ConnectionTimedOut,
    #[error("GNOME Shell is not running on the session bus")]
    ShellNotRunning,
    #[error("the WayExpand GNOME Shell extension is not enabled (run `wayexpand setup`)")]
    ExtensionNotActive,
    #[error("the WayExpand GNOME Shell extension speaks protocol {0}; this WayExpand needs {PROTOCOL_VERSION}")]
    ProtocolMismatch(u32),
    #[error("the loaded WayExpand GNOME Shell extension does not expose a build identity; log out and back in to load the installed version")]
    BuildIdentityUnavailable,
    #[error("the loaded WayExpand GNOME Shell extension build does not match this WayExpand installation (loaded {0}); log out and back in")]
    BuildIdentityMismatch(String),
    #[error("GNOME Shell input-source control is unavailable; update or enable the WayExpand GNOME Shell extension")]
    InputSourceControlUnavailable,
    #[error("the WayExpand GNOME Shell extension speaks input-source protocol {0}; this WayExpand needs {INPUT_SOURCE_CONTROL_VERSION}")]
    InputSourceControlProtocolMismatch(u32),
    #[error("GNOME Shell did not activate the requested input source")]
    InputSourceActivationFailed,
    #[error("GNOME Shell restarted; the window tracker reconnects")]
    ShellRestarted,
    #[error("window tracking setup was cancelled")]
    Cancelled,
}

struct WindowTrackerService {
    sender: Mutex<WindowUpdateSender>,
    /// Session-bus methods are callable by any local client; accept focus
    /// reports only from the process that owned org.gnome.Shell at setup.
    shell_owner: String,
}

fn is_shell_sender(sender: Option<&str>, shell_owner: &str) -> bool {
    sender == Some(shell_owner)
}

fn window_context(app_id: String, title: String, instance_id: String) -> Option<WindowContext> {
    if app_id.is_empty() && title.is_empty() && instance_id.is_empty() {
        // Shell UI (overview, a menu) has focus, or no window does.
        None
    } else {
        Some(WindowContext {
            app_id: (!app_id.is_empty()).then_some(app_id),
            title: (!title.is_empty()).then_some(title),
            instance_id: (!instance_id.is_empty()).then(|| format!("mutter:{instance_id}")),
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
        if !is_shell_sender(
            header.sender().map(|sender| sender.as_str()),
            &self.shell_owner,
        ) {
            return;
        }
        // A dropped receiver or poisoned lock means nobody is listening any
        // more; this callback must never panic on the D-Bus dispatcher.
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(window_context(app_id, title, instance_id));
        }
    }
}

/// Build a session-bus connection without letting authentication or socket
/// setup strand the caller.
fn bounded_session_connection(
    builder: zbus::connection::Builder<'_>,
    timeout: Duration,
) -> Result<Connection, GnomeWindowError> {
    let connect = async move {
        builder
            .build()
            .await
            .map(Connection::from)
            .map_err(GnomeWindowError::DBus)
    };
    let timeout = async {
        async_io::Timer::after(timeout).await;
        Err(GnomeWindowError::ConnectionTimedOut)
    };
    futures_lite::future::block_on(futures_lite::future::race(connect, timeout))
}

fn plain_connection() -> Result<Connection, GnomeWindowError> {
    bounded_session_connection(
        zbus::connection::Builder::session()?.method_timeout(DBUS_METHOD_TIMEOUT),
        DBUS_CONNECTION_TIMEOUT,
    )
}

fn shell_owner(connection: &Connection) -> Result<String, GnomeWindowError> {
    connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &(SHELL_NAME,),
        )
        .and_then(|reply| reply.body().deserialize::<String>())
        .map_err(|_| GnomeWindowError::ShellNotRunning)
}

fn input_source_control_version(
    connection: &Connection,
    owner: &str,
) -> Result<u32, GnomeWindowError> {
    let reply = connection.call_method(
        Some(SHELL_NAME),
        TRACKER_PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(TRACKER_IFACE, "InputSourceControlVersion"),
    );
    let version = match reply {
        Ok(reply) => {
            if reply.header().sender().map(|name| name.as_str()) != Some(owner) {
                return Err(GnomeWindowError::ShellRestarted);
            }
            reply
                .body()
                .deserialize::<OwnedValue>()
                .ok()
                .and_then(|value| u32::try_from(value).ok())
        }
        Err(error) if is_missing_object(&error) => {
            return Err(GnomeWindowError::InputSourceControlUnavailable)
        }
        Err(error) => return Err(GnomeWindowError::DBus(error)),
    };
    match version {
        Some(INPUT_SOURCE_CONTROL_VERSION) => Ok(INPUT_SOURCE_CONTROL_VERSION),
        Some(other) => Err(GnomeWindowError::InputSourceControlProtocolMismatch(other)),
        None => Err(GnomeWindowError::InputSourceControlUnavailable),
    }
}

/// Read the active source from GNOME Shell's live InputSourceManager. The
/// deprecated gsettings `current` key is deliberately not consulted.
pub fn active_input_source() -> Result<(String, String), GnomeWindowError> {
    let connection = plain_connection()?;
    let owner = shell_owner(&connection)?;
    input_source_control_version(&connection, &owner)?;
    read_active_input_source(&connection, &owner)
}

/// Build identity reported by the currently loaded extension instance. GNOME
/// retains the extension metadata in memory across on-disk updates, so this
/// distinguishes the loaded code from newly installed files.
pub fn extension_build_identity() -> Result<String, GnomeWindowError> {
    let connection = plain_connection()?;
    let owner = shell_owner(&connection)?;
    read_build_identity(&connection, &owner)
}

fn read_build_identity(connection: &Connection, owner: &str) -> Result<String, GnomeWindowError> {
    let reply = connection.call_method(
        Some(SHELL_NAME),
        TRACKER_PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(TRACKER_IFACE, "BuildIdentity"),
    );
    let identity = match reply {
        Ok(reply) => {
            if reply.header().sender().map(|name| name.as_str()) != Some(owner) {
                return Err(GnomeWindowError::ShellRestarted);
            }
            reply
                .body()
                .deserialize::<OwnedValue>()
                .ok()
                .and_then(|value| String::try_from(value).ok())
        }
        Err(error) if is_missing_object(&error) => {
            return Err(GnomeWindowError::BuildIdentityUnavailable)
        }
        Err(error) => return Err(GnomeWindowError::DBus(error)),
    };
    identity
        .filter(|identity| !identity.is_empty())
        .ok_or(GnomeWindowError::BuildIdentityUnavailable)
}

fn ensure_current_build_identity(
    connection: &Connection,
    owner: &str,
) -> Result<(), GnomeWindowError> {
    let identity = read_build_identity(connection, owner)?;
    let expected = bundled_extension_build_identity();
    if identity == expected {
        Ok(())
    } else {
        Err(GnomeWindowError::BuildIdentityMismatch(identity))
    }
}

/// SHA-256 identity of the bundled extension implementation modules. The
/// metadata carries the same identity for GNOME Shell to expose from its
/// loaded copy, so stale imported modules are covered as well as extension.js.
pub fn bundled_extension_build_identity() -> String {
    use std::fmt::Write as _;
    let mut digest = Sha256::new();
    for (name, contents) in extension_files()
        .into_iter()
        .filter(|(name, _)| *name != "metadata.json")
    {
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(contents.as_bytes());
        digest.update([0xff]);
    }
    digest
        .finalize()
        .iter()
        .fold(String::from("sha256:"), |mut identity, byte| {
            let _ = write!(identity, "{byte:02x}");
            identity
        })
}

fn read_active_input_source(
    connection: &Connection,
    owner: &str,
) -> Result<(String, String), GnomeWindowError> {
    let reply = connection.call_method(
        Some(SHELL_NAME),
        TRACKER_PATH,
        Some(TRACKER_IFACE),
        "GetActiveInputSource",
        &(),
    )?;
    if reply.header().sender().map(|name| name.as_str()) != Some(owner) {
        return Err(GnomeWindowError::ShellRestarted);
    }
    reply
        .body()
        .deserialize::<(String, String)>()
        .map_err(GnomeWindowError::DBus)
}

/// Activate an input source through GNOME Shell's own InputSourceManager,
/// then independently read its active-source state back from that manager.
pub fn activate_input_source(kind: &str, id: &str) -> Result<(String, String), GnomeWindowError> {
    if !matches!(kind, "ibus" | "xkb") || id.is_empty() || id.len() > 256 {
        return Err(GnomeWindowError::InputSourceActivationFailed);
    }
    let connection = plain_connection()?;
    let owner = shell_owner(&connection)?;
    input_source_control_version(&connection, &owner)?;
    let reply = connection.call_method(
        Some(SHELL_NAME),
        TRACKER_PATH,
        Some(TRACKER_IFACE),
        "ActivateInputSource",
        &(kind, id),
    )?;
    if reply.header().sender().map(|name| name.as_str()) != Some(owner.as_str()) {
        return Err(GnomeWindowError::ShellRestarted);
    }
    let requested = (kind.to_owned(), id.to_owned());
    let acknowledged: (String, String) = reply.body().deserialize()?;
    let observed = read_active_input_source(&connection, &owner)?;
    if acknowledged != requested || observed != requested {
        return Err(GnomeWindowError::InputSourceActivationFailed);
    }
    Ok(observed)
}

/// What a `Version` property read says about the extension.
#[derive(Debug)]
enum VersionAnswer {
    Version(u32),
    /// The object or interface does not exist: the extension is disabled.
    Missing,
    /// No usable answer (timeout, transient error).
    Unanswered(zbus::Error),
}

fn read_version(connection: &Connection) -> VersionAnswer {
    let reply = connection.call_method(
        Some(SHELL_NAME),
        TRACKER_PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(TRACKER_IFACE, "Version"),
    );
    match reply {
        Ok(reply) => match reply
            .body()
            .deserialize::<OwnedValue>()
            .ok()
            .and_then(|value| u32::try_from(value).ok())
        {
            Some(version) => VersionAnswer::Version(version),
            None => VersionAnswer::Missing,
        },
        Err(error) if is_missing_object(&error) => VersionAnswer::Missing,
        Err(error) => VersionAnswer::Unanswered(error),
    }
}

fn is_missing_object(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownProperty"
                | "org.freedesktop.DBus.Error.InvalidArgs"
        ),
        _ => false,
    }
}

fn is_not_subscribed(error: &zbus::Error) -> bool {
    matches!(
        error,
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.DBus.Error.AccessDenied"
    )
}

/// A `WindowTracker` backed by the WayExpand GNOME Shell extension.
pub struct GnomeWindowTracker {
    /// Serves the callback interface the extension calls.
    connection: Connection,
    receiver: WindowUpdateReceiver,
    shell_owner: String,
    next_health_check: Instant,
    last_healthy: Instant,
}

impl GnomeWindowTracker {
    /// A side-effect-free check: GNOME Shell runs and the WayExpand
    /// extension answers with a compatible protocol version.
    pub fn probe() -> Result<(), GnomeWindowError> {
        let connection = plain_connection()?;
        let owner = shell_owner(&connection)?;
        match read_version(&connection) {
            VersionAnswer::Version(PROTOCOL_VERSION) => {
                ensure_current_build_identity(&connection, &owner)
            }
            VersionAnswer::Version(other) => Err(GnomeWindowError::ProtocolMismatch(other)),
            VersionAnswer::Missing => Err(GnomeWindowError::ExtensionNotActive),
            VersionAnswer::Unanswered(error) => Err(GnomeWindowError::DBus(error)),
        }
    }

    pub fn new() -> Result<Self, GnomeWindowError> {
        Self::new_cancellable(None)
    }

    /// Subscribe to the extension. `cancelled` lets a UI give up between the
    /// bounded setup steps.
    pub fn new_cancellable(cancelled: Option<&AtomicBool>) -> Result<Self, GnomeWindowError> {
        let check_cancelled = || {
            if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                Err(GnomeWindowError::Cancelled)
            } else {
                Ok(())
            }
        };
        check_cancelled()?;
        let shell_owner = shell_owner(&plain_connection()?)?;
        check_cancelled()?;
        let (sender, receiver) = window_update_channel();
        // Serve the callback before subscribing, so no update can arrive
        // before the interface exists.
        let connection = bounded_session_connection(
            zbus::connection::Builder::session()?
                .method_timeout(DBUS_METHOD_TIMEOUT)
                .serve_at(
                    "/WindowTracker",
                    WindowTrackerService {
                        sender: Mutex::new(sender.clone()),
                        shell_owner: shell_owner.clone(),
                    },
                )?,
            DBUS_CONNECTION_TIMEOUT,
        )?;
        check_cancelled()?;
        match read_version(&connection) {
            VersionAnswer::Version(PROTOCOL_VERSION) => {
                ensure_current_build_identity(&connection, &shell_owner)?;
            }
            VersionAnswer::Version(other) => return Err(GnomeWindowError::ProtocolMismatch(other)),
            VersionAnswer::Missing => return Err(GnomeWindowError::ExtensionNotActive),
            VersionAnswer::Unanswered(error) => return Err(GnomeWindowError::DBus(error)),
        }
        let reply = connection.call_method(
            Some(SHELL_NAME),
            TRACKER_PATH,
            Some(TRACKER_IFACE),
            "Subscribe",
            &(),
        )?;
        // The reply comes from whoever owns org.gnome.Shell now; if that
        // changed since setup began, start over rather than trust it.
        if reply.header().sender().map(|name| name.as_str()) != Some(shell_owner.as_str()) {
            return Err(GnomeWindowError::ShellRestarted);
        }
        let (app_id, title, instance_id): (String, String, String) = reply.body().deserialize()?;
        let _ = sender.send(window_context(app_id, title, instance_id));
        Ok(Self {
            connection,
            receiver,
            shell_owner,
            next_health_check: Instant::now() + TRACKER_HEALTH_CHECK_INTERVAL,
            last_healthy: Instant::now(),
        })
    }

    fn lost(message: String) -> WindowTrackerError {
        WindowTrackerError {
            backend: BACKEND_NAME,
            message,
            retryable: true,
        }
    }

    fn check_health(&mut self) -> Result<(), WindowTrackerError> {
        // A restarted GNOME Shell has a new owner, and the new extension
        // instance does not know this subscriber.
        match shell_owner(&self.connection) {
            Ok(owner) if owner == self.shell_owner => {}
            Ok(_) => return Err(Self::lost(GnomeWindowError::ShellRestarted.to_string())),
            Err(error) => return Err(Self::lost(error.to_string())),
        }
        match self.connection.call_method(
            Some(SHELL_NAME),
            TRACKER_PATH,
            Some(TRACKER_IFACE),
            "Ping",
            &(),
        ) {
            Ok(_) => {
                self.last_healthy = Instant::now();
                Ok(())
            }
            Err(error) if is_missing_object(&error) || is_not_subscribed(&error) => {
                Err(Self::lost(format!(
                    "GNOME window-tracker subscription is unavailable: {error}"
                )))
            }
            Err(error) if self.last_healthy.elapsed() >= TRACKER_HEALTH_GRACE => {
                Err(Self::lost(format!(
                    "GNOME Shell did not answer for {} s: {error}",
                    TRACKER_HEALTH_GRACE.as_secs()
                )))
            }
            Err(_) => Ok(()),
        }
    }
}

impl Drop for GnomeWindowTracker {
    fn drop(&mut self) {
        let _ = self.connection.call_method(
            Some(SHELL_NAME),
            TRACKER_PATH,
            Some(TRACKER_IFACE),
            "Unsubscribe",
            &(),
        );
    }
}

impl WindowTracker for GnomeWindowTracker {
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
            if now >= self.next_health_check {
                self.check_health()?;
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
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(Self::lost("window tracker channel closed".into()))
                }
            }
        }
    }
}

/// The extension's state as GNOME Shell reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionState {
    /// Loaded and running.
    Active,
    /// Installed and known to GNOME Shell, but turned off.
    Inactive,
    /// GNOME Shell failed to load it; the string is Shell's error text.
    Error(String),
    /// This GNOME Shell version is not listed in its metadata.
    OutOfDate,
    /// GNOME Shell does not know it: not installed, or installed after this
    /// login (GNOME Shell on Wayland loads new extensions at login).
    Unknown,
    /// Another state number (downloading, enabling, ...).
    Other(u32),
}

/// Shell-wide extension facts, for setup and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionStatus {
    pub state: ExtensionState,
    /// `false` when the user switched off all user extensions.
    pub user_extensions_enabled: bool,
}

fn extension_state_from_info(
    info: &std::collections::HashMap<String, OwnedValue>,
) -> ExtensionState {
    let number = |key: &str| {
        info.get(key).and_then(|value| {
            f64::try_from(value.clone())
                .ok()
                .or_else(|| u32::try_from(value.clone()).ok().map(f64::from))
        })
    };
    let Some(state) = number("state") else {
        return ExtensionState::Unknown;
    };
    // GNOME Shell's ExtensionState: ACTIVE/ENABLED 1, INACTIVE/DISABLED 2,
    // ERROR 3, OUT_OF_DATE 4, UNINSTALLED 99.
    match state as u32 {
        1 => ExtensionState::Active,
        2 => ExtensionState::Inactive,
        3 => ExtensionState::Error(
            info.get("error")
                .and_then(|value| String::try_from(value.clone()).ok())
                .unwrap_or_default(),
        ),
        4 => ExtensionState::OutOfDate,
        99 => ExtensionState::Unknown,
        other => ExtensionState::Other(other),
    }
}

/// Ask GNOME Shell about the WayExpand extension. Errors mean GNOME Shell
/// is not running or not answering.
pub fn extension_status() -> Result<ExtensionStatus, GnomeWindowError> {
    let connection = plain_connection()?;
    shell_owner(&connection)?;
    let info: std::collections::HashMap<String, OwnedValue> = connection
        .call_method(
            Some(SHELL_NAME),
            EXTENSIONS_PATH,
            Some(EXTENSIONS_IFACE),
            "GetExtensionInfo",
            &(EXTENSION_UUID,),
        )?
        .body()
        .deserialize()?;
    let user_extensions_enabled = connection
        .call_method(
            Some(SHELL_NAME),
            EXTENSIONS_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(EXTENSIONS_IFACE, "UserExtensionsEnabled"),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<OwnedValue>().ok())
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(true);
    Ok(ExtensionStatus {
        state: extension_state_from_info(&info),
        user_extensions_enabled,
    })
}

/// Ask GNOME Shell to enable the extension now. Returns whether Shell
/// accepted; Shell refuses extensions it has not loaded yet (installed
/// after this login).
pub fn enable_extension() -> Result<bool, GnomeWindowError> {
    let connection = plain_connection()?;
    shell_owner(&connection)?;
    Ok(connection
        .call_method(
            Some(SHELL_NAME),
            EXTENSIONS_PATH,
            Some(EXTENSIONS_IFACE),
            "EnableExtension",
            &(EXTENSION_UUID,),
        )?
        .body()
        .deserialize::<bool>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn empty_report_means_no_window_and_ids_are_namespaced() {
        assert_eq!(
            window_context(String::new(), String::new(), String::new()),
            None
        );
        let window =
            window_context("org.gnome.TextEditor".into(), "notes".into(), "42".into()).unwrap();
        assert_eq!(window.app_id.as_deref(), Some("org.gnome.TextEditor"));
        assert_eq!(window.title.as_deref(), Some("notes"));
        assert_eq!(window.instance_id.as_deref(), Some("mutter:42"));
        // A window without an app ID fails closed for app filters but keeps
        // its title for explicit title_contains filters.
        let untitled = window_context(String::new(), "dialog".into(), "7".into()).unwrap();
        assert!(untitled.app_id.is_none());
    }

    #[test]
    fn only_the_current_shell_owner_may_report_windows() {
        assert!(is_shell_sender(Some(":1.20"), ":1.20"));
        assert!(!is_shell_sender(Some(":1.99"), ":1.20"));
        assert!(!is_shell_sender(None, ":1.20"));
    }

    #[test]
    fn extension_states_follow_gnome_shell_numbers() {
        let info = |state: f64| {
            HashMap::from([(
                "state".to_owned(),
                OwnedValue::try_from(zbus::zvariant::Value::from(state)).unwrap(),
            )])
        };
        assert_eq!(
            extension_state_from_info(&info(1.0)),
            ExtensionState::Active
        );
        assert_eq!(
            extension_state_from_info(&info(2.0)),
            ExtensionState::Inactive
        );
        assert_eq!(
            extension_state_from_info(&info(4.0)),
            ExtensionState::OutOfDate
        );
        assert_eq!(
            extension_state_from_info(&info(99.0)),
            ExtensionState::Unknown
        );
        assert_eq!(
            extension_state_from_info(&HashMap::new()),
            ExtensionState::Unknown
        );
        assert!(matches!(
            extension_state_from_info(&info(3.0)),
            ExtensionState::Error(_)
        ));
    }

    #[test]
    fn missing_objects_are_definite_but_timeouts_are_not() {
        let missing = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.UnknownObject")
                .unwrap(),
            None,
            zbus::message::Message::method_call("/", "Ping")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(is_missing_object(&missing));
        let not_subscribed = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.AccessDenied")
                .unwrap(),
            None,
            zbus::message::Message::method_call("/", "Ping")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(is_not_subscribed(&not_subscribed));
        assert!(!is_missing_object(&zbus::Error::InputOutput(
            std::io::Error::from(std::io::ErrorKind::TimedOut).into()
        )));
        assert!(!is_not_subscribed(&zbus::Error::InputOutput(
            std::io::Error::from(std::io::ErrorKind::TimedOut).into()
        )));
    }

    #[test]
    fn bundled_extension_matches_this_tracker() {
        let [(_, metadata), (_, script), (_, queue)] = extension_files();
        assert!(metadata.contains(EXTENSION_UUID));
        assert!(script.contains(TRACKER_PATH));
        assert!(script.contains(TRACKER_IFACE));
        assert!(script.contains("org.wayexpand.WindowTracker1"));
        assert!(script.contains(&format!("PROTOCOL_VERSION = {PROTOCOL_VERSION}")));
        let build_id = bundled_extension_build_identity();
        assert!(metadata.contains(&format!("\"x-wayexpand-build-id\": \"{build_id}\"")));
        assert!(script.contains("get BuildIdentity()"));
        assert!(script.contains("this.metadata['x-wayexpand-build-id']"));
        assert!(script.contains("<method name=\"Ping\"/>"));
        assert!(script.contains("PingAsync"));
        assert!(script.contains("<method name=\"GetActiveInputSource\">"));
        assert!(script.contains("<method name=\"ActivateInputSource\">"));
        assert!(script.contains("InputSourceControlVersion"));
        assert!(script.contains("Keyboard.getInputSourceManager().currentSource"));
        assert!(!script.contains("org.gnome.desktop.input-sources current"));
        assert!(script.contains("LatestOnlyDelivery"));
        assert!(queue.contains("class LatestOnlyDelivery"));
    }
}
