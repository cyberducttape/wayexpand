use std::io::Read;
use std::process::{Child, ChildStderr, ChildStdout, ExitStatus};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::fd::{AsRawFd, OwnedFd};
#[cfg(unix)]
use wayexpand_process_supervisor::ChildSupervisor;

pub const MAX_STREAM_OUTPUT_BYTES: usize = crate::protocol::MAX_OUTPUT_BYTES / 2;
const READS_PER_DRAIN: usize = 64;

#[derive(Debug)]
pub enum ChildRunError {
    Timeout,
    InvalidTimeout,
    IncompleteOutput,
    Cancelled,
    Io(std::io::Error),
}

pub type ChildRunOutput = (ExitStatus, Vec<u8>, Vec<u8>, bool, bool);

#[cfg(unix)]
#[derive(Default)]
struct ReadAvailable {
    eof: bool,
    truncated: bool,
}

#[cfg(unix)]
fn set_nonblocking<R: AsRawFd>(stream: &R) -> Result<(), std::io::Error> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn wait_for_child_event(
    stdout: Option<&ChildStdout>,
    stderr: Option<&ChildStderr>,
    pidfd: Option<&OwnedFd>,
    deadline: Instant,
    cancellation_watch: bool,
) -> Result<(), std::io::Error> {
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
        libc::pollfd {
            fd: pidfd.map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let fallback_deadline = if pidfd.is_none() {
        Some(Instant::now() + Duration::from_millis(5))
    } else {
        None
    };
    let cancellation_deadline =
        cancellation_watch.then(|| Instant::now() + Duration::from_millis(50));
    let wait_deadline = [Some(deadline), fallback_deadline, cancellation_deadline]
        .into_iter()
        .flatten()
        .min()
        .expect("the child deadline is always present");

    loop {
        let remaining = wait_deadline.saturating_duration_since(Instant::now());
        let timeout_ms = remaining
            .as_nanos()
            .div_ceil(1_000_000)
            .min(i32::MAX as u128) as i32;
        // SAFETY: fds points to a stack-owned array of initialized pollfd
        // values, each of which either names a live descriptor or is -1.
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if result >= 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
        if Instant::now() >= wait_deadline {
            return Ok(());
        }
    }
}

#[cfg(unix)]
fn read_available<R: Read>(
    stream: &mut R,
    bytes: &mut Vec<u8>,
    limit: usize,
    deadline: Instant,
) -> Result<ReadAvailable, std::io::Error> {
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    for _ in 0..READS_PER_DRAIN {
        if Instant::now() >= deadline {
            return Ok(ReadAvailable {
                eof: false,
                truncated,
            });
        }
        match stream.read(&mut buffer) {
            Ok(0) => {
                return Ok(ReadAvailable {
                    eof: true,
                    truncated,
                })
            }
            Ok(count) => {
                let remaining = limit.saturating_sub(bytes.len());
                let retained = count.min(remaining);
                bytes.extend_from_slice(&buffer[..retained]);
                if retained < count {
                    truncated = true;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(ReadAvailable {
                    eof: false,
                    truncated,
                })
            }
            Err(error) => return Err(error),
        }
    }
    Ok(ReadAvailable {
        eof: false,
        truncated,
    })
}

#[cfg(unix)]
pub fn run_child_unix(
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    timeout: Duration,
) -> Result<ChildRunOutput, ChildRunError> {
    run_child_unix_cancellable(child, stdout, stderr, timeout, None)
}

#[cfg(unix)]
pub fn run_child_unix_cancellable(
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    timeout: Duration,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<ChildRunOutput, ChildRunError> {
    let mut guard = ChildSupervisor::new(child);
    let mut stdout = stdout;
    let mut stderr = stderr;
    if let Some(stream) = stdout.as_ref() {
        set_nonblocking(stream).map_err(ChildRunError::Io)?;
    }
    if let Some(stream) = stderr.as_ref() {
        set_nonblocking(stream).map_err(ChildRunError::Io)?;
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(ChildRunError::InvalidTimeout)?;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_truncated = false;
    let mut stderr_truncated = false;
    let mut stdout_eof = stdout.is_none();
    let mut stderr_eof = stderr.is_none();

    let status = loop {
        if !stdout_eof {
            let result = read_available(
                stdout.as_mut().expect("stdout exists while not at EOF"),
                &mut stdout_bytes,
                MAX_STREAM_OUTPUT_BYTES,
                deadline,
            )
            .map_err(ChildRunError::Io)?;
            stdout_eof = result.eof;
            stdout_truncated |= result.truncated;
        }
        if !stderr_eof {
            let result = read_available(
                stderr.as_mut().expect("stderr exists while not at EOF"),
                &mut stderr_bytes,
                MAX_STREAM_OUTPUT_BYTES,
                deadline,
            )
            .map_err(ChildRunError::Io)?;
            stderr_eof = result.eof;
            stderr_truncated |= result.truncated;
        }
        if guard.has_exited().map_err(ChildRunError::Io)? {
            guard.kill_group();
            let status = guard.reap().map_err(ChildRunError::Io)?;
            break status;
        }
        if cancelled
            .as_ref()
            .is_some_and(|token| token.load(Ordering::Acquire))
        {
            guard.kill_group();
            let _ = guard.reap();
            return Err(ChildRunError::Cancelled);
        }
        if Instant::now() >= deadline {
            guard.kill_group();
            let _ = guard.reap();
            return Err(ChildRunError::Timeout);
        }
        wait_for_child_event(
            stdout.as_ref().filter(|_| !stdout_eof),
            stderr.as_ref().filter(|_| !stderr_eof),
            guard.pidfd(),
            deadline,
            cancelled.is_some(),
        )
        .map_err(ChildRunError::Io)?;
    };

    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while (!stdout_eof || !stderr_eof) && Instant::now() < drain_deadline {
        if !stdout_eof {
            let result = read_available(
                stdout.as_mut().expect("stdout exists while not at EOF"),
                &mut stdout_bytes,
                MAX_STREAM_OUTPUT_BYTES,
                drain_deadline,
            )
            .map_err(ChildRunError::Io)?;
            stdout_eof = result.eof;
            stdout_truncated |= result.truncated;
        }
        if !stderr_eof {
            let result = read_available(
                stderr.as_mut().expect("stderr exists while not at EOF"),
                &mut stderr_bytes,
                MAX_STREAM_OUTPUT_BYTES,
                drain_deadline,
            )
            .map_err(ChildRunError::Io)?;
            stderr_eof = result.eof;
            stderr_truncated |= result.truncated;
        }
        if !stdout_eof || !stderr_eof {
            wait_for_child_event(
                stdout.as_ref().filter(|_| !stdout_eof),
                stderr.as_ref().filter(|_| !stderr_eof),
                None,
                drain_deadline,
                false,
            )
            .map_err(ChildRunError::Io)?;
        }
    }
    if !stdout_eof || !stderr_eof {
        return Err(ChildRunError::IncompleteOutput);
    }
    Ok((
        status,
        stdout_bytes,
        stderr_bytes,
        stdout_truncated,
        stderr_truncated,
    ))
}

#[cfg(not(unix))]
pub fn run_child_fallback(
    mut child: Child,
    stdout: Option<impl Read + Send + 'static>,
    stderr: Option<impl Read + Send + 'static>,
    timeout: Duration,
) -> Result<ChildRunOutput, ChildRunError> {
    let Some(deadline) = Instant::now().checked_add(timeout) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ChildRunError::InvalidTimeout);
    };
    let stdout_thread =
        std::thread::spawn(move || stdout.map(bounded_read_stream).unwrap_or_default());
    let stderr_thread =
        std::thread::spawn(move || stderr.map(bounded_read_stream).unwrap_or_default());
    loop {
        match child.try_wait().map_err(ChildRunError::Io)? {
            Some(status) => {
                let stdout = stdout_thread.join().unwrap_or_default();
                let stderr = stderr_thread.join().unwrap_or_default();
                let stdout_truncated = stdout.len() >= MAX_STREAM_OUTPUT_BYTES;
                let stderr_truncated = stderr.len() >= MAX_STREAM_OUTPUT_BYTES;
                return Ok((status, stdout, stderr, stdout_truncated, stderr_truncated));
            }
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ChildRunError::Timeout);
            }
        }
    }
}

#[cfg(not(unix))]
fn bounded_read_stream<R: Read>(stream: Option<R>) -> Vec<u8> {
    let Some(stream) = stream else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    let _ = stream
        .take((MAX_STREAM_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut buf);
    buf.truncate(MAX_STREAM_OUTPUT_BYTES);
    buf
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use wayexpand_process_supervisor::configure_process_group;

    #[test]
    fn continuous_output_cannot_starve_the_action_deadline() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "yes wayexpand"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        let mut child = command.spawn().expect("spawn continuous-output child");
        let stdout = child.stdout.take().expect("child stdout");
        let started = Instant::now();
        let result = run_child_unix(child, Some(stdout), None, Duration::from_millis(50));
        assert!(matches!(result, Err(ChildRunError::Timeout)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "continuous output starved the timeout: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn continuous_stderr_output_cannot_starve_the_action_deadline() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "yes wayexpand >&2"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        configure_process_group(&mut command);
        let mut child = command.spawn().expect("spawn continuous-stderr child");
        let stderr = child.stderr.take().expect("child stderr");
        let started = Instant::now();
        let result = run_child_unix(child, None, Some(stderr), Duration::from_millis(50));
        assert!(matches!(result, Err(ChildRunError::Timeout)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "continuous stderr output starved the timeout: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn broker_shutdown_cancels_a_continuous_action_promptly() {
        use std::sync::atomic::AtomicBool;

        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "while :; do printf x; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        let mut child = command.spawn().expect("spawn continuous-output child");
        let stdout = child.stdout.take().expect("child stdout");
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            signal.store(true, Ordering::Release);
        });

        let started = Instant::now();
        let result = run_child_unix_cancellable(
            child,
            Some(stdout),
            None,
            Duration::from_secs(10),
            Some(cancelled),
        );

        assert!(matches!(result, Err(ChildRunError::Cancelled)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "shutdown did not cancel continuous action output promptly: {:?}",
            started.elapsed()
        );
    }
}
