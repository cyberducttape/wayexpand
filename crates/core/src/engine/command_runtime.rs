//! Asynchronous command execution and hotkey action handling.
//!
//! Organizes worker threads for executing expansion commands and hotkey actions
//! with bounded queueing and timeout enforcement.

use std::{
    io::Read,
    process::{Child, ChildStderr, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
pub(super) use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

use super::{
    BrokerOperation, BrokerProtocolFailure, BrokerUnavailableReason, CommandConfig,
    CommandEnvironment, CommandError, ProcessWaitFailure, ProcessWaitOperation,
    MAX_COMMAND_OUTPUT_BYTES, MINIMAL_COMMAND_PATH,
};

const MAX_COMMAND_STDERR_BYTES: usize = 16 * 1024;

/// Atomic counters shared by the expansion and hotkey workers. Keeping this
/// state with the runtime module makes queue accounting independent of the
/// engine's matching and transaction state.
pub(super) struct CommandMetricsState {
    pub(super) queue_depth: AtomicUsize,
    pub(super) in_flight: AtomicUsize,
    pub(super) queue_rejected_total: AtomicU64,
    pub(super) timeout_total: AtomicU64,
    pub(super) failure_total: AtomicU64,
}

impl CommandMetricsState {
    pub(super) fn new() -> Self {
        Self {
            queue_depth: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            queue_rejected_total: AtomicU64::new(0),
            timeout_total: AtomicU64::new(0),
            failure_total: AtomicU64::new(0),
        }
    }

    pub(super) fn record_error(&self, timeout: bool) {
        if timeout {
            self.timeout_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failure_total.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(super) enum QueueSendError {
    Full,
    Disconnected,
}

/// Execute one configured command with bounded output, timeout enforcement,
/// and process-group cleanup. The engine owns policy decisions; this module
/// owns the process lifecycle and I/O mechanics.
pub fn run_command(command: &CommandConfig) -> Result<String, CommandError> {
    run_command_with_shutdown(command, None)
}

/// Execute a command that may be cancelled by its owner. Cancellation is
/// checked before spawning and while waiting; on Unix the whole process group
/// is killed before this function returns.
pub fn run_command_cancellable(
    command: &CommandConfig,
    cancelled: &AtomicBool,
) -> Result<String, CommandError> {
    run_command_with_shutdown(command, Some(cancelled))
}

pub(super) fn run_command_with_shutdown(
    command: &CommandConfig,
    shutdown: Option<&AtomicBool>,
) -> Result<String, CommandError> {
    run_command_with_parameters(command, &std::collections::HashMap::new(), shutdown)
}

pub(super) fn run_command_with_parameters(
    command: &CommandConfig,
    parameters: &std::collections::HashMap<String, String>,
    shutdown: Option<&AtomicBool>,
) -> Result<String, CommandError> {
    if let Some(action_id) = &command.action {
        return run_broker_action(action_id, command, parameters, shutdown);
    }
    if !parameters.is_empty() {
        return Err(CommandError::PolicyBlocked);
    }
    #[cfg(unix)]
    {
        if shutdown.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(CommandError::StaleInput);
        }
        let mut process = Command::new(&command.program);
        configure_command_environment(&mut process, command);
        process
            .args(&command.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process_group(&mut process);
        let mut child = process.spawn().map_err(|error| CommandError::SpawnFailed {
            detail: error.to_string(),
        })?;
        let stdout = child.stdout.take().ok_or(CommandError::OutputChannelLost)?;
        let stderr = child.stderr.take().ok_or(CommandError::OutputChannelLost)?;
        run_command_unix(child, stdout, stderr, command.timeout_ms, shutdown)
    }

    #[cfg(not(unix))]
    {
        let _ = (command, shutdown);
        unreachable!("wayexpand-core requires a Unix target")
    }
}

fn run_broker_action(
    action_id: &str,
    command: &CommandConfig,
    parameters: &std::collections::HashMap<String, String>,
    shutdown: Option<&AtomicBool>,
) -> Result<String, CommandError> {
    let deadline = Instant::now() + Duration::from_millis(command.timeout_ms);
    if shutdown.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err(CommandError::StaleInput);
    }
    let socket = std::env::var_os("WAYEXPAND_ACTION_BROKER_SOCKET")
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|dir| {
                std::path::PathBuf::from(dir)
                    .join("wayexpand-broker.sock")
                    .into_os_string()
            })
        })
        .ok_or(CommandError::BrokerUnavailable {
            reason: BrokerUnavailableReason::SocketNotConfigured,
        })?;
    let mut client = wayexpand_broker_client::BrokerClient::connect(socket).map_err(|error| {
        CommandError::BrokerUnavailable {
            reason: BrokerUnavailableReason::Connect {
                detail: error.to_string(),
            },
        }
    })?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(CommandError::Timeout);
    }
    client
        .set_io_timeout(remaining)
        .map_err(|error| broker_command_error(BrokerOperation::SetIoTimeout, error))?;
    let env_vars = command
        .pass_env
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| format!("{name}={value}"))
        })
        .collect();
    client
        .send_request(&wayexpand_broker_client::ActionRequest {
            action_id: action_id.to_owned(),
            timeout_ms: command.timeout_ms,
            inherit_env: false,
            env_vars,
            stdout_capture: true,
            parameters: parameters
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        })
        .map_err(|error| broker_command_error(BrokerOperation::SendRequest, error))?;
    match client
        .recv_response_until(deadline, shutdown)
        .map_err(|error| broker_command_error(BrokerOperation::ReceiveResponse, error))?
    {
        wayexpand_broker_client::ActionResponse::Success(output) => broker_output_text(output),
        wayexpand_broker_client::ActionResponse::Error(error) => {
            Err(CommandError::BrokerRejected(error))
        }
    }
}

fn broker_command_error(
    operation: BrokerOperation,
    error: wayexpand_broker_client::IpcError,
) -> CommandError {
    match error {
        wayexpand_broker_client::IpcError::DeadlineExceeded => CommandError::Timeout,
        wayexpand_broker_client::IpcError::Cancelled => CommandError::StaleInput,
        other if other.is_timeout() => CommandError::Timeout,
        wayexpand_broker_client::IpcError::ConnectionClosed => CommandError::BrokerProtocol {
            operation,
            reason: BrokerProtocolFailure::ConnectionClosed,
        },
        wayexpand_broker_client::IpcError::MessageTooLarge(size, limit) => {
            CommandError::BrokerProtocol {
                operation,
                reason: BrokerProtocolFailure::MessageTooLarge { size, limit },
            }
        }
        wayexpand_broker_client::IpcError::Json(error) => CommandError::BrokerProtocol {
            operation,
            reason: BrokerProtocolFailure::InvalidJson {
                detail: error.to_string(),
            },
        },
        wayexpand_broker_client::IpcError::Io(error) => CommandError::BrokerProtocol {
            operation,
            reason: BrokerProtocolFailure::Io {
                detail: error.to_string(),
            },
        },
    }
}

fn broker_output_text(
    output: wayexpand_broker_client::ActionOutput,
) -> Result<String, CommandError> {
    // The broker deliberately keeps a successful protocol response when
    // output is bounded, but a text expansion must never inject an incomplete
    // stdout value as if it were complete. Stderr is diagnostic-only.
    if output.stdout_truncated {
        return Err(CommandError::OutputTooLarge);
    }
    Ok(output.stdout)
}

#[cfg(unix)]
pub(super) fn run_command_unix(
    child: Child,
    mut stdout: ChildStdout,
    mut stderr: ChildStderr,
    timeout_ms: u64,
    shutdown: Option<&AtomicBool>,
) -> Result<String, CommandError> {
    let mut guard = ChildSupervisor::new(child);
    set_nonblocking_stdout(&stdout)?;
    set_nonblocking_stderr(&stderr)?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;

    let status = loop {
        if shutdown.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(CommandError::StaleInput);
        }
        if !stdout_eof {
            stdout_eof = read_available_stdout(&mut stdout, &mut bytes, Some(deadline), shutdown)?;
        }
        if !stderr_eof {
            stderr_eof = read_available_stderr(&mut stderr, &mut stderr_bytes)?;
        }
        if guard
            .has_exited()
            .map_err(|error| CommandError::WaitFailed {
                operation: ProcessWaitOperation::Observe,
                reason: ProcessWaitFailure::Io {
                    detail: error.to_string(),
                },
            })?
        {
            // On Linux this observation uses waitid(WNOWAIT), so the
            // leader remains a zombie and its PID/PGID cannot be recycled
            // while the process group is cleaned up. Only reap after the
            // group kill. The short delay is a one-time grace period, not a
            // polling interval: it lets a just-started descendant finish
            // deliberately moving into its own session (setsid) before the
            // leader's group is killed.
            thread::sleep(Duration::from_millis(10));
            guard.kill_group();
            break guard.reap().map_err(|error| CommandError::WaitFailed {
                operation: ProcessWaitOperation::Reap,
                reason: ProcessWaitFailure::Io {
                    detail: error.to_string(),
                },
            })?;
        }
        if Instant::now() < deadline {
            wait_for_command_event(
                (!stdout_eof).then_some(&stdout),
                (!stderr_eof).then_some(&stderr),
                guard.pidfd(),
                deadline,
                shutdown.is_some(),
            )?;
        } else {
            return Err(CommandError::Timeout);
        }
    };
    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while (!stdout_eof || !stderr_eof) && Instant::now() < drain_deadline {
        stdout_eof = read_available_stdout(&mut stdout, &mut bytes, None, None)?;
        stderr_eof = read_available_stderr(&mut stderr, &mut stderr_bytes)?;
        if !stdout_eof || !stderr_eof {
            wait_for_command_event(
                (!stdout_eof).then_some(&stdout),
                (!stderr_eof).then_some(&stderr),
                None,
                drain_deadline,
                false,
            )?;
        }
    }
    if !status.success() {
        return Err(CommandError::NonZeroExit {
            code: status.code(),
            stderr: diagnostic_stderr(&stderr_bytes),
        });
    }
    if !stdout_eof {
        return Err(CommandError::IncompleteOutput);
    }
    let output = String::from_utf8(bytes).map_err(|_| CommandError::InvalidUtf8)?;
    Ok(trim_trailing_newlines(output))
}

/// Drops trailing CR/LF in place. Command output is bounded at one megabyte,
/// and copying all of it into a second allocation to remove a trailing newline
/// is the kind of waste that only shows up under a command run on every
/// expansion.
fn trim_trailing_newlines(mut output: String) -> String {
    let trimmed = output.trim_end_matches(['\r', '\n']).len();
    output.truncate(trimmed);
    output
}

#[cfg(unix)]
fn set_nonblocking_stdout(stdout: &ChildStdout) -> Result<(), CommandError> {
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(CommandError::OutputChannelLost);
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(CommandError::OutputChannelLost);
    }
    Ok(())
}

#[cfg(unix)]
fn set_nonblocking_stderr(stderr: &ChildStderr) -> Result<(), CommandError> {
    let fd = stderr.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(CommandError::OutputChannelLost);
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(CommandError::OutputChannelLost);
    }
    Ok(())
}

/// Wait until an open output pipe or the child's pidfd is ready, or the
/// deadline passes. Pass `None` for a pipe that already reached EOF: a hung-up
/// pipe stays readable forever, so polling it again would spin until the child
/// exits (a command may close stdout and keep running). Negative descriptors
/// are ignored by poll(2).
#[cfg(unix)]
fn wait_for_command_event(
    stdout: Option<&ChildStdout>,
    stderr: Option<&ChildStderr>,
    pidfd: Option<&OwnedFd>,
    deadline: Instant,
    cancellation_watch: bool,
) -> Result<(), CommandError> {
    let mut fds = [
        libc::pollfd {
            fd: stdout.map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        },
        libc::pollfd {
            fd: stderr.map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        },
    ];
    if let Some(pidfd) = pidfd {
        // The extra entry is only used on Linux, where pidfd readiness means
        // the child exited and avoids periodic waitpid polling entirely.
        let mut all_fds = [
            fds[0],
            fds[1],
            libc::pollfd {
                fd: pidfd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        poll_fds(&mut all_fds, deadline, cancellation_watch)?;
    } else {
        poll_fds(&mut fds, deadline, cancellation_watch)?;
    }
    Ok(())
}

#[cfg(unix)]
fn poll_fds(
    fds: &mut [libc::pollfd],
    deadline: Instant,
    cancellation_watch: bool,
) -> Result<(), CommandError> {
    // An AtomicBool cannot be included in poll's wait set. Keep cancellation
    // responsive without allowing repeated EINTR retries to extend the
    // command deadline. The cap is itself an absolute deadline.
    let poll_deadline = if cancellation_watch {
        deadline.min(Instant::now() + Duration::from_millis(50))
    } else {
        deadline
    };
    loop {
        let timeout_ms = poll_deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(i32::MAX as u128)
            .try_into()
            .unwrap_or(i32::MAX);
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if result >= 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(CommandError::OutputChannelLost);
        }
    }
}

#[cfg(unix)]
fn read_available_stdout(
    stdout: &mut ChildStdout,
    bytes: &mut Vec<u8>,
    deadline: Option<Instant>,
    shutdown: Option<&AtomicBool>,
) -> Result<bool, CommandError> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..STDOUT_READS_PER_DRAIN {
        if shutdown.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(CommandError::StaleInput);
        }
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            return Err(CommandError::Timeout);
        }
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() > MAX_COMMAND_OUTPUT_BYTES {
                    return Err(CommandError::OutputTooLarge);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => return Err(CommandError::OutputChannelLost),
        }
    }
    Ok(false)
}

#[cfg(unix)]
const STDERR_READS_PER_DRAIN: usize = 64;

#[cfg(unix)]
const STDOUT_READS_PER_DRAIN: usize = 64;

#[cfg(unix)]
fn read_available_stderr(
    stderr: &mut ChildStderr,
    bytes: &mut Vec<u8>,
) -> Result<bool, CommandError> {
    let mut buffer = [0_u8; 4096];
    // Once the diagnostic retention cap is reached, keep draining (discarding)
    // so the child is never throttled by our bounded buffer. The per-call read
    // budget returns control to the caller's deadline check even when a child
    // writes stderr as fast as we can read it; poll reports the pipe ready
    // again immediately, so this does not slow draining down.
    for _ in 0..STDERR_READS_PER_DRAIN {
        match stderr.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                let remaining = MAX_COMMAND_STDERR_BYTES.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..count.min(remaining)]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => return Err(CommandError::OutputChannelLost),
        }
    }
    Ok(false)
}

fn diagnostic_stderr(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

pub(super) fn configure_command_environment(process: &mut Command, command: &CommandConfig) {
    if command.environment == CommandEnvironment::Inherit {
        return;
    }
    process.env_clear();
    for name in ["HOME", "USER", "LANG"] {
        if let Some(value) = std::env::var_os(name) {
            process.env(name, value);
        }
    }
    for name in &command.pass_env {
        if let Some(value) = std::env::var_os(name) {
            process.env(name, value);
        }
    }
    process.env("PATH", MINIMAL_COMMAND_PATH);
}

/// How long a snippet form may stay open before it is abandoned.
const FORM_TIMEOUT: Duration = Duration::from_secs(600);
use crate::limits::MAX_FORM_OUTPUT_BYTES;

/// The program that shows snippet forms. `WAYEXPAND_FORM_HELPER` overrides
/// it (tests, custom installs). Under systemd the GUI is started through
/// `systemd-run --user` so it runs outside the daemon's sandbox, whose
/// memory, task, and W^X limits a graphical client cannot live with.
struct FormHelperCommand {
    program: String,
    args: Vec<String>,
    transient_unit: Option<String>,
}

static FORM_UNIT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn form_helper_command() -> FormHelperCommand {
    if let Some(program) = std::env::var_os("WAYEXPAND_FORM_HELPER") {
        return FormHelperCommand {
            program: program.to_string_lossy().into_owned(),
            args: Vec::new(),
            transient_unit: None,
        };
    }
    if std::env::var_os("INVOCATION_ID").is_some() {
        let unit = format!(
            "wayexpand-form-{}-{}.service",
            std::process::id(),
            FORM_UNIT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        return FormHelperCommand {
            program: "systemd-run".into(),
            args: [
                "--user",
                "--quiet",
                "--wait",
                "--pipe",
                "--collect",
                "--unit",
                &unit,
                "--",
                "wayexpand-gui",
            ]
            .map(String::from)
            .to_vec(),
            transient_unit: Some(unit),
        };
    }
    FormHelperCommand {
        program: "wayexpand-gui".into(),
        args: Vec::new(),
        transient_unit: None,
    }
}

const FORM_OUTPUT_READS_PER_DRAIN: usize = 64;

#[cfg(unix)]
fn read_available_form_output(
    stdout: &mut ChildStdout,
    bytes: &mut Vec<u8>,
    shutdown: Option<&AtomicBool>,
) -> Result<bool, CommandError> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..FORM_OUTPUT_READS_PER_DRAIN {
        if shutdown.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(CommandError::StaleInput);
        }
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() > MAX_FORM_OUTPUT_BYTES {
                    return Err(CommandError::OutputTooLarge);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => return Err(CommandError::OutputChannelLost),
        }
    }
    Ok(false)
}

#[cfg(unix)]
fn run_bounded_systemctl(args: &[&str]) {
    let Ok(mut child) = Command::new("systemctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
}

#[cfg(unix)]
fn transient_unit_shutdown_commands(unit: &str) -> [Vec<String>; 2] {
    [
        // Every process of the unit is systemctl kill's default target;
        // --kill-whom= is not spelled out because systemd before 252 named
        // it --kill-who= and rejects the newer spelling.
        ["--user", "kill", "--signal=SIGKILL", unit]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ["--user", "stop", "--no-block", unit]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    ]
}

#[cfg(unix)]
pub(super) fn stop_transient_unit(unit: Option<&str>) {
    let Some(unit) = unit else {
        return;
    };
    // The transient service runs in a systemd-owned cgroup, separate from the
    // systemd-run wrapper's process group. Stop must therefore target every
    // process in that unit explicitly. The unit name is unique to this form
    // helper (PID plus sequence number), so a force-kill cannot affect another
    // WayExpand operation.
    for command in transient_unit_shutdown_commands(unit) {
        let args = command.iter().map(String::as_str).collect::<Vec<_>>();
        run_bounded_systemctl(&args);
    }
}

#[cfg(unix)]
fn terminate_form_helper_in_order(stop_wrapper: impl FnOnce(), stop_unit: impl FnOnce()) {
    // Stop the systemd-run client first so it cannot submit a late unit-start
    // request after the unit cleanup has already run.
    stop_wrapper();
    stop_unit();
}

#[cfg(unix)]
fn terminate_form_helper(guard: &mut ChildSupervisor, unit: Option<&str>) {
    // systemd-run owns a separate transient cgroup; stopping the wrapper's
    // group does not terminate GUI descendants in that cgroup.
    terminate_form_helper_in_order(
        || {
            guard.kill_group();
            let _ = guard.reap();
        },
        || stop_transient_unit(unit),
    );
}

/// Show a form for `fields` and return the entered values by field key.
/// The helper prints a JSON object and exits 0, or exits non-zero when the
/// user cancels. Values are checked: every field present, choices limited to
/// their options, no control characters other than tab and newline.
pub(super) fn run_form_helper(
    title: &str,
    fields: &[crate::FormField],
    shutdown: &AtomicBool,
) -> Result<std::collections::HashMap<String, String>, CommandError> {
    let spec = serde_json::json!({ "title": title, "fields": fields }).to_string();
    let helper = form_helper_command();
    let FormHelperCommand {
        program,
        mut args,
        transient_unit,
    } = helper;
    args.push("--form".into());
    args.push(spec);
    let mut process = Command::new(program);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_process_group(&mut process);
    let mut child = process.spawn().map_err(|error| CommandError::SpawnFailed {
        detail: error.to_string(),
    })?;
    // The helper was spawned with piped stdout. If the descriptor is
    // unexpectedly absent, still clean up the transient systemd unit before
    // returning; dropping the direct child alone does not own that cgroup.
    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let mut guard = ChildSupervisor::new(child);
            terminate_form_helper(&mut guard, transient_unit.as_deref());
            return Err(CommandError::OutputChannelLost);
        }
    };
    let mut guard = ChildSupervisor::new(child);
    if let Err(error) = set_nonblocking_stdout(&stdout) {
        terminate_form_helper(&mut guard, transient_unit.as_deref());
        return Err(error);
    }
    let mut bytes = Vec::new();
    let mut stdout_eof = false;
    let deadline = Instant::now() + FORM_TIMEOUT;
    let status = loop {
        if !stdout_eof {
            stdout_eof = match read_available_form_output(&mut stdout, &mut bytes, Some(shutdown)) {
                Ok(eof) => eof,
                Err(error) => {
                    terminate_form_helper(&mut guard, transient_unit.as_deref());
                    return Err(error);
                }
            };
        }
        let has_exited = match guard.has_exited() {
            Ok(has_exited) => has_exited,
            Err(error) => {
                terminate_form_helper(&mut guard, transient_unit.as_deref());
                return Err(CommandError::WaitFailed {
                    operation: ProcessWaitOperation::TryWait,
                    reason: ProcessWaitFailure::Io {
                        detail: error.to_string(),
                    },
                });
            }
        };
        if has_exited {
            guard.kill_group();
            let status = match guard.reap() {
                Ok(status) => status,
                Err(error) => {
                    stop_transient_unit(transient_unit.as_deref());
                    return Err(CommandError::WaitFailed {
                        operation: ProcessWaitOperation::Reap,
                        reason: ProcessWaitFailure::Io {
                            detail: error.to_string(),
                        },
                    });
                }
            };
            break status;
        }
        if shutdown.load(Ordering::Acquire) || Instant::now() >= deadline {
            terminate_form_helper(&mut guard, transient_unit.as_deref());
            return Err(if shutdown.load(Ordering::Acquire) {
                CommandError::StaleInput
            } else {
                CommandError::Timeout
            });
        }
        thread::sleep(Duration::from_millis(50));
    };
    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while !stdout_eof && Instant::now() < drain_deadline {
        stdout_eof = match read_available_form_output(&mut stdout, &mut bytes, None) {
            Ok(eof) => eof,
            Err(error) => {
                stop_transient_unit(transient_unit.as_deref());
                return Err(error);
            }
        };
        if !stdout_eof {
            thread::sleep(Duration::from_millis(5));
        }
    }
    if !stdout_eof {
        stop_transient_unit(transient_unit.as_deref());
        return Err(CommandError::IncompleteFormOutput);
    }
    if !status.success() {
        // Cancelled: the trigger stays as typed.
        stop_transient_unit(transient_unit.as_deref());
        return Err(CommandError::StaleInput);
    }
    if bytes.len() > MAX_FORM_OUTPUT_BYTES {
        stop_transient_unit(transient_unit.as_deref());
        return Err(CommandError::OutputTooLarge);
    }
    let values: std::collections::HashMap<String, String> = match serde_json::from_slice(&bytes) {
        Ok(values) => values,
        Err(_) => {
            stop_transient_unit(transient_unit.as_deref());
            return Err(CommandError::InvalidFormOutput);
        }
    };
    let mut checked = std::collections::HashMap::with_capacity(fields.len());
    for field in fields {
        let value = values.get(&field.key).ok_or_else(|| {
            stop_transient_unit(transient_unit.as_deref());
            CommandError::InvalidFormOutput
        })?;
        if field.validate_value(value).is_err() {
            stop_transient_unit(transient_unit.as_deref());
            return Err(CommandError::InvalidFormOutput);
        }
        checked.insert(field.key.clone(), value.clone());
    }
    Ok(checked)
}

pub(super) fn normalize_form_action_parameters(
    fields: &std::collections::HashMap<String, String>,
) -> Result<std::collections::HashMap<String, String>, CommandError> {
    let mut normalized = std::collections::HashMap::with_capacity(fields.len());
    for (field_key, value) in fields {
        let Some((kind, name)) = field_key.split_once(':') else {
            return Err(CommandError::InvalidFormOutput);
        };
        if !matches!(kind, "field" | "date" | "choice")
            || name.is_empty()
            || name.len() > 32
            || !name.as_bytes()[0].is_ascii_lowercase()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            || normalized.insert(name.to_owned(), value.clone()).is_some()
        {
            return Err(CommandError::InvalidFormOutput);
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_action_parameters_drop_template_kind_prefixes() {
        let fields = [
            ("field:namespace".into(), "platform".into()),
            ("choice:environment".into(), "staging".into()),
            ("date:incident_date".into(), "2026-10-28".into()),
        ]
        .into_iter()
        .collect();
        let normalized = normalize_form_action_parameters(&fields).unwrap();
        assert_eq!(normalized.get("namespace").unwrap(), "platform");
        assert_eq!(normalized.get("environment").unwrap(), "staging");
        assert_eq!(normalized.get("incident_date").unwrap(), "2026-10-28");
    }

    #[test]
    fn form_action_parameters_reject_ambiguous_or_unsafe_names() {
        let duplicate = [
            ("field:host".into(), "one".into()),
            ("date:host".into(), "two".into()),
        ]
        .into_iter()
        .collect();
        assert!(normalize_form_action_parameters(&duplicate).is_err());
        let invalid = [("choice:prod|stage".into(), "prod".into())]
            .into_iter()
            .collect();
        assert!(normalize_form_action_parameters(&invalid).is_err());
    }

    fn output(
        stdout_truncated: bool,
        stderr_truncated: bool,
    ) -> wayexpand_broker_client::ActionOutput {
        wayexpand_broker_client::ActionOutput {
            exit_code: 0,
            stdout: "complete".into(),
            stderr: String::new(),
            stdout_truncated,
            stderr_truncated,
            duration_ms: 1,
        }
    }

    #[cfg(unix)]
    #[test]
    fn transient_form_cleanup_kills_the_unit_cgroup_then_stops_the_unit() {
        assert_eq!(
            transient_unit_shutdown_commands("wayexpand-form-42-7.service"),
            [
                vec![
                    "--user",
                    "kill",
                    "--signal=SIGKILL",
                    "wayexpand-form-42-7.service",
                ],
                vec![
                    "--user",
                    "stop",
                    "--no-block",
                    "wayexpand-form-42-7.service",
                ],
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn form_cleanup_stops_the_wrapper_before_the_transient_unit() {
        let steps = std::cell::RefCell::new(Vec::new());
        terminate_form_helper_in_order(
            || steps.borrow_mut().push("wrapper"),
            || steps.borrow_mut().push("unit"),
        );
        assert_eq!(steps.into_inner(), ["wrapper", "unit"]);
    }

    #[test]
    fn broker_truncated_stdout_is_not_injected() {
        assert_eq!(
            broker_output_text(output(true, false)),
            Err(CommandError::OutputTooLarge)
        );
    }

    #[test]
    fn broker_truncated_stderr_does_not_discard_complete_stdout() {
        assert_eq!(
            broker_output_text(output(false, true)),
            Ok("complete".into())
        );
    }

    #[test]
    fn broker_ipc_failures_remain_structured() {
        assert_eq!(
            broker_command_error(
                BrokerOperation::ReceiveResponse,
                wayexpand_broker_client::IpcError::ConnectionClosed,
            ),
            CommandError::BrokerProtocol {
                operation: BrokerOperation::ReceiveResponse,
                reason: BrokerProtocolFailure::ConnectionClosed,
            }
        );
    }

    #[test]
    fn broker_unavailability_preserves_the_missing_socket_reason() {
        let error = CommandError::BrokerUnavailable {
            reason: BrokerUnavailableReason::SocketNotConfigured,
        };
        assert!(matches!(
            &error,
            CommandError::BrokerUnavailable {
                reason: BrokerUnavailableReason::SocketNotConfigured
            }
        ));
        assert!(error.to_string().contains("socket is not configured"));
    }

    #[test]
    fn broker_rejections_remain_structured() {
        let error = wayexpand_broker_client::ActionError::ActionBlocked {
            action_id: "local-report".into(),
            reason: "disabled by policy".into(),
        };
        let command_error = CommandError::BrokerRejected(error.clone());
        assert_eq!(command_error, CommandError::BrokerRejected(error));
        assert!(command_error.to_string().contains("disabled by policy"));
    }
}
