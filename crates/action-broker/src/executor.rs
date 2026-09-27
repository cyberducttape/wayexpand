//! Action Executor - Safe command execution with policy enforcement.
//!
//! Executes actions within the constraints defined in ActionConfig and BrokerConfig.

use crate::{
    config::BrokerConfig,
    protocol::{ActionError, ActionOutput, ActionRequest, ActionResponse},
};
use std::collections::HashMap;
use std::io::Read;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024; // 1 MiB per stream

pub struct ActionExecutor {
    config: BrokerConfig,
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
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
fn kill_process_group(pid: u32) {
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[derive(Debug)]
enum ChildRunError {
    Timeout,
    Io(std::io::Error),
}

#[cfg(unix)]
struct ChildGuard {
    child: Option<Child>,
    pid: u32,
}

#[cfg(unix)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        kill_process_group(self.pid);
        if let Some(child) = self.child.as_mut() {
            let _ = child.wait();
        }
    }
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
fn read_available<R: Read>(stream: &mut R, bytes: &mut Vec<u8>) -> Result<bool, std::io::Error> {
    let mut buffer = [0_u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() >= MAX_OUTPUT_BYTES {
                    bytes.truncate(MAX_OUTPUT_BYTES);
                    return Ok(false);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error),
        }
    }
}

#[cfg(unix)]
fn run_child_unix(
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), ChildRunError> {
    let pid = child.id();
    let mut guard = ChildGuard {
        child: Some(child),
        pid,
    };
    let mut stdout = stdout;
    let mut stderr = stderr;
    if let Some(stream) = stdout.as_ref() {
        set_nonblocking(stream).map_err(ChildRunError::Io)?;
    }
    if let Some(stream) = stderr.as_ref() {
        set_nonblocking(stream).map_err(ChildRunError::Io)?;
    }
    let deadline = Instant::now() + timeout;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = stdout.is_none();
    let mut stderr_eof = stderr.is_none();

    let status = loop {
        if !stdout_eof {
            stdout_eof = read_available(
                stdout.as_mut().expect("stdout exists while not at EOF"),
                &mut stdout_bytes,
            )
            .map_err(ChildRunError::Io)?;
        }
        if !stderr_eof {
            stderr_eof = read_available(
                stderr.as_mut().expect("stderr exists while not at EOF"),
                &mut stderr_bytes,
            )
            .map_err(ChildRunError::Io)?;
        }
        match guard
            .child
            .as_mut()
            .expect("child guard is armed")
            .try_wait()
            .map_err(ChildRunError::Io)?
        {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            None => {
                kill_process_group(pid);
                let _ = guard.child.as_mut().expect("child guard is armed").wait();
                guard.child = None;
                return Err(ChildRunError::Timeout);
            }
        }
    };

    // A successful/failed leader may have left ordinary descendants behind.
    // Remove the whole action process group before returning to the caller.
    kill_process_group(pid);
    guard.child = None;
    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while (!stdout_eof || !stderr_eof) && Instant::now() < drain_deadline {
        if !stdout_eof {
            stdout_eof = read_available(
                stdout.as_mut().expect("stdout exists while not at EOF"),
                &mut stdout_bytes,
            )
            .map_err(ChildRunError::Io)?;
        }
        if !stderr_eof {
            stderr_eof = read_available(
                stderr.as_mut().expect("stderr exists while not at EOF"),
                &mut stderr_bytes,
            )
            .map_err(ChildRunError::Io)?;
        }
        if !stdout_eof || !stderr_eof {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    Ok((status, stdout_bytes, stderr_bytes))
}

#[cfg(not(unix))]
fn run_child_fallback(
    mut child: Child,
    stdout: Option<impl Read + Send + 'static>,
    stderr: Option<impl Read + Send + 'static>,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), ChildRunError> {
    let deadline = Instant::now() + timeout;
    let stdout_thread =
        std::thread::spawn(move || stdout.map(bounded_read_stream).unwrap_or_default());
    let stderr_thread =
        std::thread::spawn(move || stderr.map(bounded_read_stream).unwrap_or_default());
    loop {
        match child.try_wait().map_err(ChildRunError::Io)? {
            Some(status) => {
                let stdout = stdout_thread.join().unwrap_or_default();
                let stderr = stderr_thread.join().unwrap_or_default();
                return Ok((status, stdout, stderr));
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

/// Read up to MAX_OUTPUT_BYTES from a stream, silently truncating beyond that.
#[cfg(not(unix))]
fn bounded_read_stream<R: Read>(stream: Option<R>) -> Vec<u8> {
    let Some(stream) = stream else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    // Read one byte beyond the limit to detect truncation, then truncate.
    let _ = stream
        .take((MAX_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut buf);
    buf.truncate(MAX_OUTPUT_BYTES);
    buf
}

impl ActionExecutor {
    pub fn new(config: &BrokerConfig) -> Result<Self, Box<dyn std::error::Error>> {
        config.validate()?;
        Ok(Self {
            config: config.clone(),
        })
    }

    /// Resolve the effective timeout: the broker config is the upper bound.
    fn effective_timeout(&self, request: &ActionRequest, action_timeout_ms: u64) -> Duration {
        let config_timeout = action_timeout_ms;
        let effective = config_timeout.min(request.timeout_ms);
        Duration::from_millis(effective)
    }

    /// Execute an action with process-group containment and policy enforcement.
    pub async fn execute(&self, request: ActionRequest) -> Result<ActionResponse, ActionError> {
        let action_config =
            self.config
                .get_action(&request.action_id)
                .ok_or(ActionError::ActionNotFound {
                    action_id: request.action_id.clone(),
                })?;

        if !action_config.is_enabled() {
            return Err(ActionError::ActionBlocked {
                action_id: request.action_id.clone(),
                reason: "action is disabled".to_string(),
            });
        }

        let mut cmd = Command::new(&action_config.program);

        for arg in &action_config.args_prefix {
            cmd.arg(arg);
        }

        if let Some(cwd) = &action_config.cwd {
            cmd.current_dir(cwd);
        } else if let Some(default_cwd) = &self.config.default_cwd {
            cmd.current_dir(default_cwd);
        }

        cmd.env_clear();

        // The action configuration is authoritative; request.inherit_env is
        // deliberately ignored. Even an explicitly inheriting action receives
        // only the command runner's small baseline plus its pass_env entries.
        if action_config.inherit_env && !self.config.strict_env {
            for key in ["HOME", "USER", "LANG", "PATH"] {
                if let Some(value) = std::env::var_os(key) {
                    cmd.env(key, value);
                }
            }
            for key in &action_config.pass_env {
                if let Some(value) = std::env::var_os(key) {
                    cmd.env(key, value);
                }
            }
        } else {
            // Restricted mode: accept only client-supplied variables that appear
            // in the action's pass_env allowlist. An empty allowlist means
            // nothing is permitted.
            let allowed_vars = self.build_env_map(&request.env_vars, &action_config.pass_env);
            for (key, value) in allowed_vars {
                cmd.env(&key, value);
            }
        }

        let capture_stdout = request.stdout_capture;
        if capture_stdout {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            // The broker service's stdout/stderr are normally collected by its
            // service manager (for example journald). Do not buffer discarded
            // action output in the broker when the caller did not request it.
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }

        #[cfg(unix)]
        configure_process_group(&mut cmd);

        let start = Instant::now();

        let mut child = cmd.spawn().map_err(|e| ActionError::SpawnFailed {
            action_id: request.action_id.clone(),
            program: action_config.program.clone(),
            reason: e.to_string(),
        })?;

        let timeout = self.effective_timeout(&request, action_config.timeout_ms);
        let action_id = request.action_id.clone();

        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();
        // The blocking task owns the child for its entire lifetime. Its own
        // deadline kills and reaps the process group, so dropping/cancelling
        // this future cannot orphan an action behind Tokio's scheduler.
        let result = tokio::task::spawn_blocking(move || {
            #[cfg(unix)]
            {
                run_child_unix(child, child_stdout, child_stderr, timeout)
            }
            #[cfg(not(unix))]
            {
                run_child_fallback(child, child_stdout, child_stderr, timeout)
            }
        })
        .await
        .map_err(|e| ActionError::Internal {
            reason: format!("task join error: {}", e),
        })?;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok((status, stdout_bytes, stderr_bytes)) => {
                let exit_code = status.code().unwrap_or(-1);
                let stdout = if capture_stdout {
                    String::from_utf8_lossy(&stdout_bytes).to_string()
                } else {
                    String::new()
                };
                let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

                if exit_code == 0 {
                    Ok(ActionResponse::Success(ActionOutput {
                        exit_code,
                        stdout,
                        stderr,
                        duration_ms,
                    }))
                } else {
                    Err(ActionError::ExitFailure {
                        action_id: request.action_id.clone(),
                        exit_code,
                        stderr: stderr.trim().to_string(),
                    })
                }
            }
            Err(ChildRunError::Timeout) => Err(ActionError::Timeout {
                action_id,
                timeout_ms: timeout.as_millis() as u64,
            }),
            Err(ChildRunError::Io(e)) => Err(ActionError::Internal {
                reason: format!("child process error: {}", e),
            }),
        }
    }

    /// Build environment map from request variables and allowed list.
    ///
    /// Only variables whose names appear in `allowed_vars` are accepted.
    /// An empty allowlist means nothing is permitted.
    fn build_env_map(
        &self,
        request_vars: &[String],
        allowed_vars: &[String],
    ) -> HashMap<String, String> {
        if allowed_vars.is_empty() {
            return HashMap::new();
        }

        let mut map = HashMap::new();
        for var_str in request_vars {
            if let Some((key, value)) = var_str.split_once('=') {
                if allowed_vars.contains(&key.to_string()) {
                    map.insert(key.to_string(), value.to_string());
                }
            }
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ActionConfig;

    #[tokio::test]
    async fn executor_action_not_found() {
        let config = BrokerConfig::default();
        let executor = ActionExecutor::new(&config).unwrap();

        let request = ActionRequest {
            action_id: "nonexistent".to_string(),
            timeout_ms: 5000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ActionError::ActionNotFound { .. })));
    }

    #[tokio::test]
    async fn executor_disabled_action() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "disabled".to_string(),
            ActionConfig {
                program: "/bin/echo".to_string(),
                args_prefix: vec![],
                timeout_ms: 5000,
                pass_env: vec![],
                inherit_env: false,
                cwd: None,
                enabled: false,
                description: None,
            },
        );

        let executor = ActionExecutor::new(&config).unwrap();
        let request = ActionRequest {
            action_id: "disabled".to_string(),
            timeout_ms: 5000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ActionError::ActionBlocked { .. })));
    }

    #[test]
    fn executor_env_map_building() {
        let config = BrokerConfig::default();
        let executor = ActionExecutor::new(&config).unwrap();

        let request_vars = vec![
            "HOME=/home/user".to_string(),
            "SECRET=should_be_filtered".to_string(),
        ];
        let allowed_vars = vec!["HOME".to_string()];

        let map = executor.build_env_map(&request_vars, &allowed_vars);
        assert_eq!(map.get("HOME"), Some(&"/home/user".to_string()));
        assert!(!map.contains_key("SECRET"));
    }

    #[tokio::test]
    async fn executor_timeout_kills_process() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "sleeper".to_string(),
            ActionConfig {
                program: "/bin/sleep".to_string(),
                args_prefix: vec!["60".to_string()],
                timeout_ms: 200,
                pass_env: vec![],
                inherit_env: false,
                cwd: None,
                enabled: true,
                description: None,
            },
        );

        let executor = ActionExecutor::new(&config).unwrap();
        let request = ActionRequest {
            action_id: "sleeper".to_string(),
            timeout_ms: 10000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ActionError::Timeout { .. })));
        if let Err(ActionError::Timeout { timeout_ms, .. }) = &result {
            assert_eq!(*timeout_ms, 200);
        }
    }

    #[tokio::test]
    async fn executor_client_cannot_exceed_config_timeout() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "echo".to_string(),
            ActionConfig {
                program: "/bin/echo".to_string(),
                args_prefix: vec!["hi".to_string()],
                timeout_ms: 500,
                pass_env: vec![],
                inherit_env: false,
                cwd: None,
                enabled: true,
                description: None,
            },
        );

        let executor = ActionExecutor::new(&config).unwrap();
        let effective = executor.effective_timeout(
            &ActionRequest {
                action_id: "echo".to_string(),
                timeout_ms: 99999,
                inherit_env: false,
                env_vars: vec![],
                stdout_capture: true,
            },
            500,
        );
        assert_eq!(effective, Duration::from_millis(500));
    }

    #[tokio::test]
    async fn executor_empty_allowlist_does_not_inherit_client_environment() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "print-env".to_string(),
            ActionConfig {
                program: "/usr/bin/env".to_string(),
                args_prefix: vec![],
                timeout_ms: 1000,
                pass_env: vec![],
                inherit_env: false,
                cwd: None,
                enabled: true,
                description: None,
            },
        );
        let executor = ActionExecutor::new(&config).unwrap();
        let response = executor
            .execute(ActionRequest {
                action_id: "print-env".to_string(),
                timeout_ms: 1000,
                inherit_env: true,
                env_vars: vec![
                    "HOME=/should-not-pass".to_string(),
                    "LD_PRELOAD=/should-not-pass.so".to_string(),
                ],
                stdout_capture: true,
            })
            .await
            .expect("environment command should succeed");

        let output = response.output().expect("successful action output");
        assert!(
            output.stdout.is_empty(),
            "unexpected environment: {}",
            output.stdout
        );
    }
}
