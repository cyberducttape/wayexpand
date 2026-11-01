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

/// The shared state one control request may touch.
#[derive(Clone)]
struct Flags {
    reload: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    insert: Arc<Mutex<Option<InsertRequest>>>,
    require_verified_insert: bool,
    disable_control_socket_mutations: bool,
    focus: Arc<Mutex<FocusSnapshot>>,
    explain: std::sync::mpsc::SyncSender<ExplainRequest>,
    usage_clear: Arc<Mutex<Option<crate::usage::UsageClearHandle>>>,
    status: Arc<Mutex<String>>,
    waker: crate::waker::WakerSlot,
}

fn reject_busy(mut stream: UnixStream, rejected_total: &AtomicU64) {
    let total = rejected_total.fetch_add(1, Ordering::Relaxed) + 1;
    crate::counters::note_control_rejected();
    warn!(
        control_rejected_total = total,
        "control request rejected: worker pool busy"
    );
    let _ = stream.set_write_timeout(Some(Duration::from_millis(50)));
    let _ = stream.write_all(CONTROL_BUSY_RESPONSE);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    // Keep reading after replying so a client that connected before writing
    // its request does not get ECONNRESET when its bytes arrive. The total
    // deadline and byte budget keep a slow or oversized client from holding
    // the accept loop indefinitely.
    discard_unread_request(&mut stream);
}

/// Closing a Unix stream with unread request bytes makes the kernel report
/// ECONNRESET to the peer, which would hide the response behind a generic
/// read error. Drain the bounded one-line request, including bytes that arrive
/// just after accept, and return on EOF, newline, timeout, or the byte limit.
fn discard_unread_request(stream: &mut UnixStream) {
    let deadline = Instant::now() + BUSY_DRAIN_TIMEOUT;
    let mut discard = [0_u8; 1024];
    for _ in 0..BUSY_DRAIN_READS {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || stream.set_read_timeout(Some(remaining)).is_err() {
            break;
        }
        match stream.read(&mut discard) {
            Ok(0) => break,
            Ok(count) if discard[..count].contains(&b'\n') => break,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(_) => break,
        }
    }
}

/// Reply to an insert without a focus target under
/// `require_verified_insert_target`.
pub const UNVERIFIED_INSERT_REFUSED: &str =
    "insert refused: organization policy requires a verified insert target (use insert --verified)\n";

/// The trigger of an `insert <trigger>` request, taken verbatim: a
/// configured trigger may legitimately begin or end with a space. Control
/// characters are refused so the line protocol cannot be confused.
/// Returns `None` for any other command, `Some(None)` for an invalid trigger.
fn insert_request(line: &str) -> Option<Option<InsertRequest>> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line == "insert" {
        return Some(None);
    }
    if let Some(request) = line.strip_prefix("insert-target ") {
        let mut fields = request.splitn(3, ' ');
        let generation = fields.next()?.parse::<u64>().ok()?;
        let token = fields.next()?;
        let trigger = fields.next()?;
        let valid_token =
            !token.is_empty() && token.chars().all(|character| character.is_ascii_hexdigit());
        let valid_trigger =
            (1..=128).contains(&trigger.chars().count()) && !trigger.chars().any(char::is_control);
        return Some((valid_token && valid_trigger).then_some(InsertRequest {
            trigger: trigger.to_owned(),
            focus_token: Some(token.to_owned()),
            focus_generation: Some(generation),
        }));
    }
    let trigger = line.strip_prefix("insert ")?;
    let valid =
        (1..=128).contains(&trigger.chars().count()) && !trigger.chars().any(char::is_control);
    Some(valid.then_some(InsertRequest {
        trigger: trigger.to_owned(),
        focus_token: None,
        focus_generation: None,
    }))
}

/// Handle `explain <text>` / `explain-json <text>`: hand the text to the
/// reactor and wait for its answer. Returns `None` for other commands.
fn explain_request(
    line: &str,
    queue: &std::sync::mpsc::SyncSender<ExplainRequest>,
) -> Result<Option<String>> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let (text, json) = if let Some(text) = line.strip_prefix("explain-json ") {
        (text, true)
    } else if let Some(text) = line.strip_prefix("explain ") {
        (text, false)
    } else {
        return Ok(None);
    };
    if !(1..=256).contains(&text.chars().count()) || text.chars().any(char::is_control) {
        return Ok(Some("invalid text\n".into()));
    }
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    let request = ExplainRequest {
        text: text.to_owned(),
        json,
        reply,
    };
    if queue.try_send(request).is_err() {
        return Ok(Some(if json {
            "{\"error\":\"busy\",\"retryable\":true}\n".into()
        } else {
            "explain unavailable: daemon is busy; retry\n".into()
        }));
    }
    Ok(Some(
        answer
            .recv_timeout(EXPLAIN_REPLY_TIMEOUT)
            .unwrap_or_else(|_| "explain unavailable: the daemon did not answer in time\n".into()),
    ))
}

fn explain_worker(
    requests: std::sync::mpsc::Receiver<ExplainRequest>,
    snapshot: Arc<Mutex<Option<ExplainSnapshot>>>,
    detailed: bool,
    pause_requested: Arc<AtomicBool>,
) {
    let mut engine: Option<wayexpand_core::ExpansionEngine> = None;
    let mut validated: Option<Arc<wayexpand_core::ValidatedConfig>> = None;
    while let Ok(request) = requests.recv() {
        let started = Instant::now();
        let current = snapshot.lock().ok().and_then(|snapshot| snapshot.clone());
        let answer = current.map_or_else(
            || "explain unavailable: diagnostics are starting; retry\n".to_owned(),
            |state| {
                if validated
                    .as_ref()
                    .is_none_or(|previous| !Arc::ptr_eq(previous, &state.validated))
                {
                    engine = Some(wayexpand_core::ExpansionEngine::from_validated(Arc::clone(
                        &state.validated,
                    )));
                    validated = Some(Arc::clone(&state.validated));
                }
                let Some(engine) = engine.as_mut() else {
                    return "explain unavailable: diagnostics could not initialize\n".to_owned();
                };
                // `pause` and `resume` reply before the reactor applies
                // them; answer from the request rather than a stale snapshot.
                let requested_pause = pause_requested.load(Ordering::Acquire);
                engine.set_user_paused(if requested_pause == state.requested_pause {
                    state.paused
                } else {
                    requested_pause || state.profile_switch_pending
                });
                engine.set_sensitive_focus(state.sensitive);
                engine.set_composition_active(state.composition);
                engine.set_form_open_for_diagnostics(state.form_open);
                engine.set_current_window(state.window.as_deref().cloned());
                engine.set_form_replacement_guarantee(state.form_guarantee);
                engine.set_commands_disabled(state.commands_disabled);
                engine.set_direct_commands_disabled(state.direct_commands_disabled);
                engine.set_title_matching_disabled(state.title_matching_disabled);

                let mut explanation = engine.explain_diagnostic(&request.text, state.backend);
                explanation.push(
                    "daemon",
                    wayexpand_core::CheckStatus::Pass,
                    format!(
                        "running with {} capture and {} output",
                        state.capture, state.output
                    ),
                );
                if state.output == "none" {
                    explanation.push(
                        "output backend",
                        wayexpand_core::CheckStatus::Info,
                        "no output backend (test mode): matches are reported, not typed",
                    );
                } else if state.output_connected {
                    explanation.push(
                        "output backend",
                        wayexpand_core::CheckStatus::Pass,
                        "connected",
                    );
                    if explanation.snippet.is_some() {
                        let route_detail = state.capabilities.as_ref().map_or_else(
                            || "route-specific replacement compatibility is not evaluated by control-socket diagnostics".to_owned(),
                            |capabilities| format!(
                                "{} selected; replacement compatibility is not evaluated by control-socket diagnostics",
                                capabilities.insertion_mode
                            ),
                        );
                        explanation.push(
                            "output route",
                            wayexpand_core::CheckStatus::Info,
                            route_detail,
                        );
                    }
                } else {
                    explanation.push(
                        "output backend",
                        wayexpand_core::CheckStatus::Fail,
                        format!("not connected ({})", state.connection_detail),
                    );
                }
                if started.elapsed() > EXPLAIN_REPLY_TIMEOUT {
                    return if request.json {
                        "{\"error\":\"deadline\",\"retryable\":true}\n".to_owned()
                    } else {
                        "explain unavailable: diagnostics exceeded their time budget; retry\n"
                            .to_owned()
                    };
                }
                if detailed {
                    detailed_explain_response(&explanation, request.json)
                } else {
                    restricted_explain_response(&explanation, request.json)
                }
            },
        );
        let _ = request.reply.try_send(answer);
    }
}

/// Summary fields shared by the detailed and restricted responses.
fn explain_summary(explanation: &wayexpand_core::Explanation) -> (bool, bool, &'static str) {
    let matched = explanation.snippet.is_some();
    let unavailable = explanation
        .checks
        .iter()
        .any(|check| check.name == "diagnostics");
    let would_expand = matched && explanation.suppressed_by().is_none();
    let suppression_reason = if unavailable {
        "unavailable"
    } else if !matched {
        "no_match"
    } else if would_expand {
        "none"
    } else {
        "blocked"
    };
    (matched, would_expand, suppression_reason)
}

/// The full explanation for the user who owns the daemon. Anything this
/// reveals (triggers, checks, command paths) is already readable by that user
/// in their own configuration, and replacements are never rendered here.
fn detailed_explain_response(explanation: &wayexpand_core::Explanation, json: bool) -> String {
    let (matched, would_expand, suppression_reason) = explain_summary(explanation);
    if json {
        format!(
            "{}\n",
            serde_json::json!({
                "matched": matched,
                "would_expand": would_expand,
                "suppression_reason": suppression_reason,
                "suppressed_by": explanation.suppressed_by().map(|check| check.name),
                "typed": explanation.typed,
                "snippet": explanation.snippet,
                "checks": explanation.checks,
            })
        )
    } else {
        explanation.render_text()
    }
}

/// When administrator policy makes the control socket read-only, explain
/// answers only whether the text would expand, independent of configured
/// trigger names, checks, command metadata, and template-variable details.
fn restricted_explain_response(explanation: &wayexpand_core::Explanation, json: bool) -> String {
    let (matched, would_expand, suppression_reason) = explain_summary(explanation);
    if json {
        format!(
            "{}\n",
            serde_json::json!({
                "matched": matched,
                "would_expand": would_expand,
                "suppression_reason": suppression_reason,
            })
        )
    } else {
        format!(
            "Snippet match: {}\nWould expand: {}\nReason: {}\n",
            if matched { "yes" } else { "no" },
            if would_expand { "yes" } else { "no" },
            suppression_reason,
        )
    }
}

/// Read one request line within `budget` for the whole request, not per
/// read: a client that trickles bytes would otherwise hold one of the few
/// control workers for as long as it likes. `None` means the request is
/// larger than [`MAX_COMMAND_BYTES`]; it is refused as soon as that is known
/// instead of reading on to the end of the line.
fn read_command(stream: &mut UnixStream, budget: Duration) -> std::io::Result<Option<String>> {
    let deadline = Instant::now() + budget;
    let mut command_bytes = Vec::with_capacity(MAX_COMMAND_BYTES);
    let mut chunk = [0_u8; 64];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "control request was not completed in time",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let count = match stream.read(&mut chunk) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            // The read timeout is the time left for the whole request.
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "control request was not completed in time",
                ))
            }
            Err(error) => return Err(error),
        };
        if count == 0 {
            break;
        }
        let line_end = chunk[..count].iter().position(|&byte| byte == b'\n');
        let line = &chunk[..line_end.map_or(count, |end| end + 1)];
        if command_bytes.len() + line.len() > MAX_COMMAND_BYTES {
            return Ok(None);
        }
        command_bytes.extend_from_slice(line);
        if line_end.is_some() {
            break;
        }
    }
    Ok(Some(String::from_utf8_lossy(&command_bytes).into_owned()))
}

fn handle_request(mut stream: UnixStream, flags: Flags) -> Result<()> {
    let Flags {
        reload,
        stop,
        pause,
        insert,
        require_verified_insert,
        disable_control_socket_mutations,
        focus,
        explain,
        usage_clear,
        status,
        waker,
    } = flags;
    stream.set_write_timeout(Some(CONTROL_IO_TIMEOUT))?;
    let peer_uid = peer_uid(&stream)?;
    if !peer_uid_is_authorized(peer_uid, rustix::process::geteuid().as_raw()) {
        stream.write_all(b"error=permission\n")?;
        return Ok(());
    }
    // An oversized request is answered like any unknown command.
    let command = match read_command(&mut stream, CONTROL_IO_TIMEOUT)? {
        Some(command) => command,
        None => {
            discard_unread_request(&mut stream);
            String::new()
        }
    };
    if disable_control_socket_mutations && is_mutating_request(&command) {
        stream.write_all(CONTROL_MUTATIONS_DISABLED_RESPONSE)?;
        return Ok(());
    }
    if let Some(request) = insert_request(&command) {
        let response = match request {
            Some(request) if require_verified_insert && request.focus_token.is_none() => {
                UNVERIFIED_INSERT_REFUSED
            }
            Some(request) => {
                *insert
                    .lock()
                    .map_err(|_| anyhow::anyhow!("insert lock poisoned"))? = Some(request);
                crate::waker::wake_slot(&waker);
                "insert scheduled\n"
            }
            None => "invalid trigger\n",
        };
        stream.write_all(response.as_bytes())?;
        return Ok(());
    }
    if let Some(response) = explain_request(&command, &explain)? {
        stream.write_all(response.as_bytes())?;
        return Ok(());
    }
    if let Some(path) = command.trim().strip_prefix("stats-clear ") {
        let handle = usage_clear
            .lock()
            .map_err(|_| anyhow::anyhow!("usage clear lock poisoned"))?
            .clone();
        let response = match handle.map(|handle| handle.clear(std::path::Path::new(path))) {
            Some(Ok(true)) => "local usage statistics cleared\n".to_string(),
            Some(Ok(false)) => "no local usage statistics to clear\n".to_string(),
            Some(Err(error)) => format!("could not clear local usage statistics: {error}\n"),
            None => "usage statistics writer unavailable\n".to_string(),
        };
        stream.write_all(response.as_bytes())?;
        return Ok(());
    }
    let response = match command.trim() {
        "status" => format!(
            "running\n{}\n",
            status
                .lock()
                .map_err(|_| anyhow::anyhow!("status lock poisoned"))?
        ),
        "focus" => {
            let snapshot = focus
                .lock()
                .map_err(|_| anyhow::anyhow!("focus lock poisoned"))?;
            // Focus identifiers are target-freshness data, not authorization
            // credentials. A read-only socket has no use for them, so withhold
            // them there; the generation is withheld too because it reveals
            // focus transitions. Verified inserts (the picker and
            // `insert --verified`) need them, so the verified-target policy
            // alone must not hide them or verified insertion is impossible.
            let restricted_focus = disable_control_socket_mutations;
            let generation = if restricted_focus {
                String::new()
            } else {
                snapshot.generation.to_string()
            };
            let token = if restricted_focus {
                ""
            } else {
                snapshot.token.as_deref().unwrap_or_default()
            };
            format!(
                "focus_generation={}\nfocus_token={}\nfocus_identity={}\n",
                generation,
                token,
                if snapshot.exact_window_identity {
                    "exact"
                } else {
                    "unavailable"
                }
            )
        }
        "reload" => {
            reload.store(true, Ordering::Release);
            crate::waker::wake_slot(&waker);
            "reload scheduled\n".to_string()
        }
        "stop" => {
            stop.store(true, Ordering::Release);
            crate::waker::wake_slot(&waker);
            "stopping\n".to_string()
        }
        "pause" => {
            pause.store(true, Ordering::Release);
            crate::waker::wake_slot(&waker);
            "paused\n".to_string()
        }
        "resume" => {
            pause.store(false, Ordering::Release);
            crate::waker::wake_slot(&waker);
            "resumed\n".to_string()
        }
        _ => "unknown command; expected status, focus, reload, pause, resume, stop, insert <trigger>, or explain <text>\n"
            .to_string(),
    };
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn is_mutating_request(command: &str) -> bool {
    let command = command.trim();
    matches!(
        command,
        "insert" | "insert-target" | "reload" | "stop" | "pause" | "resume" | "stats-clear"
    ) || command.starts_with("insert ")
        || command.starts_with("insert-target ")
        || command.starts_with("stats-clear ")
}

fn peer_uid_is_authorized(peer_uid: u32, daemon_uid: u32) -> bool {
    peer_uid == daemon_uid
}

#[cfg(target_os = "linux")]
fn validate_peer_credential_length(length: libc::socklen_t) -> std::io::Result<()> {
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unexpected control peer credentials size",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `credentials` points to a writable `ucred` of the exact size
    // requested by SO_PEERCRED. The descriptor is a connected Unix stream.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    validate_peer_credential_length(length)?;
    Ok(credentials.uid)
}

#[cfg(not(target_os = "linux"))]
fn peer_uid(_stream: &UnixStream) -> std::io::Result<u32> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "control peer credentials are unavailable on this platform",
    ))
}

pub fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WAYEXPAND_SOCKET") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::Ordering;

    #[test]
    fn simultaneous_explain_clients_receive_their_own_fifo_answers() {
        let (queue, requests) = std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY);
        let first_queue = queue.clone();
        let first = thread::spawn(move || {
            explain_request("explain-json .first", &first_queue)
                .unwrap()
                .unwrap()
        });

        let second_queue = queue.clone();
        let second = thread::spawn(move || {
            explain_request("explain-json .second", &second_queue)
                .unwrap()
                .unwrap()
        });

        for _ in 0..2 {
            let request = requests.recv_timeout(Duration::from_millis(500)).unwrap();
            request
                .reply
                .try_send(format!("answer for {}", request.text))
                .unwrap();
        }
        assert_eq!(first.join().unwrap(), "answer for .first");
        assert_eq!(second.join().unwrap(), "answer for .second");
    }

    #[test]
    fn saturated_explanation_queue_returns_retryable_response_without_overwriting() {
        let (queue, _requests) = std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY);
        let mut held_replies = Vec::new();
        for index in 0..EXPLAIN_QUEUE_CAPACITY {
            let (reply, response) = std::sync::mpsc::sync_channel(1);
            held_replies.push(response);
            queue
                .try_send(ExplainRequest {
                    text: format!(".pending{index}"),
                    json: true,
                    reply,
                })
                .unwrap();
        }

        let json = explain_request("explain-json .extra", &queue)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            serde_json::json!({"error": "busy", "retryable": true})
        );
        let text = explain_request("explain .extra", &queue).unwrap().unwrap();
        assert_eq!(text, "explain unavailable: daemon is busy; retry\n");
        drop(held_replies);
    }

    #[test]
    fn diagnostics_worker_answers_clipboard_snippets_without_rendering_them() {
        let config = wayexpand_core::Config::parse(
            "[settings]\nallow_clipboard = true\n\
             [[expansion]]\ntrigger = ':clip'\nreplacement = '{{clipboard}}'\n",
        )
        .unwrap();
        let validated = Arc::new(config.validate_snapshot().unwrap());
        let snapshot = Arc::new(Mutex::new(Some(ExplainSnapshot {
            validated,
            backend: "test",
            capture: "test-capture",
            output: "none",
            connection_detail: "test mode",
            output_connected: true,
            capabilities: None,
            paused: false,
            requested_pause: false,
            profile_switch_pending: false,
            sensitive: false,
            composition: false,
            form_open: false,
            window: None,
            form_guarantee: wayexpand_core::ReplacementGuarantee::Unsupported,
            commands_disabled: false,
            direct_commands_disabled: false,
            title_matching_disabled: false,
        })));
        for detailed in [true, false] {
            let (sender, receiver) = std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY);
            let worker_snapshot = Arc::clone(&snapshot);
            let worker = thread::spawn(move || {
                explain_worker(
                    receiver,
                    worker_snapshot,
                    detailed,
                    Arc::new(AtomicBool::new(false)),
                )
            });

            let started = Instant::now();
            let answer = explain_request("explain-json :clip", &sender)
                .unwrap()
                .unwrap();
            let elapsed = started.elapsed();
            let response: serde_json::Value = serde_json::from_str(&answer).unwrap();
            assert_eq!(response["matched"], true);
            assert_eq!(response["would_expand"], true);
            assert_eq!(response["suppression_reason"], "none");
            if detailed {
                assert_eq!(response["snippet"], ":clip");
                assert!(response["checks"].as_array().unwrap().iter().any(|check| {
                    check["name"] == "replacement"
                        && check["detail"] == "not rendered by control-socket diagnostics"
                }));
            } else {
                assert_eq!(
                    response,
                    serde_json::json!({
                        "matched": true,
                        "would_expand": true,
                        "suppression_reason": "none",
                    })
                );
            }
            assert!(elapsed < Duration::from_millis(500));

            drop(sender);
            worker.join().unwrap();
        }
    }

    #[test]
    fn restricted_diagnostics_redact_snippet_and_dynamic_value_metadata() {
        let explanation = wayexpand_core::Explanation {
            typed: "::payroll-secret".to_owned(),
            snippet: Some("::payroll-secret".to_owned()),
            checks: vec![
                wayexpand_core::ExplainCheck {
                    name: "command",
                    status: wayexpand_core::CheckStatus::Info,
                    detail: "runs /usr/local/libexec/private-helper".to_owned(),
                },
                wayexpand_core::ExplainCheck {
                    name: "variables",
                    status: wayexpand_core::CheckStatus::Info,
                    detail: "reads env:PAYROLL_TOKEN and clipboard".to_owned(),
                },
                wayexpand_core::ExplainCheck {
                    name: "replacement",
                    status: wayexpand_core::CheckStatus::Pass,
                    detail: "renders to 47 characters; Unicode U+202F at character 12".to_owned(),
                },
            ],
        };

        let response = restricted_explain_response(&explanation, true);
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "matched": true,
                "would_expand": true,
                "suppression_reason": "none",
            })
        );
        for private in [
            "::payroll-secret",
            "/usr/local/libexec/private-helper",
            "PAYROLL_TOKEN",
            "clipboard",
            "47",
            "U+202F",
            "character 12",
        ] {
            assert!(
                !response.contains(private),
                "leaked {private:?}: {response}"
            );
        }
        assert_eq!(
            restricted_explain_response(&explanation, false),
            "Snippet match: yes\nWould expand: yes\nReason: none\n"
        );
    }

    #[test]
    fn socket_diagnostic_flood_does_not_stall_keyboard_processing_with_slow_clipboard() {
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-explain-stress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.join("control.sock");
        let server = ControlServer::start_at(&socket, false, false).unwrap();
        let config = wayexpand_core::Config::parse(
            "[settings]\nallow_clipboard = true\n\
             [[expansion]]\ntrigger = ':clip'\nreplacement = '{{clipboard}}'\n",
        )
        .unwrap();
        server.set_explain_snapshot(ExplainSnapshot {
            validated: Arc::new(config.validate_snapshot().unwrap()),
            backend: "test",
            capture: "test-capture",
            output: "none",
            connection_detail: "test mode",
            output_connected: true,
            capabilities: None,
            paused: false,
            requested_pause: false,
            profile_switch_pending: false,
            sensitive: false,
            composition: false,
            form_open: false,
            window: None,
            form_guarantee: wayexpand_core::ReplacementGuarantee::Unsupported,
            commands_disabled: false,
            direct_commands_disabled: false,
            title_matching_disabled: false,
        });

        let stop_input = Arc::new(AtomicBool::new(false));
        let max_input_delay_ns = Arc::new(AtomicU64::new(0));
        let clipboard_reads = Arc::new(AtomicU64::new(0));
        let input_stop = Arc::clone(&stop_input);
        let max_delay = Arc::clone(&max_input_delay_ns);
        let reads = Arc::clone(&clipboard_reads);
        let input_thread = thread::spawn(move || {
            let mut engine = wayexpand_core::ExpansionEngine::new(
                wayexpand_core::Config::parse(
                    "[settings]\nallow_clipboard = true\n\
                     [[expansion]]\ntrigger = ':clip'\nreplacement = '{{clipboard}}'\n",
                )
                .unwrap(),
            )
            .unwrap();
            engine.set_clipboard_reader(Some(wayexpand_core::ClipboardReader(Arc::new(
                move || {
                    reads.fetch_add(1, Ordering::Relaxed);
                    thread::sleep(Duration::from_millis(250));
                    Some("slow clipboard".to_owned())
                },
            ))));
            let mut pending = Vec::new();
            while !input_stop.load(Ordering::Acquire) {
                let started = Instant::now();
                engine.process_deferred_char_into('x', &mut pending);
                max_delay.fetch_max(
                    started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                    Ordering::Relaxed,
                );
            }
        });

        let clients = (0..CONTROL_WORKERS)
            .map(|_| {
                let socket = socket.clone();
                thread::spawn(move || {
                    for _ in 0..12 {
                        let mut stream = UnixStream::connect(&socket).unwrap();
                        stream.write_all(b"explain-json :clip\n").unwrap();
                        let mut response = String::new();
                        stream.read_to_string(&mut response).unwrap();
                        assert!(!response.is_empty());
                    }
                })
            })
            .collect::<Vec<_>>();
        for client in clients {
            client.join().unwrap();
        }
        stop_input.store(true, Ordering::Release);
        input_thread.join().unwrap();

        assert_eq!(clipboard_reads.load(Ordering::Relaxed), 0);
        assert!(
            max_input_delay_ns.load(Ordering::Relaxed) < 100_000_000,
            "maximum simulated input processing gap was {} ms",
            max_input_delay_ns.load(Ordering::Relaxed) / 1_000_000
        );
        drop(server);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn focus_read_does_not_authorize_insert_target_in_restricted_mode() {
        let insert = Arc::new(Mutex::new(None));
        let focus = Arc::new(Mutex::new(FocusSnapshot {
            generation: 42,
            token: Some("deadbeef".into()),
            exact_window_identity: true,
        }));
        let flags = Flags {
            reload: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            insert: Arc::clone(&insert),
            require_verified_insert: true,
            disable_control_socket_mutations: true,
            focus,
            explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
            usage_clear: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new("running".into())),
            waker: Arc::default(),
        };
        let send = |command: &str| {
            let (mut client, server) = UnixStream::pair().unwrap();
            client.write_all(command.as_bytes()).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            handle_request(server, flags.clone()).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            response
        };

        assert_eq!(
            send("focus\n"),
            "focus_generation=\nfocus_token=\nfocus_identity=exact\n"
        );
        for request in [
            "insert-target 42 deadbeef ;sig\n",
            "insert ;sig\n",
            "reload\n",
            "pause\n",
            "resume\n",
            "stop\n",
            "stats-clear /tmp/usage.json\n",
        ] {
            assert_eq!(
                send(request).as_bytes(),
                CONTROL_MUTATIONS_DISABLED_RESPONSE,
                "request {request:?}"
            );
        }
        assert!(insert.lock().unwrap().is_none());
        assert!(!flags.stop.load(Ordering::Acquire));
        assert!(!flags.reload.load(Ordering::Acquire));
        assert!(!flags.pause.load(Ordering::Acquire));
        assert_eq!(send("status\n"), "running\nrunning\n");
    }

    #[test]
    fn verified_target_policy_keeps_focus_identifiers_for_verified_inserts() {
        let insert = Arc::new(Mutex::new(None));
        let flags = Flags {
            reload: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            insert: Arc::clone(&insert),
            require_verified_insert: true,
            disable_control_socket_mutations: false,
            focus: Arc::new(Mutex::new(FocusSnapshot {
                generation: 9,
                token: Some("deadbeef".into()),
                exact_window_identity: true,
            })),
            explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
            usage_clear: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new("running".into())),
            waker: Arc::default(),
        };
        let send = |command: &str| {
            let (mut client, server) = UnixStream::pair().unwrap();
            client.write_all(command.as_bytes()).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            handle_request(server, flags.clone()).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            response
        };

        // The picker and `insert --verified` read the token, then insert.
        assert_eq!(
            send("focus\n"),
            "focus_generation=9\nfocus_token=deadbeef\nfocus_identity=exact\n"
        );
        assert_eq!(
            send("insert-target 9 deadbeef ;sig\n"),
            "insert scheduled\n"
        );
        assert!(insert.lock().unwrap().is_some());
        assert_eq!(send("insert ;sig\n"), UNVERIFIED_INSERT_REFUSED);
    }

    #[test]
    fn control_mutation_classifier_covers_all_state_changing_commands() {
        for request in [
            "insert ;sig",
            "insert-target 42 deadbeef ;sig",
            "reload",
            "stop",
            "pause",
            "resume",
            "stats-clear /tmp/usage.json",
        ] {
            assert!(is_mutating_request(request), "{request}");
        }
        for request in ["status", "focus", "explain ;sig", "explain-json ;sig"] {
            assert!(!is_mutating_request(request), "{request}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn control_peer_credentials_match_the_daemon_uid() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        assert_eq!(
            peer_uid(&stream).unwrap(),
            rustix::process::geteuid().as_raw()
        );
        let uid = rustix::process::geteuid().as_raw();
        assert!(peer_uid_is_authorized(uid, uid));
        assert!(!peer_uid_is_authorized(uid.saturating_add(1), uid));
        let expected = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        assert!(validate_peer_credential_length(expected).is_ok());
        let error = validate_peer_credential_length(expected.saturating_sub(1)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn shutdown_completes_when_the_socket_path_was_removed() {
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-control-removed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("wayexpand.sock");
        let server = ControlServer::start_at(&path, false, false).unwrap();
        // For example, a session cleanup removed the runtime directory.
        fs::remove_file(&path).unwrap();

        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(server);
            let _ = done.send(());
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(5)).is_ok(),
            "control server shutdown hung after its socket path was removed"
        );
        let _ = fs::remove_dir_all(directory);
    }

    fn request(
        command: &str,
        reload: &Arc<AtomicBool>,
        stop: &Arc<AtomicBool>,
        pause: &Arc<AtomicBool>,
        status: &Arc<Mutex<String>>,
    ) -> String {
        let (mut client, server) = UnixStream::pair().unwrap();
        let reload_worker = Arc::clone(reload);
        let stop_worker = Arc::clone(stop);
        let pause_worker = Arc::clone(pause);
        let status_worker = Arc::clone(status);
        let join = thread::spawn(move || {
            let insert = Arc::new(Mutex::new(None));
            let focus = Arc::new(Mutex::new(FocusSnapshot::default()));
            handle_request(
                server,
                Flags {
                    reload: reload_worker,
                    stop: stop_worker,
                    pause: pause_worker,
                    insert,
                    require_verified_insert: false,
                    disable_control_socket_mutations: false,
                    focus,
                    explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
                    usage_clear: Arc::new(Mutex::new(None)),
                    status: status_worker,
                    waker: Arc::default(),
                },
            )
            .unwrap();
        });
        client.write_all(command.as_bytes()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        join.join().unwrap();
        response
    }

    #[test]
    fn lifecycle_commands_set_flags_and_reply() {
        let reload = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new("source=stdin\nbackend=none".into()));
        assert_eq!(
            request("status\n", &reload, &stop, &pause, &status),
            "running\nsource=stdin\nbackend=none\n"
        );
        assert_eq!(
            request("reload\n", &reload, &stop, &pause, &status),
            "reload scheduled\n"
        );
        assert!(reload.load(Ordering::Acquire));
        assert_eq!(
            request("pause\n", &reload, &stop, &pause, &status),
            "paused\n"
        );
        assert!(pause.load(Ordering::Acquire));
        assert_eq!(
            request("resume\n", &reload, &stop, &pause, &status),
            "resumed\n"
        );
        assert!(!pause.load(Ordering::Acquire));
        assert_eq!(
            request("stop\n", &reload, &stop, &pause, &status),
            "stopping\n"
        );
        assert!(stop.load(Ordering::Acquire));
    }

    #[test]
    fn verified_insert_policy_refuses_requests_without_a_focus_target() {
        let insert = Arc::new(Mutex::new(None));
        let flags = Flags {
            reload: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            insert: Arc::clone(&insert),
            require_verified_insert: true,
            disable_control_socket_mutations: false,
            focus: Arc::new(Mutex::new(FocusSnapshot::default())),
            explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
            usage_clear: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(String::new())),
            waker: Arc::default(),
        };
        let send = |command: &str| {
            let (mut client, server) = UnixStream::pair().unwrap();
            client.write_all(command.as_bytes()).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            handle_request(server, flags.clone()).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            response
        };

        assert_eq!(send("insert ;sig\n"), UNVERIFIED_INSERT_REFUSED);
        assert!(insert.lock().unwrap().is_none(), "nothing may be queued");

        assert_eq!(send("insert-target 4 abcd ;sig\n"), "insert scheduled\n");
        let queued = insert.lock().unwrap().clone().unwrap();
        assert_eq!(queued.focus_token.as_deref(), Some("abcd"));
        assert_eq!(queued.focus_generation, Some(4));
    }

    #[test]
    fn insert_takes_the_trigger_verbatim_and_refuses_control_characters() {
        let reload = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let insert = Arc::new(Mutex::new(None));
        let focus = Arc::new(Mutex::new(FocusSnapshot::default()));
        let status = Arc::new(Mutex::new(String::new()));
        let flags = Flags {
            reload,
            stop,
            pause,
            insert: Arc::clone(&insert),
            require_verified_insert: false,
            disable_control_socket_mutations: false,
            focus: Arc::clone(&focus),
            explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
            usage_clear: Arc::new(Mutex::new(None)),
            status,
            waker: Arc::default(),
        };
        let send = |command: &str| {
            let (mut client, server) = UnixStream::pair().unwrap();
            client.write_all(command.as_bytes()).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            handle_request(server, flags.clone()).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            response
        };
        for (line, trigger) in [
            ("insert ;sig\n", ";sig"),
            ("insert :sig \r\n", ":sig "),
            ("insert  lead\n", " lead"),
            ("insert 🙂x\n", "🙂x"),
        ] {
            assert_eq!(send(line), "insert scheduled\n", "{line:?}");
            assert_eq!(
                insert
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|request| request.trigger.as_str()),
                Some(trigger)
            );
        }
        *focus.lock().unwrap() = FocusSnapshot {
            generation: 4,
            token: Some("abcd".into()),
            exact_window_identity: true,
        };
        assert_eq!(
            send("focus\n"),
            "focus_generation=4\nfocus_token=abcd\nfocus_identity=exact\n"
        );
        assert_eq!(send("insert-target 4 abcd ;target\n"), "insert scheduled\n");
        assert_eq!(
            insert
                .lock()
                .unwrap()
                .as_ref()
                .map(|request| request.focus_token.as_deref()),
            Some(Some("abcd"))
        );
        let too_long = format!("insert {}\n", "x".repeat(129));
        for bad in ["insert\n", "insert \n", "insert a\tb\n", too_long.as_str()] {
            *insert.lock().unwrap() = None;
            assert_eq!(send(bad), "invalid trigger\n", "{bad:?}");
            assert!(insert.lock().unwrap().is_none());
        }
    }

    #[test]
    fn maximum_exact_focus_insert_fits_control_frame() {
        let window = crate::WindowContext {
            app_id: None,
            title: None,
            instance_id: Some("x".repeat(192)),
        };
        let token = crate::focus::focus_token(&window).unwrap();
        let trigger = "🙂".repeat(128);
        let command = format!("insert-target {} {} {}\n", u64::MAX, token, trigger);

        assert!(command.len() <= MAX_COMMAND_BYTES);
        assert!(insert_request(&command).unwrap().is_some());
    }

    #[test]
    fn only_an_unusable_socket_stops_the_listener() {
        let error =
            |errno: rustix::io::Errno| std::io::Error::from_raw_os_error(errno.raw_os_error());
        assert_eq!(
            accept_failure(&error(rustix::io::Errno::CONNABORTED)),
            AcceptFailure::Retry
        );
        assert_eq!(
            accept_failure(&error(rustix::io::Errno::INTR)),
            AcceptFailure::Retry
        );
        assert_eq!(
            accept_failure(&error(rustix::io::Errno::MFILE)),
            AcceptFailure::Backoff
        );
        assert_eq!(
            accept_failure(&error(rustix::io::Errno::NFILE)),
            AcceptFailure::Backoff
        );
        assert_eq!(
            accept_failure(&error(rustix::io::Errno::INVAL)),
            AcceptFailure::Fatal
        );
        assert_eq!(
            accept_failure(&error(rustix::io::Errno::BADF)),
            AcceptFailure::Fatal
        );
    }

    #[test]
    fn unknown_command_is_nonfatal() {
        let reload = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new("starting".into()));
        assert_eq!(
            request("bogus\n", &reload, &stop, &pause, &status),
            "unknown command; expected status, focus, reload, pause, resume, stop, insert <trigger>, or explain <text>\n"
        );
        assert!(!reload.load(Ordering::Acquire));
        assert!(!stop.load(Ordering::Acquire));
    }

    #[test]
    fn oversized_command_is_bounded_and_nonfatal() {
        let reload = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new("starting".into()));
        let command = format!("{}\n", "x".repeat(MAX_COMMAND_BYTES + 1024));
        assert_eq!(
            request(&command, &reload, &stop, &pause, &status),
        "unknown command; expected status, focus, reload, pause, resume, stop, insert <trigger>, or explain <text>\n"
        );
        assert!(!reload.load(Ordering::Acquire));
        assert!(!stop.load(Ordering::Acquire));
    }

    #[test]
    fn a_request_split_across_writes_is_read_whole() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            client.write_all(b"sta").unwrap();
            std::thread::sleep(Duration::from_millis(30));
            client.write_all(b"tus\n").unwrap();
            client
        });
        let command = read_command(&mut server, Duration::from_secs(5)).unwrap();
        assert_eq!(command.as_deref(), Some("status\n"));
        drop(writer.join().unwrap());
    }

    #[test]
    fn an_oversized_request_is_refused_without_waiting_for_its_end() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        // No newline, and the client stays connected.
        client.write_all(&[b'x'; MAX_COMMAND_BYTES + 1]).unwrap();
        let started = Instant::now();
        assert_eq!(
            read_command(&mut server, Duration::from_secs(5)).unwrap(),
            None
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(client);
    }

    #[test]
    fn a_trickling_client_cannot_outlast_the_request_deadline() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let trickle_stop = Arc::clone(&stop);
        // One byte every 20 ms never trips a per-read timeout.
        let trickler = std::thread::spawn(move || {
            while !trickle_stop.load(Ordering::Acquire) {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let started = Instant::now();
        let error = read_command(&mut server, Duration::from_millis(300)).unwrap_err();
        let elapsed = started.elapsed();
        stop.store(true, Ordering::Release);
        drop(server);
        trickler.join().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            elapsed < Duration::from_secs(1),
            "the request held its worker for {elapsed:?}"
        );
    }

    #[test]
    fn overloaded_control_request_gets_machine_readable_busy_response() {
        let rejected = AtomicU64::new(0);
        let (mut client, server) = UnixStream::pair().unwrap();
        // An unread, oversized request must not turn the close into a
        // connection reset.
        client
            .write_all(&vec![b'x'; MAX_COMMAND_BYTES * 4])
            .unwrap();
        reject_busy(server, &rejected);
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert_eq!(response, "error=busy\nretryable=true\n");
        assert_eq!(rejected.load(Ordering::Acquire), 1);
    }

    #[test]
    fn busy_response_keeps_socket_open_for_request_bytes_sent_after_connect() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let server_thread = thread::spawn(move || {
            let rejected = AtomicU64::new(0);
            reject_busy(server, &rejected);
            rejected.load(Ordering::Acquire)
        });

        // Read the response before sending the request to force the accept-
        // then-write ordering that used to close the socket with a broken
        // pipe. The server's write half is shut down, but its read half must
        // stay alive long enough to consume this bounded request.
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert_eq!(response, "error=busy\nretryable=true\n");
        client.write_all(b"explain-json :clip\n").unwrap();
        assert_eq!(server_thread.join().unwrap(), 1);
    }

    #[test]
    fn stale_socket_policy_requires_socket_and_matching_owner() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-control-test-{}", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        let uid = rustix::process::geteuid().as_raw();
        assert!(is_owned_socket(&metadata, uid));
        let identity = (metadata.dev(), metadata.ino());
        assert!(is_original_socket(&metadata, identity, uid));
        assert!(!is_original_socket(&metadata, (0, 0), uid));
        assert!(!is_owned_socket(&metadata, uid.saturating_add(1)));
        drop(listener);
        fs::remove_file(path).unwrap();

        let regular =
            std::env::temp_dir().join(format!("wayexpand-control-regular-{}", std::process::id()));
        fs::write(&regular, b"not a socket").unwrap();
        let metadata = fs::metadata(&regular).unwrap();
        assert!(!is_owned_socket(&metadata, uid));
        fs::remove_file(regular).unwrap();
    }

    #[test]
    fn only_refused_or_missing_connections_are_stale_socket_errors() {
        assert!(is_stale_socket_error(&std::io::Error::from(
            std::io::ErrorKind::ConnectionRefused,
        )));
        assert!(is_stale_socket_error(&std::io::Error::from(
            std::io::ErrorKind::NotFound,
        )));
        assert!(!is_stale_socket_error(&std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        )));
        assert!(!is_stale_socket_error(&std::io::Error::from(
            std::io::ErrorKind::Other,
        )));
    }

    #[test]
    fn socket_parent_must_be_private_and_user_owned() {
        let parent =
            std::env::temp_dir().join(format!("wayexpand-control-parent-{}", std::process::id()));
        fs::create_dir(&parent).unwrap();
        // Explicit mode rather than the ambient umask: a default of 002
        // (Debian/Ubuntu user-private-group setups) creates this 0775, which
        // this check correctly rejects as group-writable.
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = parent.join("wayexpand.sock");
        assert!(validate_socket_parent(&socket).is_ok());

        fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_socket_parent(&socket).is_err());
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn root_owned_world_writable_mode_is_rejected() {
        // Keep this regression test independent of test-runner privileges:
        // the validation rule is about mode bits, regardless of uid.
        assert!(!socket_parent_mode_is_secure(0o0777));
        assert!(socket_parent_mode_is_secure(0o1777));
        assert!(socket_parent_mode_is_secure(0o0755));
    }

    #[test]
    fn socket_ancestors_must_be_trusted() {
        if rustix::process::geteuid().as_raw() != 0 {
            return;
        }
        let root =
            std::env::temp_dir().join(format!("wayexpand-control-ancestor-{}", std::process::id()));
        let untrusted = root.join("untrusted");
        let inner = untrusted.join("inner");
        fs::create_dir_all(&inner).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&untrusted, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o700)).unwrap();
        rustix::fs::chown(&untrusted, Some(rustix::fs::Uid::from_raw(65_534)), None).unwrap();

        assert!(validate_socket_parent(&inner.join("wayexpand.sock")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn socket_parent_symlink_is_resolved_before_binding() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-control-symlink-{}", std::process::id()));
        let target = root.join("target");
        let link = root.join("link");
        fs::create_dir_all(&target).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert_eq!(
            secure_socket_path(&link.join("wayexpand.sock")).unwrap(),
            target.join("wayexpand.sock")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn state_changing_requests_wake_the_reactor() {
        let waker = crate::waker::Waker::new().unwrap();
        let slot: crate::waker::WakerSlot = Arc::default();
        slot.set(waker.clone()).ok().unwrap();
        let flags = Flags {
            reload: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            insert: Arc::new(Mutex::new(None)),
            require_verified_insert: false,
            disable_control_socket_mutations: false,
            focus: Arc::new(Mutex::new(FocusSnapshot::default())),
            explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
            usage_clear: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(String::new())),
            waker: slot,
        };
        let fd = waker.fd();
        let woken = || {
            let mut fds = [rustix::event::PollFd::new(
                &*fd,
                rustix::event::PollFlags::IN,
            )];
            let zero = rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            let ready = rustix::event::poll(&mut fds, Some(&zero)).unwrap() > 0;
            if ready {
                let mut buffer = [0_u8; 8];
                rustix::io::read(&*fd, &mut buffer).unwrap();
            }
            ready
        };
        for (command, wakes) in [
            ("status\n", false),
            ("focus\n", false),
            ("pause\n", true),
            ("resume\n", true),
            ("reload\n", true),
            ("insert :x\n", true),
            ("stop\n", true),
        ] {
            let (mut client, server) = UnixStream::pair().unwrap();
            client.write_all(command.as_bytes()).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            handle_request(server, flags.clone()).unwrap();
            assert_eq!(woken(), wakes, "{command:?}");
        }
    }
}
