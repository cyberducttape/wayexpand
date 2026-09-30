//! Asynchronous command execution and hotkey action handling.
//!
//! Organizes worker threads for executing expansion commands and hotkey actions
//! with bounded queueing and timeout enforcement.

use std::{
    io::Read,
    process::{Child, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use super::{
    CommandConfig, CommandEnvironment, CommandError, MAX_COMMAND_OUTPUT_BYTES, MINIMAL_COMMAND_PATH,
};

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
            .stderr(Stdio::null());
        configure_process_group(&mut process);
        let mut child = process.spawn().map_err(|_| CommandError::SpawnFailed)?;
        let stdout = child.stdout.take().ok_or(CommandError::SpawnFailed)?;
        run_command_unix(child, stdout, command.timeout_ms, shutdown)
    }

    #[cfg(not(unix))]
    {
        let _ = (command, shutdown);
        unreachable!("wayexpand-core requires a Unix target")
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
    timeout_ms: u64,
    shutdown: Option<&AtomicBool>,
) -> Result<String, CommandError> {
    let pid = child.id();
    let mut guard = ChildGuard {
        child: Some(child),
        pid: Some(pid),
    };
    set_nonblocking_stdout(&stdout)?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut bytes = Vec::new();
    let mut stdout_eof = false;

    let status = loop {
        if shutdown.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(CommandError::StaleInput);
        }
        if !stdout_eof {
            stdout_eof = read_available_stdout(&mut stdout, &mut bytes)?;
        }
        match guard.child.as_mut().unwrap().try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => return Err(CommandError::Timeout),
            Err(error) => return Err(CommandError::WaitFailed(error.to_string())),
        }
    };

    if let Some(pid) = guard.pid {
        kill_process_group_by_pid(pid);
        guard.pid = None;
    }
    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while !stdout_eof && Instant::now() < drain_deadline {
        stdout_eof = read_available_stdout(&mut stdout, &mut bytes)?;
        if !stdout_eof {
            thread::sleep(Duration::from_millis(5));
        }
    }
    if !status.success() {
        return Err(CommandError::NonZeroExit(status.code()));
    }
    if !stdout_eof {
        return Err(CommandError::IncompleteOutput);
    }
    let output = String::from_utf8(bytes).map_err(|_| CommandError::InvalidUtf8)?;
    guard.child = None;
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
