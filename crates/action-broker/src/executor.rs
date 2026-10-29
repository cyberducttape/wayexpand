//! Action Executor - Safe command execution with policy enforcement.
//!
//! Executes actions within the constraints defined in ActionConfig and BrokerConfig.

#[cfg(not(unix))]
use crate::child_process::run_child_fallback;
use crate::child_process::ChildRunError;
#[cfg(test)]
use crate::child_process::MAX_STREAM_OUTPUT_BYTES;
#[cfg(unix)]
use crate::child_process::{run_child_unix, run_child_unix_cancellable};
use crate::{
    config::{BrokerConfig, MAX_ACTION_TIMEOUT_MS},
    protocol::{ActionError, ActionOutput, ActionRequest, ActionResponse},
};
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
#[cfg(unix)]
use wayexpand_process_supervisor::configure_process_group;

pub struct ActionExecutor {
    config: BrokerConfig,
}

impl ActionExecutor {
    pub fn new(config: &BrokerConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = config.clone();
        config.validate_and_canonicalize()?;
        Ok(Self { config })
    }

    /// Resolve the effective timeout: the broker config is the upper bound,
    /// and MAX_ACTION_TIMEOUT_MS bounds both in case validation was bypassed.
    fn effective_timeout(&self, request: &ActionRequest, action_timeout_ms: u64) -> Duration {
        let effective = action_timeout_ms
            .min(request.timeout_ms)
            .min(MAX_ACTION_TIMEOUT_MS);
        Duration::from_millis(effective)
    }

    /// Execute an action with process-group containment and policy enforcement.
    pub async fn execute(&self, request: ActionRequest) -> Result<ActionResponse, ActionError> {
        self.execute_inner(request, None).await
    }

    /// Execute an action that can be cancelled when the broker begins
    /// shutdown. The process-group supervisor kills and reaps the action
    /// before returning, allowing the broker to audit the cancellation.
    pub async fn execute_cancellable(
        &self,
        request: ActionRequest,
        shutdown: Arc<AtomicBool>,
    ) -> Result<ActionResponse, ActionError> {
        self.execute_inner(request, Some(shutdown)).await
    }

    async fn execute_inner(
        &self,
        request: ActionRequest,
        shutdown: Option<Arc<AtomicBool>>,
    ) -> Result<ActionResponse, ActionError> {
        if shutdown
            .as_ref()
            .is_some_and(|token| token.load(Ordering::Acquire))
        {
            return Err(ActionError::CommunicationError {
                reason: "broker is shutting down; action was not started".to_string(),
            });
        }
        let action_config = self.config.get_action(&request.action_id).ok_or_else(|| {
            ActionError::ActionNotFound {
                action_id: request.action_id.clone(),
            }
        })?;

        if !action_config.is_enabled() {
            return Err(ActionError::ActionBlocked {
                action_id: request.action_id.clone(),
                reason: "action is disabled".to_string(),
            });
        }

        if request.timeout_ms == 0 {
            return Err(ActionError::ActionBlocked {
                action_id: request.action_id.clone(),
                reason: "request timeout_ms must be > 0".to_string(),
            });
        }

        let pinned_program = if self.config.require_absolute_paths {
            match crate::path_security::open_validated_program(
                &request.action_id,
                &action_config.program,
            ) {
                Ok((file, resolved))
                    if resolved.as_os_str() == std::ffi::OsStr::new(&action_config.program) =>
                {
                    Some(file)
                }
                Ok(_) => {
                    return Err(ActionError::ActionBlocked {
                        action_id: request.action_id.clone(),
                        reason: "program failed revalidation before launch: path no longer resolves to the validated file".to_string(),
                    });
                }
                Err(reason) => {
                    return Err(ActionError::ActionBlocked {
                        action_id: request.action_id.clone(),
                        reason: format!("program failed revalidation before launch: {reason}"),
                    });
                }
            }
        } else {
            None
        };
        let pinned_path = pinned_program
            .as_ref()
            .map(|file| format!("/proc/self/fd/{}", file.as_raw_fd()));
        let mut cmd = Command::new(
            pinned_path
                .as_deref()
                .unwrap_or(action_config.program.as_str()),
        );
        #[cfg(target_os = "linux")]
        if let Some(file) = pinned_program.as_ref() {
            configure_pinned_executable(&mut cmd, file, &action_config.program);
        }

        #[cfg(target_os = "linux")]
        let sandbox_ruleset = if let Some(sandbox) = &action_config.sandbox {
            let cwd = action_config
                .cwd
                .as_deref()
                .or(self.config.default_cwd.as_deref());
            Some(
                crate::sandbox::prepare(sandbox, &action_config.program, cwd).map_err(
                    |reason| ActionError::ActionBlocked {
                        action_id: request.action_id.clone(),
                        reason: format!("could not enforce action sandbox: {reason}"),
                    },
                )?,
            )
        } else {
            None
        };
        #[cfg(not(target_os = "linux"))]
        if action_config.sandbox.is_some() {
            return Err(ActionError::ActionBlocked {
                action_id: request.action_id.clone(),
                reason: "per-action filesystem sandboxing requires Linux Landlock".to_string(),
            });
        }
        #[cfg(target_os = "linux")]
        if let Some(ruleset) = &sandbox_ruleset {
            crate::sandbox::configure_command(&mut cmd, ruleset);
        }

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
        } else if action_config.log_output {
            // An administrator explicitly opted this action into logging: the
            // broker's stdout/stderr are normally collected by its service
            // manager (for example journald).
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        } else {
            // Output nobody asked for is discarded rather than persisted to a
            // journal, where tokens or infrastructure data would outlive the
            // action. It is never buffered in the broker either.
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }

        #[cfg(unix)]
        configure_process_group(&mut cmd);

        if shutdown
            .as_ref()
            .is_some_and(|token| token.load(Ordering::Acquire))
        {
            return Err(ActionError::CommunicationError {
                reason: "broker is shutting down; action was not started".to_string(),
            });
        }

        let start = Instant::now();

        let mut child = spawn_action(&mut cmd).map_err(|e| ActionError::SpawnFailed {
            action_id: request.action_id.clone(),
            program: action_config.program.clone(),
            reason: e.to_string(),
        })?;

        let timeout = self.effective_timeout(&request, action_config.timeout_ms);
        let action_id = request.action_id.clone();

        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();
        #[cfg(not(unix))]
        let _shutdown = shutdown.clone();
        // The blocking task owns the child for its entire lifetime. Its own
        // deadline kills and reaps the process group, so dropping/cancelling
        // this future cannot orphan an action behind Tokio's scheduler.
        let result = tokio::task::spawn_blocking(move || {
            #[cfg(unix)]
            {
                match shutdown {
                    Some(shutdown) => run_child_unix_cancellable(
                        child,
                        child_stdout,
                        child_stderr,
                        timeout,
                        Some(shutdown),
                    ),
                    None => run_child_unix(child, child_stdout, child_stderr, timeout),
                }
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
            Err(ChildRunError::Cancelled) => Err(ActionError::CommunicationError {
                reason: "action was cancelled because the broker is shutting down".to_string(),
            }),
            Err(ChildRunError::InvalidTimeout) => Err(ActionError::Internal {
                reason: "action timeout cannot be represented as a deadline".to_string(),
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

/// Some filesystems briefly return ETXTBSY when an executable is atomically
/// replaced while a pinned descriptor is being launched. `Command::spawn`
/// reports this before a child starts, so a short bounded retry cannot
/// duplicate an action. Other spawn errors are returned immediately.
#[cfg(unix)]
fn spawn_action(command: &mut Command) -> std::io::Result<Child> {
    retry_text_busy(|| command.spawn())
}

#[cfg(not(unix))]
fn spawn_action(command: &mut Command) -> std::io::Result<Child> {
    command.spawn()
}

#[cfg(unix)]
fn retry_text_busy<T>(mut spawn: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    const RETRIES: usize = 3;
    for attempt in 0..=RETRIES {
        match spawn() {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && attempt < RETRIES => {
                std::thread::sleep(Duration::from_millis(10 << attempt));
            }
            result => return result,
        }
    }
    unreachable!("the final spawn attempt returns")
}

#[cfg(target_os = "linux")]
fn configure_pinned_executable(cmd: &mut Command, file: &std::fs::File, original: &str) {
    use std::os::unix::process::CommandExt;

    cmd.arg0(original);
    let fd = file.as_raw_fd();
    // The descriptor must survive execve's path lookup through /proc/self/fd.
    // It remains open for shebang interpreters too.
    unsafe {
        cmd.pre_exec(move || {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ActionConfig;

    #[cfg(unix)]
    #[test]
    fn transient_text_busy_spawn_is_retried_but_other_errors_are_not() {
        use std::io;

        let mut attempts = 0;
        let result = retry_text_busy(|| {
            attempts += 1;
            if attempts < 3 {
                Err(io::Error::from_raw_os_error(libc::ETXTBSY))
            } else {
                Ok(42)
            }
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts, 3);

        let mut attempts = 0;
        let result: io::Result<()> = retry_text_busy(|| {
            attempts += 1;
            Err(io::Error::from_raw_os_error(libc::ENOENT))
        });
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ENOENT));
        assert_eq!(attempts, 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pinned_shebang_launch_uses_the_validated_inode_after_path_replacement() {
        use std::os::unix::{fs::PermissionsExt, io::AsRawFd};

        let directory = std::env::temp_dir().join(format!(
            "wayexpand-broker-pinned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let program = directory.join("action.sh");
        std::fs::write(&program, "#!/bin/sh\nprintf 'validated-inode\\n'\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();

        let (file, resolved) =
            crate::path_security::open_validated_program("test", program.to_str().unwrap())
                .unwrap();
        assert_eq!(resolved, program);

        let replacement = directory.join("replacement.sh");
        std::fs::write(&replacement, "#!/bin/sh\nprintf 'replacement-inode\\n'\n").unwrap();
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::rename(&replacement, &program).unwrap();

        let mut command = Command::new(format!("/proc/self/fd/{}", file.as_raw_fd()));
        configure_pinned_executable(&mut command, &file, program.to_str().unwrap());
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = spawn_action(&mut command)
            .unwrap()
            .wait_with_output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "validated-inode\n"
        );
        drop(file);
        let _ = std::fs::remove_dir_all(directory);
    }

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

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn landlock_profile_blocks_reads_outside_its_allowlist() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "restricted-cat".to_string(),
            ActionConfig {
                sandbox: Some(crate::config::ActionSandbox::default()),
                program: "/usr/bin/cat".to_string(),
                args: vec!["/etc/passwd".to_string()],
                timeout_ms: 5000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
            },
        );

        let executor = ActionExecutor::new(&config).unwrap();
        let result = executor
            .execute(ActionRequest {
                action_id: "restricted-cat".to_string(),
                timeout_ms: 5000,
                inherit_env: false,
                env_vars: vec![],
                stdout_capture: true,
            })
            .await;

        assert!(
            matches!(&result, Err(ActionError::ExitFailure { stderr, .. }) if stderr.contains("Permission denied")),
            "unexpected sandbox result: {result:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn landlock_profile_allows_writes_only_under_granted_directories() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "wayexpand-landlock-write-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = directory.join("allowed.txt");
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "restricted-touch".to_string(),
            ActionConfig {
                sandbox: Some(crate::config::ActionSandbox {
                    read_paths: vec![],
                    write_paths: vec![directory.to_string_lossy().into_owned()],
                }),
                program: "/usr/bin/touch".to_string(),
                args: vec![output.to_string_lossy().into_owned()],
                timeout_ms: 5000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
            },
        );

        let executor = ActionExecutor::new(&config).unwrap();
        let result = executor
            .execute(ActionRequest {
                action_id: "restricted-touch".to_string(),
                timeout_ms: 5000,
                inherit_env: false,
                env_vars: vec![],
                stdout_capture: true,
            })
            .await;
        assert!(
            matches!(result, Ok(ActionResponse::Success(_))),
            "{result:?}"
        );
        assert!(output.is_file());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn executor_revalidates_the_program_immediately_before_launch() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "wayexpand-broker-revalidate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let program = directory.join("action.sh");
        std::fs::write(&program, "#!/bin/sh\necho ok\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();

        let mut config = BrokerConfig::default();
        config.actions.insert(
            "action".to_string(),
            ActionConfig {
                sandbox: None,
                program: program.to_string_lossy().into_owned(),
                args: vec![],
                timeout_ms: 5000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
            },
        );
        let executor = ActionExecutor::new(&config).unwrap();
        let request = || ActionRequest {
            action_id: "action".to_string(),
            timeout_ms: 5000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };
        assert!(executor.execute(request()).await.is_ok());

        // Made group-writable after the policy loaded: refused at launch.
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o770)).unwrap();
        let result = executor.execute(request()).await;
        assert!(
            matches!(&result, Err(ActionError::ActionBlocked { reason, .. }) if reason.contains("revalidation")),
            "{result:?}"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn executor_disabled_action() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "disabled".to_string(),
            ActionConfig {
                sandbox: None,
                program: "/bin/echo".to_string(),
                args: vec![],
                timeout_ms: 5000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
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
                sandbox: None,
                program: "/bin/sleep".to_string(),
                args: vec!["60".to_string()],
                timeout_ms: 200,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
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
                sandbox: None,
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
                log_output: false,
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
                sandbox: None,
                program: "/bin/echo".to_string(),
                args: vec!["hi".to_string()],
                timeout_ms: 500,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn executor_cancels_a_running_action_for_bounded_broker_shutdown() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "stream".to_string(),
            ActionConfig {
                sandbox: None,
                program: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "while :; do printf x; done".to_string()],
                timeout_ms: 30_000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
            },
        );
        let executor = ActionExecutor::new(&config).expect("valid stream action");
        let shutdown = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&shutdown);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            signal.store(true, Ordering::Release);
        });
        let started = Instant::now();

        let result = executor
            .execute_cancellable(
                ActionRequest {
                    action_id: "stream".to_string(),
                    timeout_ms: 30_000,
                    inherit_env: false,
                    env_vars: vec![],
                    stdout_capture: true,
                },
                shutdown,
            )
            .await;

        assert!(matches!(
            result,
            Err(ActionError::CommunicationError { ref reason })
                if reason.contains("cancelled because the broker is shutting down")
        ));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "action outlived the bounded shutdown cancellation: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn uncaptured_output_is_discarded_unless_policy_logs_it() {
        let target_file = format!(
            "/tmp/wayexpand-broker-uncaptured-output-{}.txt",
            std::process::id()
        );
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "report-stdout".to_string(),
            ActionConfig {
                sandbox: None,
                program: "/bin/sh".to_string(),
                args: vec![
                    "-c".to_string(),
                    // Command substitutions keep the shell itself alive, so
                    // $$ names the process holding the action's descriptors.
                    format!(
                        "printf '%s\\n%s\\n' \"$(/usr/bin/readlink /proc/$$/fd/1)\" \"$(/usr/bin/readlink /proc/$$/fd/2)\" > {target_file}"
                    ),
                ],
                timeout_ms: 5_000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
            },
        );
        let executor = ActionExecutor::new(&config).unwrap();
        let result = executor
            .execute(ActionRequest {
                action_id: "report-stdout".to_string(),
                timeout_ms: 5_000,
                inherit_env: false,
                env_vars: vec![],
                stdout_capture: false,
            })
            .await;
        let targets = std::fs::read_to_string(&target_file).unwrap_or_default();
        let _ = std::fs::remove_file(&target_file);
        assert!(result.is_ok(), "unexpected result: {result:?}");
        assert_eq!(targets, "/dev/null\n/dev/null\n");
    }

    fn timeout_test_config(timeout_ms: u64) -> BrokerConfig {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "true".to_string(),
            ActionConfig {
                sandbox: None,
                program: "/bin/true".to_string(),
                args: vec![],
                timeout_ms,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
            },
        );
        config
    }

    fn timeout_test_request(timeout_ms: u64) -> ActionRequest {
        ActionRequest {
            action_id: "true".to_string(),
            timeout_ms,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        }
    }

    #[test]
    fn executor_rejects_policy_timeout_above_maximum() {
        assert!(ActionExecutor::new(&timeout_test_config(u64::MAX)).is_err());
        assert!(ActionExecutor::new(&timeout_test_config(MAX_ACTION_TIMEOUT_MS + 1)).is_err());
        assert!(ActionExecutor::new(&timeout_test_config(MAX_ACTION_TIMEOUT_MS)).is_ok());
    }

    #[tokio::test]
    async fn executor_runs_with_maximum_policy_and_u64_max_request_timeout() {
        let executor = ActionExecutor::new(&timeout_test_config(MAX_ACTION_TIMEOUT_MS)).unwrap();
        assert_eq!(
            executor.effective_timeout(&timeout_test_request(u64::MAX), MAX_ACTION_TIMEOUT_MS),
            Duration::from_millis(MAX_ACTION_TIMEOUT_MS)
        );
        let result = executor.execute(timeout_test_request(u64::MAX)).await;
        assert!(result.is_ok(), "unexpected result: {result:?}");
    }

    #[test]
    fn executor_effective_timeout_is_bounded_even_if_validation_is_bypassed() {
        let executor = ActionExecutor::new(&BrokerConfig::default()).unwrap();
        assert_eq!(
            executor.effective_timeout(&timeout_test_request(u64::MAX), u64::MAX),
            Duration::from_millis(MAX_ACTION_TIMEOUT_MS)
        );
    }

    #[tokio::test]
    async fn executor_rejects_zero_request_timeout_before_spawning() {
        let executor = ActionExecutor::new(&timeout_test_config(1_000)).unwrap();
        let result = executor.execute(timeout_test_request(0)).await;
        assert!(
            matches!(&result, Err(ActionError::ActionBlocked { reason, .. }) if reason.contains("timeout_ms")),
            "unexpected result: {result:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_child_unrepresentable_deadline_errors_without_panicking() {
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut cmd);
        let child = cmd.spawn().expect("spawn sleep");
        let result = run_child_unix(child, None, None, Duration::MAX);
        assert!(matches!(result, Err(ChildRunError::InvalidTimeout)));
    }

    #[tokio::test]
    async fn executor_empty_allowlist_does_not_inherit_client_environment() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "print-env".to_string(),
            ActionConfig {
                sandbox: None,
                program: "/usr/bin/env".to_string(),
                args: vec![],
                timeout_ms: 1000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
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
                sandbox: None,
                program: "/usr/bin/env".to_string(),
                args: vec![],
                timeout_ms: 1000,
                server_env: vec!["WAYEXPAND_BROKER_TEST_VALUE".to_string()],
                client_forward_env: vec!["CLIENT_VALUE".to_string()],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
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
                sandbox: None,
                program: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "yes x | head -c 20971520".to_string()],
                timeout_ms: 2000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
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
