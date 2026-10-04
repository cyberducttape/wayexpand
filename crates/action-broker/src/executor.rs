//! Action Executor - Safe command execution with policy enforcement.
//!
//! Executes actions within the constraints defined in ActionConfig and BrokerConfig.

use crate::{
    config::BrokerConfig,
    protocol::{ActionError, ActionOutput, ActionRequest, ActionResponse, MAX_OUTPUT_BYTES},
};
use std::collections::HashMap;
use std::io::Read;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
#[cfg(unix)]
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const MAX_STREAM_OUTPUT_BYTES: usize = MAX_OUTPUT_BYTES / 2;

pub struct ActionExecutor {
    config: BrokerConfig,
}

#[derive(Debug)]
enum ChildRunError {
    Timeout,
    IncompleteOutput,
    Io(std::io::Error),
}

type ChildRunOutput = (ExitStatus, Vec<u8>, Vec<u8>, bool, bool);

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
fn read_available<R: Read>(
    stream: &mut R,
    bytes: &mut Vec<u8>,
    limit: usize,
) -> Result<ReadAvailable, std::io::Error> {
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
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
}

#[cfg(unix)]
fn run_child_unix(
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    timeout: Duration,
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
    let deadline = Instant::now() + timeout;
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
            )
            .map_err(ChildRunError::Io)?;
            stderr_eof = result.eof;
            stderr_truncated |= result.truncated;
        }
        if guard.has_exited().map_err(ChildRunError::Io)? {
            // On Linux has_exited uses waitid(WNOWAIT), keeping the leader
            // unreaped until the entire process group has been terminated.
            guard.kill_group();
            let status = guard.reap().map_err(ChildRunError::Io)?;
            break status;
        }
        if Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        } else {
            guard.kill_group();
            let _ = guard.reap();
            return Err(ChildRunError::Timeout);
        }
    };

    // A successful/failed leader may have left ordinary descendants behind.
    // Remove the whole action process group before returning to the caller.
    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while (!stdout_eof || !stderr_eof) && Instant::now() < drain_deadline {
        if !stdout_eof {
            let result = read_available(
                stdout.as_mut().expect("stdout exists while not at EOF"),
                &mut stdout_bytes,
                MAX_STREAM_OUTPUT_BYTES,
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
            )
            .map_err(ChildRunError::Io)?;
            stderr_eof = result.eof;
            stderr_truncated |= result.truncated;
        }
        if !stdout_eof || !stderr_eof {
            std::thread::sleep(Duration::from_millis(5));
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
fn run_child_fallback(
    mut child: Child,
    stdout: Option<impl Read + Send + 'static>,
    stderr: Option<impl Read + Send + 'static>,
    timeout: Duration,
) -> Result<ChildRunOutput, ChildRunError> {
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

/// Read up to one stream's share of MAX_OUTPUT_BYTES, retaining whether the
/// stream exceeded that budget.
#[cfg(not(unix))]
fn bounded_read_stream<R: Read>(stream: Option<R>) -> Vec<u8> {
    let Some(stream) = stream else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    // Read one byte beyond the limit to detect truncation, then truncate.
    let _ = stream
        .take((MAX_STREAM_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut buf);
    buf.truncate(MAX_STREAM_OUTPUT_BYTES);
    buf
}

impl ActionExecutor {
    pub fn new(config: &BrokerConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = config.clone();
        config.validate_and_canonicalize()?;
        Ok(Self { config })
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

        for arg in &action_config.args {
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
        // only the command runner's small baseline plus broker-owned values.
        if action_config.inherit_env && !self.config.strict_env {
            for key in ["HOME", "USER", "LANG"] {
                if let Some(value) = std::env::var_os(key) {
                    cmd.env(key, value);
                }
            }
        }
        for key in &action_config.server_env {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        let allowed_vars = self.build_env_map(&request.env_vars, &action_config.client_forward_env);
        for (key, value) in allowed_vars {
            cmd.env(&key, value);
        }

        // Actions are non-interactive. Never hand them the broker's stdin,
        // which is a terminal when the broker is started by hand.
        cmd.stdin(Stdio::null());
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
            Ok((status, stdout_bytes, stderr_bytes, stdout_truncated, stderr_truncated)) => {
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
                        stdout_truncated,
                        stderr_truncated,
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
            Err(ChildRunError::IncompleteOutput) => Err(ActionError::Internal {
                reason: "child output stream did not close before the drain deadline; output may be incomplete".to_string(),
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
                if !value.contains('\0') && allowed_vars.iter().any(|allowed| allowed == key) {
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
                args: vec![],
                timeout_ms: 5000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
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

        let invalid =
            executor.build_env_map(&["HOME=contains\0nul".to_string()], &["HOME".to_string()]);
        assert!(invalid.is_empty());
    }

    #[tokio::test]
    async fn executor_timeout_kills_process() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "sleeper".to_string(),
            ActionConfig {
                program: "/bin/sleep".to_string(),
                args: vec!["60".to_string()],
                timeout_ms: 200,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
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
    #[cfg(unix)]
    async fn executor_rejects_output_when_descendant_keeps_pipe_open() {
        let pid_file = format!(
            "/tmp/wayexpand-broker-incomplete-output-{}.pid",
            std::process::id()
        );
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "holds-output".to_string(),
            ActionConfig {
                program: "/bin/sh".to_string(),
                args: vec![
                    "-c".to_string(),
                    format!(
                        "/usr/bin/setsid /bin/sh -c '/bin/sleep 10 & echo $! > {pid_file}; wait' & while [ ! -s {pid_file} ]; do /bin/sleep 0.01; done; printf complete"
                    ),
                ],
                timeout_ms: 5_000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                enabled: true,
                description: None,
            },
        );
        let executor = ActionExecutor::new(&config).unwrap();
        let request = ActionRequest {
            action_id: "holds-output".to_string(),
            timeout_ms: 5_000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };

        let result = executor.execute(request).await;
        if let Ok(pid) = std::fs::read_to_string(&pid_file).and_then(|text| {
            text.trim()
                .parse::<libc::pid_t>()
                .map_err(std::io::Error::other)
        }) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        let _ = std::fs::remove_file(pid_file);
        assert!(
            matches!(
                &result,
                Err(ActionError::Internal { reason }) if reason.contains("output may be incomplete")
            ),
            "unexpected result: {result:?}"
        );
    }

    #[tokio::test]
    async fn executor_client_cannot_exceed_config_timeout() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "echo".to_string(),
            ActionConfig {
                program: "/bin/echo".to_string(),
                args: vec!["hi".to_string()],
                timeout_ms: 500,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
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
                args: vec![],
                timeout_ms: 1000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
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

    #[tokio::test]
    async fn executor_reads_server_environment_and_rejects_client_override() {
        std::env::set_var("WAYEXPAND_BROKER_TEST_VALUE", "broker-owned");
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "print-env".to_string(),
            ActionConfig {
                program: "/usr/bin/env".to_string(),
                args: vec![],
                timeout_ms: 1000,
                server_env: vec!["WAYEXPAND_BROKER_TEST_VALUE".to_string()],
                client_forward_env: vec!["CLIENT_VALUE".to_string()],
                allow_dangerous_env: false,
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
                inherit_env: false,
                env_vars: vec![
                    "WAYEXPAND_BROKER_TEST_VALUE=client-spoof".to_string(),
                    "CLIENT_VALUE=accepted".to_string(),
                ],
                stdout_capture: true,
            })
            .await
            .expect("environment command should succeed");
        let stdout = &response.output().unwrap().stdout;
        assert!(stdout
            .lines()
            .any(|line| line == "WAYEXPAND_BROKER_TEST_VALUE=broker-owned"));
        assert!(stdout.lines().any(|line| line == "CLIENT_VALUE=accepted"));
        assert!(!stdout.lines().any(|line| line.contains("client-spoof")));
        std::env::remove_var("WAYEXPAND_BROKER_TEST_VALUE");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn executor_drains_large_output_without_timing_out() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "noisy".to_string(),
            ActionConfig {
                program: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "yes x | head -c 20971520".to_string()],
                timeout_ms: 2000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                enabled: true,
                description: None,
            },
        );
        let executor = ActionExecutor::new(&config).unwrap();
        let response = executor
            .execute(ActionRequest {
                action_id: "noisy".to_string(),
                timeout_ms: 2000,
                inherit_env: false,
                env_vars: vec![],
                stdout_capture: true,
            })
            .await
            .expect("large output should be drained without a timeout");
        let output = response.output().expect("noisy action should succeed");
        assert!(output.stdout_truncated);
        assert_eq!(output.stdout.len(), MAX_STREAM_OUTPUT_BYTES);
    }
}
