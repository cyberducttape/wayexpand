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
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use super::{
    CommandConfig, CommandEnvironment, CommandError, MAX_COMMAND_OUTPUT_BYTES, MINIMAL_COMMAND_PATH,
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
    if let Some(action_id) = &command.action {
        return run_broker_action(action_id, command, shutdown);
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
        let mut child = process.spawn().map_err(|_| CommandError::SpawnFailed)?;
        let stdout = child.stdout.take().ok_or(CommandError::SpawnFailed)?;
        let stderr = child.stderr.take().ok_or(CommandError::SpawnFailed)?;
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
    shutdown: Option<&AtomicBool>,
) -> Result<String, CommandError> {
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
        .ok_or_else(|| CommandError::WaitFailed("action broker socket is not configured".into()))?;
    let mut client = action_broker::BrokerClient::connect(socket).map_err(|error| {
        CommandError::WaitFailed(format!("connecting to action broker: {error}"))
    })?;
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
        .send_request(&action_broker::ActionRequest {
            action_id: action_id.to_owned(),
            timeout_ms: command.timeout_ms,
            inherit_env: false,
            env_vars,
            stdout_capture: true,
        })
        .map_err(|error| CommandError::WaitFailed(format!("sending broker request: {error}")))?;
    match client
        .recv_response()
        .map_err(|error| CommandError::WaitFailed(format!("receiving broker response: {error}")))?
    {
        action_broker::ActionResponse::Success(output) => Ok(output.stdout),
        action_broker::ActionResponse::Error(error) => {
            Err(CommandError::WaitFailed(error.to_string()))
        }
    }
}

#[cfg(unix)]
struct ChildGuard {
    child: Option<Child>,
    pid: Option<u32>,
}

#[cfg(unix)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            kill_process_group_by_pid(pid);
        }
        if let Some(ref mut child) = self.child {
            let _ = child.wait();
        }
    }
}

#[cfg(unix)]
fn run_command_unix(
    child: Child,
    mut stdout: ChildStdout,
    mut stderr: ChildStderr,
    timeout_ms: u64,
    shutdown: Option<&AtomicBool>,
) -> Result<String, CommandError> {
    let pid = child.id();
    let mut guard = ChildGuard {
        child: Some(child),
        pid: Some(pid),
    };
    set_nonblocking_stdout(&stdout)?;
    set_nonblocking_stderr(&stderr)?;
    let pidfd = open_pidfd(pid);
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
            stdout_eof = read_available_stdout(&mut stdout, &mut bytes)?;
        }
        if !stderr_eof {
            stderr_eof = read_available_stderr(&mut stderr, &mut stderr_bytes)?;
        }
        match child_exit_observed(guard.child.as_mut().unwrap(), pid, pidfd.as_ref())? {
            ChildExitObservation::Running => {}
            ChildExitObservation::Exited(status) => {
                // On Linux this observation uses waitid(WNOWAIT), so the
                // leader remains a zombie and its PID/PGID cannot be
                // recycled while the process group is cleaned up. Only reap
                // after the group kill.
                thread::sleep(Duration::from_millis(10));
                #[cfg(target_os = "linux")]
                kill_process_group_by_pid(pid);
                guard.pid = None;
                if let Some(status) = status {
                    break status;
                }
                let mut child = guard.child.take().expect("child guard owns the child");
                break child
                    .wait()
                    .map_err(|error| CommandError::WaitFailed(error.to_string()))?;
            }
        }
        if Instant::now() < deadline {
            wait_for_command_event(
                &stdout,
                &stderr,
                pidfd.as_ref(),
                deadline,
                shutdown.is_some(),
            )?;
        } else {
            return Err(CommandError::Timeout);
        }
    };
    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while (!stdout_eof || !stderr_eof) && Instant::now() < drain_deadline {
        stdout_eof = read_available_stdout(&mut stdout, &mut bytes)?;
        stderr_eof = read_available_stderr(&mut stderr, &mut stderr_bytes)?;
        if !stdout_eof || !stderr_eof {
            wait_for_command_event(&stdout, &stderr, None, drain_deadline, false)?;
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
    guard.child = None;
    Ok(trim_trailing_newlines(output))
}

#[cfg(unix)]
enum ChildExitObservation {
    Running,
    Exited(Option<std::process::ExitStatus>),
}

/// Observe whether the child has exited without reaping it, so the leader
/// stays a zombie and its PID/PGID cannot be recycled before the group kill.
#[cfg(target_os = "linux")]
fn child_exit_observed(
    _child: &mut Child,
    pid: u32,
    pidfd: Option<&OwnedFd>,
) -> Result<ChildExitObservation, CommandError> {
    let (id_type, id) = if let Some(pidfd) = pidfd {
        (libc::P_PIDFD, pidfd.as_raw_fd() as libc::id_t)
    } else {
        (libc::P_PID, pid as libc::id_t)
    };
    // SAFETY: siginfo_t is plain data; waitid only writes into `info`.
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    let result = unsafe {
        libc::waitid(
            id_type,
            id,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(CommandError::WaitFailed(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(if unsafe { info.si_pid() } != 0 {
        ChildExitObservation::Exited(None)
    } else {
        ChildExitObservation::Running
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn child_exit_observed(
    child: &mut Child,
    _pid: u32,
    _pidfd: Option<&OwnedFd>,
) -> Result<ChildExitObservation, CommandError> {
    child
        .try_wait()
        .map(|status| match status {
            Some(status) => ChildExitObservation::Exited(Some(status)),
            None => ChildExitObservation::Running,
        })
        .map_err(|error| CommandError::WaitFailed(error.to_string()))
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

#[cfg(unix)]
fn open_pidfd(pid: u32) -> Option<OwnedFd> {
    #[cfg(target_os = "linux")]
    {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        if fd >= 0 {
            // SAFETY: the successful syscall returned a newly-owned fd.
            return Some(unsafe { OwnedFd::from_raw_fd(fd as RawFd) });
        }
    }
    None
}

#[cfg(unix)]
fn wait_for_command_event(
    stdout: &ChildStdout,
    stderr: &ChildStderr,
    pidfd: Option<&OwnedFd>,
    deadline: Instant,
    cancellation_watch: bool,
) -> Result<(), CommandError> {
    let mut fds = [
        libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        },
        libc::pollfd {
            fd: stderr.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        },
    ];
    if let Some(pidfd) = pidfd {
        // The extra entry is only used on Linux, where pidfd readiness means
        // the child exited and avoids periodic waitpid polling entirely.
        fds[0].revents = 0;
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
) -> Result<bool, CommandError> {
    let mut buffer = [0_u8; 8192];
    loop {
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
}

#[cfg(unix)]
fn read_available_stderr(
    stderr: &mut ChildStderr,
    bytes: &mut Vec<u8>,
) -> Result<bool, CommandError> {
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                let remaining = MAX_COMMAND_STDERR_BYTES.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..count.min(remaining)]);
                if bytes.len() >= MAX_COMMAND_STDERR_BYTES {
                    return Ok(false);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => return Err(CommandError::OutputChannelLost),
        }
    }
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

#[cfg(unix)]
pub(super) fn configure_process_group(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(unix)]
pub(super) fn kill_process_group_by_pid(pid: u32) {
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}
