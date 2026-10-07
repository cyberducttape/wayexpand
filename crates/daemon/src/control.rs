use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tracing::warn;

use crate::socket_security::{
    is_original_socket, is_owned_socket, secure_socket_path, validate_socket_parent,
};
#[cfg(test)]
use crate::socket_security::socket_parent_mode_is_secure;
#[cfg(target_os = "linux")]
use crate::socket_security::open_socket_parent;

/// Large enough for `insert ` plus a maximum-length (128 character) trigger.
const MAX_COMMAND_BYTES: usize = 1024;
const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(2);
const CONTROL_WORKERS: usize = 4;

pub struct ControlServer {
    pub reload_requested: Arc<AtomicBool>,
    pub stop_requested: Arc<AtomicBool>,
    pub pause_requested: Arc<AtomicBool>,
    /// A snippet trigger to insert at the cursor (quick-insert picker,
    /// `wayexpand insert`). Only the latest request is kept: an insert is a one-shot
    /// user action and a stale queued one must never fire later.
    insert_requested: Arc<Mutex<Option<InsertRequest>>>,
    focus_snapshot: Arc<Mutex<FocusSnapshot>>,
    /// A pending `explain` request, answered by the reactor from its live
    /// engine state.
    explain_requested: Arc<Mutex<Option<ExplainRequest>>>,
    status: Arc<Mutex<String>>,
    /// Wakes the reactor after a request that changes daemon state.
    waker: crate::waker::WakerSlot,
    path: Option<PathBuf>,
    socket_identity: Option<(u64, u64)>,
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

/// How long a control client waits for the reactor to answer an explain
/// request before being told the daemon is busy.
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
            insert_requested: Arc::new(Mutex::new(None)),
            focus_snapshot: Arc::new(Mutex::new(FocusSnapshot::default())),
            explain_requested: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new("starting".into())),
            waker: Arc::default(),
            path: None,
            socket_identity: None,
        }
    }

    pub fn start() -> Result<Self> {
        let Some(requested_path) = socket_path() else {
            return Ok(Self::detached());
        };
        let path = secure_socket_path(&requested_path)?;
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
                Err(_) => {
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
            }
        }
        // A restrictive umask closes the permission window between bind and
        // chmod. Restore the caller's mask immediately after bind so this
        // process does not change unrelated file-creation behavior.
        let previous_umask = rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
        let listener_result = UnixListener::bind(&operation_path);
        rustix::process::umask(previous_umask);
        let listener = listener_result
            .with_context(|| format!("binding control socket {}", path.display()))?;
        fs::set_permissions(&operation_path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&operation_path)?;
        let socket_identity = Some((metadata.dev(), metadata.ino()));
        let reload_requested = Arc::new(AtomicBool::new(false));
        let stop_requested = Arc::new(AtomicBool::new(false));
        let pause_requested = Arc::new(AtomicBool::new(false));
        let insert_requested = Arc::new(Mutex::new(None));
        let focus_snapshot = Arc::new(Mutex::new(FocusSnapshot::default()));
        let explain_requested = Arc::new(Mutex::new(None));
        let explain_slot = Arc::clone(&explain_requested);
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
        thread::Builder::new()
            .name("wayexpand-control-listener".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    let flags = Flags {
                        reload: Arc::clone(&reload_flag),
                        stop: Arc::clone(&stop_flag),
                        pause: Arc::clone(&pause_flag),
                        insert: Arc::clone(&insert_slot),
                        focus: Arc::clone(&focus_slot),
                        explain: Arc::clone(&explain_slot),
                        status: Arc::clone(&status_flag),
                        waker: Arc::clone(&waker_slot),
                    };
                    let admitted = active_requests_for_listener.fetch_update(
                        Ordering::AcqRel,
                        Ordering::Acquire,
                        |active| (active < CONTROL_WORKERS).then_some(active + 1),
                    );
                    if admitted.is_err() {
                        // Keep control-plane concurrency bounded. A busy or
                        // malicious same-user client can be dropped without
                        // delaying the accept loop or keyboard data plane.
                        continue;
                    }
                    let active_requests = Arc::clone(&active_requests_for_listener);
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
                        warn!(%error, "could not start control request worker; dropping request");
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
            insert_requested,
            focus_snapshot,
            explain_requested,
            status,
            waker,
            path: Some(path),
            socket_identity,
        })
    }

    /// Take the pending insert request, if any.
    /// Take a pending explain request; the reactor answers it on `reply`.
    pub fn take_explain_request(&self) -> Option<ExplainRequest> {
        self.explain_requested
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

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

    /// Publishes a status body. Taking `StatusBody` rather than any string
    /// keeps `crate::status`'s builder the single producer of the documented
    /// control-socket field set.
    /// Install the reactor's waker; requests that change daemon state then
    /// wake the loop immediately.
    pub fn set_waker(&self, waker: crate::waker::Waker) {
        let _ = self.waker.set(waker);
    }

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

/// The shared state one control request may touch.
#[derive(Clone)]
struct Flags {
    reload: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    insert: Arc<Mutex<Option<InsertRequest>>>,
    focus: Arc<Mutex<FocusSnapshot>>,
    explain: Arc<Mutex<Option<ExplainRequest>>>,
    status: Arc<Mutex<String>>,
    waker: crate::waker::WakerSlot,
}

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
    slot: &Mutex<Option<ExplainRequest>>,
    waker: &crate::waker::WakerSlot,
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
    *slot
        .lock()
        .map_err(|_| anyhow::anyhow!("explain lock poisoned"))? = Some(ExplainRequest {
        text: text.to_owned(),
        json,
        reply,
    });
    crate::waker::wake_slot(waker);
    Ok(Some(
        answer
            .recv_timeout(EXPLAIN_REPLY_TIMEOUT)
            .unwrap_or_else(|_| "explain unavailable: the daemon did not answer in time\n".into()),
    ))
}

fn handle_request(mut stream: UnixStream, flags: Flags) -> Result<()> {
    let Flags {
        reload,
        stop,
        pause,
        insert,
        focus,
        explain,
        status,
        waker,
    } = flags;
    stream.set_read_timeout(Some(CONTROL_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_IO_TIMEOUT))?;
    let mut command_bytes = Vec::with_capacity(MAX_COMMAND_BYTES);
    let mut chunk = [0_u8; 64];
    let mut oversized = false;
    loop {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let remaining = MAX_COMMAND_BYTES.saturating_sub(command_bytes.len());
        let retained = count.min(remaining);
        command_bytes.extend_from_slice(&chunk[..retained]);
        oversized |= retained != count;
        if chunk[..count].contains(&b'\n') {
            break;
        }
    }
    let command = if oversized {
        String::new()
    } else {
        String::from_utf8_lossy(&command_bytes).into_owned()
    };
    if let Some(request) = insert_request(&command) {
        let response = match request {
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
    if let Some(response) = explain_request(&command, &explain, &waker)? {
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
            format!(
                "focus_generation={}\nfocus_token={}\nfocus_identity={}\n",
                snapshot.generation,
                snapshot.token.as_deref().unwrap_or_default(),
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
                    focus,
                    explain: Arc::new(Mutex::new(None)),
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
            focus: Arc::clone(&focus),
            explain: Arc::new(Mutex::new(None)),
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
            focus: Arc::new(Mutex::new(FocusSnapshot::default())),
            explain: Arc::new(Mutex::new(None)),
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
