use super::diagnostics::explain_request;
use super::*;

/// The shared state one control request may touch.
#[derive(Clone)]
pub(super) struct Flags {
    pub(super) reload: Arc<AtomicBool>,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) pause: Arc<AtomicBool>,
    pub(super) insert: Arc<Mutex<Option<InsertRequest>>>,
    pub(super) require_verified_insert: bool,
    pub(super) disable_control_socket_mutations: bool,
    pub(super) focus: Arc<Mutex<FocusSnapshot>>,
    pub(super) explain: std::sync::mpsc::SyncSender<ExplainRequest>,
    pub(super) usage_clear: Arc<Mutex<Option<crate::usage::UsageClearHandle>>>,
    pub(super) status: Arc<Mutex<String>>,
    pub(super) waker: crate::waker::WakerSlot,
}

pub(super) fn reject_busy(mut stream: UnixStream, rejected_total: &AtomicU64) {
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
pub(super) const UNVERIFIED_INSERT_REFUSED: &str =
    "insert refused: organization policy requires a verified insert target (use insert --verified)\n";

/// The trigger of an `insert <trigger>` request, taken verbatim: a
/// configured trigger may legitimately begin or end with a space. Control
/// characters are refused so the line protocol cannot be confused.
/// Returns `None` for any other command, `Some(None)` for an invalid trigger.
pub(super) fn insert_request(line: &str) -> Option<Option<InsertRequest>> {
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
/// Read one request line within `budget` for the whole request, not per
/// read: a client that trickles bytes would otherwise hold one of the few
/// control workers for as long as it likes. `None` means the request is
/// larger than [`MAX_COMMAND_BYTES`]; it is refused as soon as that is known
/// instead of reading on to the end of the line.
pub(super) fn read_command(
    stream: &mut UnixStream,
    budget: Duration,
) -> std::io::Result<Option<String>> {
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

pub(super) fn handle_request(mut stream: UnixStream, flags: Flags) -> Result<()> {
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
            let status = status
                .lock()
                .map_err(|_| anyhow::anyhow!("status lock poisoned"))?;
            let atomic_replace = status
                .lines()
                .any(|line| line == "inject_atomic_replace=true");
            let verified_replace = status
                .lines()
                .any(|line| line == "replacement_assurance=verified");
            format!(
                "focus_generation={}\nfocus_token={}\nfocus_identity={}\natomic_replace={}\nverified_replace={}\n",
                generation,
                token,
                if snapshot.exact_window_identity {
                    "exact"
                } else {
                    "unavailable"
                },
                atomic_replace,
                verified_replace
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

pub(super) fn is_mutating_request(command: &str) -> bool {
    let command = command.trim();
    matches!(
        command,
        "insert" | "insert-target" | "reload" | "stop" | "pause" | "resume" | "stats-clear"
    ) || command.starts_with("insert ")
        || command.starts_with("insert-target ")
        || command.starts_with("stats-clear ")
}

pub(super) fn peer_uid_is_authorized(peer_uid: u32, daemon_uid: u32) -> bool {
    peer_uid == daemon_uid
}

#[cfg(target_os = "linux")]
pub(super) fn validate_peer_credential_length(length: libc::socklen_t) -> std::io::Result<()> {
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unexpected control peer credentials size",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
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
pub(super) fn peer_uid(_stream: &UnixStream) -> std::io::Result<u32> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "control peer credentials are unavailable on this platform",
    ))
}
