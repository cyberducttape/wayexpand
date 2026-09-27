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
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024; // 1 MiB per stream

pub struct ActionExecutor {
    config: BrokerConfig,
}

/// RAII guard that kills the child's process group on drop.
#[cfg(unix)]
struct ChildGuard {
    child: Option<Child>,
    pid: Option<u32>,
}

#[cfg(unix)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            kill_process_group(pid);
        }
        if let Some(ref mut child) = self.child {
            let _ = child.wait();
        }
    }
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

/// Read up to MAX_OUTPUT_BYTES from a stream, silently truncating beyond that.
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

        // Environment policy is server-authoritative: action_config.inherit_env
        // is the policy decision; request.inherit_env is ignored. strict_env on
        // the broker config overrides per-action inherit_env as a global deny.
        if action_config.inherit_env && !self.config.strict_env {
            // Inherit only the variables named in pass_env.  An empty pass_env
            // with inherit_env means "inherit nothing" — the config author must
            // explicitly list every variable they want forwarded.
            for key in &action_config.pass_env {
                if let Ok(value) = std::env::var(key) {
                    cmd.env(key, value);
                }
            }
        } else {
            // Restricted mode: accept only client-supplied variables that appear
            // in the action's pass_env allowlist.  An empty allowlist means
            // nothing is permitted — not everything.
            let allowed_vars = self.build_env_map(&request.env_vars, &action_config.pass_env);
            for (key, value) in allowed_vars {
                cmd.env(&key, value);
            }
        }

        let capture_stdout = request.stdout_capture;
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

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

        #[cfg(unix)]
        let pid = child.id();

        // Take ownership of stdout/stderr for bounded reading.
        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();

        #[cfg(unix)]
        let mut guard = ChildGuard {
            child: Some(child),
            pid: Some(pid),
        };

        #[cfg(unix)]
        let result = {
            let mut child_ref = guard.child.take().unwrap();
            tokio::time::timeout(timeout, async move {
                tokio::task::spawn_blocking(move || {
                    let stdout_bytes = bounded_read_stream(child_stdout);
                    let stderr_bytes = bounded_read_stream(child_stderr);
                    let status = child_ref.wait()?;
                    Ok::<_, std::io::Error>((status, stdout_bytes, stderr_bytes))
                })
                .await
                .map_err(|e| ActionError::Internal {
                    reason: format!("task join error: {}", e),
                })
                .and_then(|res| {
                    res.map_err(|e| ActionError::Internal {
                        reason: format!("child process error: {}", e),
                    })
                })
            })
            .await
        };

        #[cfg(not(unix))]
        let result = {
            tokio::time::timeout(timeout, async move {
                tokio::task::spawn_blocking(move || {
                    let stdout_bytes = bounded_read_stream(child_stdout);
                    let stderr_bytes = bounded_read_stream(child_stderr);
                    let status = child.wait()?;
                    Ok::<_, std::io::Error>((status, stdout_bytes, stderr_bytes))
                })
                .await
                .map_err(|e| ActionError::Internal {
                    reason: format!("task join error: {}", e),
                })
                .and_then(|res| {
                    res.map_err(|e| ActionError::Internal {
                        reason: format!("child process error: {}", e),
                    })
                })
            })
            .await
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok((status, stdout_bytes, stderr_bytes))) => {
                #[cfg(unix)]
                {
                    kill_process_group(pid);
                    guard.pid = None;
                }

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
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Timeout: the guard's Drop kills the process group and reaps.
                Err(ActionError::Timeout {
                    action_id,
                    timeout_ms: timeout.as_millis() as u64,
                })
            }
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
                allow_network: false,
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
                allow_network: false,
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
                allow_network: false,
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
}
