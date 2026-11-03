use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::{Read, Write},
    os::fd::AsRawFd,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tracing::warn;

#[cfg(target_os = "linux")]
use crate::socket_security::open_socket_parent;
#[cfg(test)]
use crate::socket_security::socket_parent_mode_is_secure;
use crate::socket_security::{
    is_original_socket, is_owned_socket, secure_socket_path, validate_socket_parent,
};

/// Large enough for `insert ` plus a maximum-length (128 character) trigger.
const MAX_COMMAND_BYTES: usize = 1024;
const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(2);
const CONTROL_WORKERS: usize = 4;
/// The diagnostics worker is single-consumer and bounded independently of
/// the keyboard reactor.
const EXPLAIN_QUEUE_CAPACITY: usize = CONTROL_WORKERS;
const CONTROL_BUSY_RESPONSE: &[u8] = b"error=busy\nretryable=true\n";
const CONTROL_MUTATIONS_DISABLED_RESPONSE: &[u8] =
    b"error=policy\nmessage=control socket mutations are disabled by organization policy\n";
/// Bound how much request data and wall time the listener spends draining an
/// overloaded client before closing its socket.
const BUSY_DRAIN_READS: usize = 16;
const BUSY_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);

fn is_stale_socket_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
    )
}

pub struct ControlServer {
    pub reload_requested: Arc<AtomicBool>,
    pub stop_requested: Arc<AtomicBool>,
    pub pause_requested: Arc<AtomicBool>,
    /// Number of accepted connections rejected because all request workers
    /// were occupied. This is deliberately separate from reactor metrics so
    /// control-plane overload cannot consume keyboard data-plane capacity.
    control_rejected_total: Arc<AtomicU64>,
    /// A snippet trigger to insert at the cursor (quick-insert picker,
    /// `wayexpand insert`). Only the latest request is kept: an insert is a one-shot
    /// user action and a stale queued one must never fire later.
    insert_requested: Arc<Mutex<Option<InsertRequest>>>,
    focus_snapshot: Arc<Mutex<FocusSnapshot>>,
    explain_snapshot: Arc<Mutex<Option<ExplainSnapshot>>>,
    _explain_worker: Option<JoinHandle<()>>,
    usage_clear: Arc<Mutex<Option<crate::usage::UsageClearHandle>>>,
    status: Arc<Mutex<String>>,
    /// Wakes the reactor after a request that changes daemon state.
    waker: crate::waker::WakerSlot,
    path: Option<PathBuf>,
    socket_identity: Option<(u64, u64)>,
    listener: Option<JoinHandle<()>>,
    /// A second handle to the listening socket, used to wake a blocked
    /// accept at shutdown without depending on the socket path still
    /// existing.
    listener_socket: Option<UnixListener>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertRequest {
    pub trigger: String,
    pub focus_token: Option<String>,
    pub focus_generation: Option<u64>,
}

/// An `explain <text>` request waiting for the reactor.
pub struct ExplainRequest {
    pub text: String,
    pub json: bool,
    pub reply: std::sync::mpsc::SyncSender<String>,
}

/// Cheap immutable daemon state consumed by the diagnostics worker. The
/// worker owns the engine and refreshes runtime state from this snapshot.
#[derive(Clone)]
pub(crate) struct ExplainSnapshot {
    pub validated: Arc<wayexpand_core::ValidatedConfig>,
    pub backend: &'static str,
    pub capture: &'static str,
    pub output: &'static str,
    pub connection_detail: &'static str,
    pub output_connected: bool,
    pub capabilities: Option<wayexpand_core::InjectorCapabilities>,
    pub paused: bool,
    /// The control-socket pause request the reactor had applied when this
    /// snapshot was taken, and whether a profile switch was holding expansion.
    /// A newer request answers `explain` before the reactor catches up.
    pub requested_pause: bool,
    pub profile_switch_pending: bool,
    pub sensitive: bool,
    pub composition: bool,
    pub form_open: bool,
    pub window: Option<Arc<wayexpand_core::WindowContext>>,
    pub form_guarantee: wayexpand_core::ReplacementGuarantee,
    pub commands_disabled: bool,
    pub direct_commands_disabled: bool,
    pub title_matching_disabled: bool,
}

/// Bound the time a control client waits for a diagnostics-worker response.
const EXPLAIN_REPLY_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusSnapshot {
    pub generation: u64,
    pub token: Option<String>,
    pub exact_window_identity: bool,
}

impl ControlServer {
    /// A control server with no socket: flags and status are kept in memory
    /// only. Used when no runtime directory is available, and by tests.
    pub fn detached() -> Self {
        Self {
            reload_requested: Arc::new(AtomicBool::new(false)),
            stop_requested: Arc::new(AtomicBool::new(false)),
            pause_requested: Arc::new(AtomicBool::new(false)),
            control_rejected_total: Arc::new(AtomicU64::new(0)),
            insert_requested: Arc::new(Mutex::new(None)),
            focus_snapshot: Arc::new(Mutex::new(FocusSnapshot::default())),
            explain_snapshot: Arc::new(Mutex::new(None)),
            _explain_worker: None,
            usage_clear: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new("starting".into())),
            waker: Arc::default(),
            path: None,
            socket_identity: None,
            listener: None,
            listener_socket: None,
        }
    }

    /// Start the control socket. With `require_verified_insert`, an insert
    /// request without a focus target is refused before it reaches the
    /// reactor, so the client learns why nothing was inserted.
    pub fn start(
        require_verified_insert: bool,
        disable_control_socket_mutations: bool,
    ) -> Result<Self> {
        let Some(requested_path) = socket_path() else {
            return Ok(Self::detached());
        };
        Self::start_at(
            &requested_path,
            require_verified_insert,
            disable_control_socket_mutations,
        )
    }

    fn start_at(
        requested_path: &std::path::Path,
        require_verified_insert: bool,
        disable_control_socket_mutations: bool,
    ) -> Result<Self> {
        let path = secure_socket_path(requested_path)?;
        validate_socket_parent(&path)?;
        #[cfg(target_os = "linux")]
        let (_socket_parent, operation_path) = open_socket_parent(&path)?;
        #[cfg(not(target_os = "linux"))]
        let operation_path = path.clone();
        if let Ok(metadata) = fs::symlink_metadata(&operation_path) {
            match UnixStream::connect(&operation_path) {
                Ok(_) => bail!(
                    "another WayExpand daemon is already using {}",
                    path.display()
                ),
                Err(error) if is_stale_socket_error(&error) => {
                    let owner = rustix::process::geteuid().as_raw();
                    if !is_owned_socket(&metadata, owner) {
                        if !metadata.file_type().is_socket() {
                            bail!(
                                "refusing to remove non-socket control path {}",
                                path.display()
                            );
                        }
                        bail!(
                            "refusing to remove control socket {} owned by uid {}",
                            path.display(),
                            metadata.uid()
                        );
                    }
                    let identity = (metadata.dev(), metadata.ino());
                    let current = fs::symlink_metadata(&operation_path)
                        .with_context(|| format!("rechecking stale socket {}", path.display()))?;
                    if !is_original_socket(&current, identity, owner) {
                        bail!(
                            "control path {} changed while checking stale socket",
                            path.display()
                        );
                    }
                    fs::remove_file(&operation_path)
                        .with_context(|| format!("removing stale socket {}", path.display()))?
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "checking whether control socket {} is active",
                            path.display()
                        )
                    })
                }
            }
        }
        // `main` sets the process umask before any threads exist, so the
        // socket is private from bind onward without a process-wide umask
        // mutation while worker threads are active.
        let listener = UnixListener::bind(&operation_path)
            .with_context(|| format!("binding control socket {}", path.display()))?;
        let listener_socket = listener
            .try_clone()
            .context("duplicating the control socket for shutdown")?;
        fs::set_permissions(&operation_path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&operation_path)?;
        let socket_identity = Some((metadata.dev(), metadata.ino()));
        let reload_requested = Arc::new(AtomicBool::new(false));
        let stop_requested = Arc::new(AtomicBool::new(false));
        let pause_requested = Arc::new(AtomicBool::new(false));
        let control_rejected_total = Arc::new(AtomicU64::new(0));
        let insert_requested = Arc::new(Mutex::new(None));
        let focus_snapshot = Arc::new(Mutex::new(FocusSnapshot::default()));
        let explain_snapshot = Arc::new(Mutex::new(None));
        let (explain_requests, explain_receiver) =
            std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY);
        let worker_snapshot = Arc::clone(&explain_snapshot);
        let worker_pause = Arc::clone(&pause_requested);
        let explain_worker = thread::Builder::new()
            .name("wayexpand-diagnostics".into())
            .spawn(move || {
                explain_worker(
                    explain_receiver,
                    worker_snapshot,
                    !disable_control_socket_mutations,
                    worker_pause,
                )
            })
            .context("starting diagnostics worker")?;
        let usage_clear = Arc::new(Mutex::new(None));
        let explain_slot = explain_requests.clone();
        let usage_clear_slot = Arc::clone(&usage_clear);
        let status = Arc::new(Mutex::new("starting".into()));
        let waker: crate::waker::WakerSlot = Arc::default();
        let waker_slot = Arc::clone(&waker);
        let reload_flag = Arc::clone(&reload_requested);
        let stop_flag = Arc::clone(&stop_requested);
        let pause_flag = Arc::clone(&pause_requested);
        let insert_slot = Arc::clone(&insert_requested);
        let focus_slot = Arc::clone(&focus_snapshot);
        let status_flag = Arc::clone(&status);
        let active_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let active_requests_for_listener = Arc::clone(&active_requests);
        let rejected_for_listener = Arc::clone(&control_rejected_total);
        let listener = thread::Builder::new()
            .name("wayexpand-control-listener".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let stream = match stream {
                        Ok(stream) => stream,
                        // Shutdown (see Drop) makes accept fail on purpose.
                        Err(_) if stop_flag.load(Ordering::Acquire) => break,
                        Err(error) => match accept_failure(&error) {
                            AcceptFailure::Retry => continue,
                            AcceptFailure::Backoff => {
                                // Out of descriptors or memory: the pending
                                // connection stays queued, so wait instead of
                                // spinning on the same error.
                                warn!(%error, "control socket accept failed; retrying");
                                thread::sleep(ACCEPT_BACKOFF);
                                continue;
                            }
                            AcceptFailure::Fatal => {
                                warn!(%error, "control socket stopped accepting requests");
                                break;
                            }
                        },
                    };
                    // A connection made by ControlServer::drop only wakes
                    // accept so the listener can terminate. Do not admit it
                    // as a request after shutdown has been published.
                    if stop_flag.load(Ordering::Acquire) {
                        break;
                    }
                    let flags = Flags {
                        reload: Arc::clone(&reload_flag),
                        stop: Arc::clone(&stop_flag),
                        pause: Arc::clone(&pause_flag),
                        insert: Arc::clone(&insert_slot),
                        require_verified_insert,
                        disable_control_socket_mutations,
                        focus: Arc::clone(&focus_slot),
                        explain: explain_slot.clone(),
                        usage_clear: Arc::clone(&usage_clear_slot),
                        status: Arc::clone(&status_flag),
                        waker: Arc::clone(&waker_slot),
                    };
                    let admitted = active_requests_for_listener.fetch_update(
                        Ordering::AcqRel,
                        Ordering::Acquire,
                        |active| (active < CONTROL_WORKERS).then_some(active + 1),
                    );
                    if admitted.is_err() {
                        // Keep control-plane concurrency bounded. Return a
                        // structured response so clients can distinguish
                        // overload from a dead daemon, and count it without
                        // involving the reactor or keyboard data plane.
                        reject_busy(stream, &rejected_for_listener);
                        continue;
                    }
                    let active_requests = Arc::clone(&active_requests_for_listener);
                    // Keep a spare descriptor so a transient thread-resource
                    // failure can still return the same bounded busy response
                    // as request-pool saturation. `spawn` consumes its
                    // closure on failure, which would otherwise drop the only
                    // stream and leave clients with a broken pipe.
                    let busy_stream = stream.try_clone().ok();
                    let worker = thread::Builder::new()
                        .name("wayexpand-control-request".into())
                        .spawn(move || {
                            // The control plane is deliberately isolated from
                            // keyboard processing. A client that holds a socket open
                            // cannot head-of-line block later requests.
                            let _ = handle_request(stream, flags);
                            active_requests.fetch_sub(1, Ordering::AcqRel);
                        });
                    if let Err(error) = worker {
                        active_requests_for_listener.fetch_sub(1, Ordering::AcqRel);
                        warn!(%error, "could not start control request worker");
                        if let Some(stream) = busy_stream {
                            reject_busy(stream, &rejected_for_listener);
                        }
                    }
                    if stop_flag.load(Ordering::Acquire) {
                        break;
                    }
                }
            })
            .context("starting control socket listener")?;
        Ok(Self {
            reload_requested,
            stop_requested,
            pause_requested,
            control_rejected_total,
            insert_requested,
            focus_snapshot,
            explain_snapshot,
            _explain_worker: Some(explain_worker),
            usage_clear,
            status,
            waker,
            path: Some(path),
            socket_identity,
            listener: Some(listener),
            listener_socket: Some(listener_socket),
        })
    }

    pub fn set_explain_snapshot(&self, snapshot: ExplainSnapshot) {
        if let Ok(mut current) = self.explain_snapshot.lock() {
            *current = Some(snapshot);
        }
    }

    /// Take the pending insert request, if any.
    pub fn take_insert_request(&self) -> Option<InsertRequest> {
        self.insert_requested
            .lock()
            .map_or(None, |mut slot| slot.take())
    }

    pub fn set_focus_snapshot(&self, snapshot: FocusSnapshot) {
        if let Ok(mut current) = self.focus_snapshot.lock() {
            *current = snapshot;
        }
    }

    pub fn focus_snapshot(&self) -> FocusSnapshot {
        self.focus_snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_default()
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    /// Install the reactor's waker; requests that change daemon state then
    /// wake the loop immediately.
    pub fn set_waker(&self, waker: crate::waker::Waker) {
        let _ = self.waker.set(waker);
    }

    pub fn set_usage_clear_handle(&self, handle: crate::usage::UsageClearHandle) {
        if let Ok(mut slot) = self.usage_clear.lock() {
            *slot = Some(handle);
        }
    }

    /// Publishes a status body. Taking `StatusBody` rather than any string
    /// keeps `crate::status`'s builder the single producer of the documented
    /// control-socket field set.
    pub fn set_status(&self, status: crate::status::StatusBody) {
        if let Ok(mut current) = self.status.lock() {
            if current.as_str() != status.as_str() {
                *current = status.into_string();
            }
        }
    }

    /// The status body currently served to clients.
    #[cfg(test)]
    pub fn status_text(&self) -> String {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Release);
        // `incoming()` blocks in accept. Shutting the listening socket down
        // makes accept fail at once, even when the socket path is gone (a
        // cleaned runtime directory); connecting to the path would then fail
        // and leave the join below waiting forever. The connection remains a
        // fallback.
        let woken = self.listener_socket.as_ref().is_some_and(|socket| {
            rustix::net::shutdown(socket, rustix::net::Shutdown::Both).is_ok()
        });
        if !woken {
            if let Some(path) = &self.path {
                let _ = UnixStream::connect(path);
            }
        }
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        let rejected = self.control_rejected_total.load(Ordering::Relaxed);
        if rejected > 0 {
            tracing::info!(
                control_rejected_total = rejected,
                "control socket stopped after rejecting busy requests"
            );
        }
        if let (Some(path), Some(identity)) = (&self.path, self.socket_identity) {
            if let Ok(metadata) = fs::symlink_metadata(path) {
                let uid = rustix::process::geteuid().as_raw();
                if is_original_socket(&metadata, identity, uid) {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
}

/// How long the listener waits after a resource-exhaustion accept error.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

#[derive(Debug, PartialEq, Eq)]
enum AcceptFailure {
    /// The client went away or a signal interrupted accept.
    Retry,
    /// The process is out of descriptors or memory for now.
    Backoff,
    /// The listening socket itself is unusable.
    Fatal,
}

/// Classify an accept error. Ending the listener on any error, as before,
/// left the daemon without a control socket for the rest of its life after
/// one client reset its connection before it was accepted (ECONNABORTED) or
/// the process briefly ran out of descriptors.
fn accept_failure(error: &std::io::Error) -> AcceptFailure {
    use rustix::io::Errno;
    let Some(code) = error.raw_os_error() else {
        return AcceptFailure::Fatal;
    };
    match Errno::from_raw_os_error(code) {
        Errno::CONNABORTED | Errno::INTR | Errno::AGAIN | Errno::PROTO => AcceptFailure::Retry,
        Errno::MFILE | Errno::NFILE | Errno::NOBUFS | Errno::NOMEM => AcceptFailure::Backoff,
        _ => AcceptFailure::Fatal,
    }
}

mod diagnostics;
mod protocol;
use diagnostics::explain_worker;
#[cfg(test)]
use diagnostics::{explain_request, restricted_explain_response};
use protocol::{handle_request, reject_busy, Flags};
#[cfg(test)]
use protocol::{
    insert_request, is_mutating_request, peer_uid, peer_uid_is_authorized, read_command,
    validate_peer_credential_length, UNVERIFIED_INSERT_REFUSED,
};

pub fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WAYEXPAND_SOCKET") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
}

#[cfg(test)]
mod tests;
