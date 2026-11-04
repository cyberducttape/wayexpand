//! Action Executor - Safe command execution with policy enforcement.
//!
//! Executes actions within the constraints defined in ActionConfig and BrokerConfig.

#[cfg(not(unix))]
use crate::child_process::run_child_fallback;
use crate::child_process::ChildRunError;
#[cfg(test)]
use crate::child_process::MAX_STREAM_OUTPUT_BYTES;
#[cfg(unix)]
use crate::child_process::{run_child_unix_cancellable_with_cgroup, run_child_unix_with_cgroup};
use crate::{
    config::{BrokerConfig, MAX_ACTION_TIMEOUT_MS},
    protocol::{ActionError, ActionOutput, ActionRequest, ActionResponse},
};
use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
#[cfg(unix)]
use wayexpand_process_supervisor::{configure_process_group, ChildCgroup};

pub struct ActionExecutor {
    config: BrokerConfig,
    authorization_gate: AuthorizationGate,
}

const MAX_PENDING_AUTHORIZATIONS: usize = 4;
const MAX_AUTHORIZATION_FAILURES: usize = 256;
const AUTHORIZATION_COOLDOWN_BASE: Duration = Duration::from_secs(2);
const AUTHORIZATION_COOLDOWN_MAX: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CallerKey {
    uid: u32,
    pid: u32,
    start_time: u64,
}

#[derive(Default)]
struct AuthorizationGateState {
    pending: HashSet<CallerKey>,
    failures: HashMap<CallerKey, AuthorizationFailure>,
}

#[derive(Clone, Copy)]
struct AuthorizationFailure {
    count: u8,
    retry_at: Instant,
    forget_at: Instant,
}

#[derive(Default)]
struct AuthorizationGate {
    state: Mutex<AuthorizationGateState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthorizationAdmissionError {
    AlreadyPending,
    CoolingDown,
    Busy,
}

struct AuthorizationPermit<'a> {
    gate: &'a AuthorizationGate,
    caller: CallerKey,
    active: bool,
}

impl AuthorizationGate {
    fn begin(
        &self,
        uid: u32,
        pid: u32,
        start_time: u64,
    ) -> Result<AuthorizationPermit<'_>, AuthorizationAdmissionError> {
        let caller = CallerKey {
            uid,
            pid,
            start_time,
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        state.failures.retain(|_, failure| failure.forget_at > now);
        if state
            .failures
            .get(&caller)
            .is_some_and(|failure| failure.retry_at > now)
        {
            return Err(AuthorizationAdmissionError::CoolingDown);
        }
        if state.pending.contains(&caller) {
            return Err(AuthorizationAdmissionError::AlreadyPending);
        }
        if state.pending.len() >= MAX_PENDING_AUTHORIZATIONS {
            return Err(AuthorizationAdmissionError::Busy);
        }
        state.pending.insert(caller);
        Ok(AuthorizationPermit {
            gate: self,
            caller,
            active: true,
        })
    }

    fn complete(&self, caller: CallerKey, authorized: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending.remove(&caller);
        if authorized {
            state.failures.remove(&caller);
            return;
        }

        let previous = state.failures.get(&caller).copied();
        if state.failures.len() >= MAX_AUTHORIZATION_FAILURES && previous.is_none() {
            if let Some(oldest) = state
                .failures
                .iter()
                .min_by_key(|(_, failure)| failure.retry_at)
                .map(|(caller, _)| *caller)
            {
                state.failures.remove(&oldest);
            }
        }
        let count = previous.map_or(1, |failure| failure.count.saturating_add(1));
        let exponent = u32::from(count.saturating_sub(1).min(5));
        let cooldown = AUTHORIZATION_COOLDOWN_BASE
            .saturating_mul(1_u32 << exponent)
            .min(AUTHORIZATION_COOLDOWN_MAX);
        let now = Instant::now();
        state.failures.insert(
            caller,
            AuthorizationFailure {
                count,
                retry_at: now + cooldown,
                forget_at: now + Duration::from_secs(10 * 60),
            },
        );
    }
}

impl AuthorizationPermit<'_> {
    fn complete(mut self, authorized: bool) {
        self.gate.complete(self.caller, authorized);
        self.active = false;
    }
}

impl Drop for AuthorizationPermit<'_> {
    fn drop(&mut self) {
        if self.active {
            self.gate.complete(self.caller, false);
            self.active = false;
        }
    }
}

impl ActionExecutor {
    pub fn new(config: &BrokerConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = config.clone();
        config.validate_and_canonicalize()?;
        Ok(Self {
            config,
            authorization_gate: AuthorizationGate::default(),
        })
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
    #[cfg(test)]
    pub async fn execute(&self, request: ActionRequest) -> Result<ActionResponse, ActionError> {
        self.execute_inner(request, None).await
    }

    /// Execute an action that can be cancelled when the broker begins
    /// shutdown. The process-group supervisor kills and reaps the action
    /// before returning, allowing the broker to audit the cancellation.
    #[cfg(test)]
    pub async fn execute_cancellable(
        &self,
        request: ActionRequest,
        shutdown: Arc<AtomicBool>,
    ) -> Result<ActionResponse, ActionError> {
        self.execute_inner(request, Some(shutdown)).await
    }

    /// Authorize the authenticated socket peer for this specific action with
    /// the active desktop's polkit authority before entering the executor.
    /// Authorization is intentionally non-cached: each invocation requires
    /// fresh active-session authentication under the packaged policy.
    pub async fn execute_authorized_cancellable(
        &self,
        request: ActionRequest,
        peer: &crate::ipc::PeerIdentity,
        shutdown: Arc<AtomicBool>,
    ) -> Result<ActionResponse, ActionError> {
        let action_id = request.action_id.clone();
        if !valid_action_id(&action_id) {
            return Err(ActionError::ActionNotFound { action_id });
        }
        let Some(action) = self.config.get_action(&action_id) else {
            return Err(ActionError::ActionNotFound { action_id });
        };
        if !action.is_enabled() {
            return Err(ActionError::ActionBlocked {
                action_id,
                reason: "action is disabled".into(),
            });
        }
        if let Err(reason) = action.arguments_for(&request.parameters) {
            return Err(ActionError::ActionBlocked {
                action_id,
                reason: format!("action parameters rejected: {reason}"),
            });
        }
        if request.timeout_ms == 0 {
            return Err(ActionError::ActionBlocked {
                action_id,
                reason: "request timeout_ms must be > 0".into(),
            });
        }
        let Some(pid) = peer.pid else {
            return Err(authorization_denied(&action_id));
        };
        let Some(start_time) = peer.start_time else {
            return Err(authorization_denied(&action_id));
        };
        if peer.uid != rustix::process::geteuid().as_raw() {
            return Err(authorization_denied(&action_id));
        }
        let prompt = self
            .authorization_gate
            .begin(peer.uid, pid, start_time)
            .map_err(|reason| ActionError::ActionBlocked {
                action_id: action_id.clone(),
                reason: match reason {
                    AuthorizationAdmissionError::AlreadyPending => {
                        "an action authorization is already pending for this caller"
                    }
                    AuthorizationAdmissionError::CoolingDown => {
                        "authorization was recently denied; retry after the cooldown"
                    }
                    AuthorizationAdmissionError::Busy => {
                        "authorization service is busy; retry shortly"
                    }
                }
                .into(),
            })?;
        if shutdown.load(Ordering::Acquire) {
            return Err(ActionError::CommunicationError {
                reason: "broker is shutting down; action was not started".into(),
            });
        }
        let authorized =
            polkit_authorized(pid, start_time, peer.uid, &request.action_id, &shutdown).await;
        prompt.complete(authorized);
        if !authorized {
            return Err(authorization_denied(&request.action_id));
        }
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

        let arguments = action_config
            .arguments_for(&request.parameters)
            .map_err(|reason| ActionError::ActionBlocked {
                action_id: request.action_id.clone(),
                reason: format!("action parameters rejected: {reason}"),
            })?;

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
            let deny_network = action_config
                .sandbox
                .as_ref()
                .is_some_and(|sandbox| sandbox.deny_network);
            let deny_devices = action_config
                .sandbox
                .as_ref()
                .is_some_and(|sandbox| sandbox.deny_devices);
            crate::sandbox::configure_command_with_restrictions(
                &mut cmd,
                ruleset,
                deny_network,
                deny_devices,
            )
            .map_err(|reason| ActionError::ActionBlocked {
                action_id: request.action_id.clone(),
                reason: format!("could not configure action sandbox: {reason}"),
            })?;
        }

        for arg in &arguments {
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
        #[cfg(unix)]
        let cgroup = ChildCgroup::prepare(&mut cmd).map_err(|error| ActionError::SpawnFailed {
            action_id: request.action_id.clone(),
            program: action_config.program.clone(),
            reason: format!("could not establish command containment: {error}"),
        })?;

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
                    Some(shutdown) => run_child_unix_cancellable_with_cgroup(
                        child,
                        child_stdout,
                        child_stderr,
                        timeout,
                        Some(shutdown),
                        cgroup,
                    ),
                    None => run_child_unix_with_cgroup(
                        child,
                        child_stdout,
                        child_stderr,
                        timeout,
                        cgroup,
                    ),
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

pub(crate) fn valid_action_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= crate::protocol::MAX_ACTION_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn authorization_denied(action_id: &str) -> ActionError {
    ActionError::ActionBlocked {
        action_id: action_id.to_string(),
        reason: "caller was not authorized by the active desktop session".into(),
    }
}

#[cfg(target_os = "linux")]
async fn polkit_authorized(
    pid: u32,
    start_time: u64,
    uid: u32,
    action_id: &str,
    shutdown: &AtomicBool,
) -> bool {
    let helper = trusted_pkcheck_path();
    let Some(helper) = helper else { return false };
    run_pkcheck(&helper, pid, start_time, uid, action_id, shutdown).await
}

#[cfg(target_os = "linux")]
fn trusted_pkcheck_path() -> Option<std::path::PathBuf> {
    use std::os::unix::fs::MetadataExt;
    ["/usr/bin/pkcheck", "/bin/pkcheck"]
        .into_iter()
        .find_map(|path| {
            let (file, canonical) =
                crate::path_security::open_validated_program("polkit", path).ok()?;
            (file.metadata().ok()?.uid() == 0).then_some(canonical)
        })
}

#[cfg(target_os = "linux")]
async fn run_pkcheck(
    helper: &std::path::Path,
    pid: u32,
    start_time: u64,
    uid: u32,
    action_id: &str,
    shutdown: &AtomicBool,
) -> bool {
    use tokio::process::Command as TokioCommand;

    const POLKIT_ACTION: &str = "org.wayexpand.execute-action";
    const POLKIT_PROMPT_TIMEOUT: Duration = Duration::from_secs(30);
    let subject = format!("{pid},{start_time},{uid}");
    let mut child = match TokioCommand::new(helper)
        .arg("--action-id")
        .arg(POLKIT_ACTION)
        .arg("--process")
        .arg(subject)
        .arg("--allow-user-interaction")
        .arg("--detail")
        .arg("action_id")
        .arg(action_id)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let deadline = tokio::time::Instant::now() + POLKIT_PROMPT_TIMEOUT;
    loop {
        if shutdown.load(Ordering::Acquire) || tokio::time::Instant::now() >= deadline {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return false;
        }
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(_) => return false,
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn polkit_authorized(
    _pid: u32,
    _start_time: u64,
    _uid: u32,
    _action_id: &str,
    _shutdown: &AtomicBool,
) -> bool {
    false
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
    #[cfg(unix)]
    use crate::child_process::run_child_unix;
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
            parameters: std::collections::BTreeMap::new(),
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ActionError::ActionNotFound { .. })));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn parameter_values_are_validated_and_passed_as_single_argv_values() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "print-value".into(),
            ActionConfig {
                parameters: [(
                    "value".into(),
                    crate::config::ActionParameter {
                        arg_index: 1,
                        kind: "text".into(),
                        values: vec![],
                        max_length: Some(128),
                    },
                )]
                .into_iter()
                .collect(),
                sandbox: None,
                program: "/usr/bin/printf".into(),
                args: vec!["%s".into(), "placeholder".into()],
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
        let value = "hello; $(touch /tmp/wayexpand-param-must-not-execute)";
        let response = executor
            .execute(ActionRequest {
                action_id: "print-value".into(),
                timeout_ms: 1000,
                inherit_env: false,
                env_vars: vec![],
                stdout_capture: true,
                parameters: [("value".into(), value.into())].into_iter().collect(),
            })
            .await
            .unwrap();
        assert_eq!(response.output().unwrap().stdout, value);
        assert!(!std::path::Path::new("/tmp/wayexpand-param-must-not-execute").exists());

        let rejected = executor
            .execute(ActionRequest {
                action_id: "print-value".into(),
                timeout_ms: 1000,
                inherit_env: false,
                env_vars: vec![],
                stdout_capture: true,
                parameters: [("wrong".into(), "value".into())].into_iter().collect(),
            })
            .await;
        assert!(matches!(rejected, Err(ActionError::ActionBlocked { .. })));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn landlock_profile_blocks_reads_outside_its_allowlist() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "restricted-cat".to_string(),
            ActionConfig {
                parameters: std::collections::HashMap::new(),
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
                parameters: std::collections::BTreeMap::new(),
            })
            .await;

        assert!(
            matches!(&result, Err(ActionError::ExitFailure { stderr, .. }) if stderr.contains("Permission denied")),
            "unexpected sandbox result: {result:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn action_network_and_device_policies_are_applied_before_exec() {
        use std::{
            os::{unix::fs::PermissionsExt, unix::net::UnixListener},
            thread,
            time::{Duration, Instant},
        };

        let directory = std::env::temp_dir().join(format!(
            "wayexpand-network-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let executable = directory.join("network-probe");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let proxy_path = directory.join("local-proxy.sock");
        let proxy_listener = UnixListener::bind(&proxy_path).unwrap();
        proxy_listener.set_nonblocking(true).unwrap();
        let proxy_thread = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut connected = false;
            while Instant::now() < deadline {
                match proxy_listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = [0; 4];
                        std::io::Read::read_exact(&mut stream, &mut request).unwrap();
                        assert_eq!(&request, b"ping");
                        std::io::Write::write_all(&mut stream, b"pong").unwrap();
                        connected = true;
                        break;
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("local proxy accept failed: {error}"),
                }
            }
            assert!(
                connected,
                "sandboxed action did not connect to the local proxy"
            );
        });
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "network-probe".into(),
            ActionConfig {
                program: executable.to_string_lossy().into_owned(),
                args: vec![
                    "--exact".into(),
                    "sandbox::tests::action_executor_restriction_probe_child".into(),
                    "--ignored".into(),
                    "--nocapture".into(),
                ],
                parameters: std::collections::HashMap::new(),
                timeout_ms: 5000,
                server_env: vec![],
                client_forward_env: vec!["WAYEXPAND_TEST_PROXY_SOCKET".into()],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
                sandbox: Some(crate::config::ActionSandbox {
                    read_paths: vec![directory.to_string_lossy().into_owned()],
                    deny_network: true,
                    deny_devices: true,
                    ..Default::default()
                }),
            },
        );
        let executor = ActionExecutor::new(&config).unwrap();
        let response = executor
            .execute(ActionRequest {
                action_id: "network-probe".into(),
                timeout_ms: 5000,
                inherit_env: false,
                env_vars: vec![format!(
                    "WAYEXPAND_TEST_PROXY_SOCKET={}",
                    proxy_path.display()
                )],
                stdout_capture: true,
                parameters: std::collections::BTreeMap::new(),
            })
            .await;
        let response = response.unwrap();
        proxy_thread.join().unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        let output = response.output().expect("probe action should succeed");
        assert_eq!(output.exit_code, 0, "{}", output.stderr);
        assert!(
            output
                .stdout
                .contains("action_executor_restriction_probe_child ... ok"),
            "unexpected child test output: {}",
            output.stdout
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
                parameters: std::collections::HashMap::new(),
                sandbox: Some(crate::config::ActionSandbox {
                    read_paths: vec![],
                    write_paths: vec![directory.to_string_lossy().into_owned()],
                    deny_network: false,
                    deny_devices: false,
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
                parameters: std::collections::BTreeMap::new(),
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
                parameters: std::collections::HashMap::new(),
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
            parameters: std::collections::BTreeMap::new(),
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
                parameters: std::collections::HashMap::new(),
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
            parameters: std::collections::BTreeMap::new(),
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
                parameters: std::collections::HashMap::new(),
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
            parameters: std::collections::BTreeMap::new(),
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ActionError::Timeout { .. })));
        if let Err(ActionError::Timeout { timeout_ms, .. }) = &result {
            assert_eq!(*timeout_ms, 200);
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn executor_cleans_session_descendant_holding_output_pipe() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let pid_file = format!(
            "/tmp/wayexpand-broker-incomplete-output-{}-{nonce}.pid",
            std::process::id(),
        );
        let _ = std::fs::remove_file(&pid_file);
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "holds-output".to_string(),
            ActionConfig {
                parameters: std::collections::HashMap::new(),
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
            parameters: std::collections::BTreeMap::new(),
        };

        let result = executor.execute(request).await;
        let descendant_pid = std::fs::read_to_string(&pid_file).and_then(|text| {
            text.trim()
                .parse::<libc::pid_t>()
                .map_err(std::io::Error::other)
        });
        let cgroup_required = wayexpand_process_supervisor::containment_mode() == "cgroup";
        if cgroup_required {
            let pid = descendant_pid.expect("test command should report its detached child");
            let exited = |pid: libc::pid_t| {
                let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                    return true;
                };
                stat.rsplit_once(')')
                    .and_then(|(_, fields)| fields.split_whitespace().next())
                    .is_some_and(|state| state == "Z")
            };
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while !exited(pid) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                exited(pid),
                "setsid descendant {pid} survived broker cleanup"
            );
        } else if let Ok(pid) = descendant_pid {
            // Unmanaged compatibility mode has only process-group cleanup.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        let _ = std::fs::remove_file(pid_file);
        if cgroup_required {
            assert!(
                matches!(&result, Ok(ActionResponse::Success(output)) if output.stdout == "complete"),
                "cgroup cleanup should close the descendant pipe cleanly: {result:?}"
            );
        } else {
            assert!(
                matches!(
                    &result,
                    Err(ActionError::Internal { reason }) if reason.contains("output may be incomplete")
                ),
                "unexpected result: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn executor_client_cannot_exceed_config_timeout() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "echo".to_string(),
            ActionConfig {
                parameters: std::collections::HashMap::new(),
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
                parameters: std::collections::BTreeMap::new(),
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
                parameters: std::collections::HashMap::new(),
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
                    parameters: std::collections::BTreeMap::new(),
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
                parameters: std::collections::HashMap::new(),
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
                parameters: std::collections::BTreeMap::new(),
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
                parameters: std::collections::HashMap::new(),
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
            parameters: std::collections::BTreeMap::new(),
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn authorized_executor_refuses_peer_without_kernel_process_identity() {
        let executor = ActionExecutor::new(&timeout_test_config(1000)).unwrap();
        let peer = crate::ipc::PeerIdentity {
            pid: None,
            start_time: None,
            uid: rustix::process::geteuid().as_raw(),
            executable: None,
        };
        let error = executor
            .execute_authorized_cancellable(
                timeout_test_request(1000),
                &peer,
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ActionError::ActionBlocked { .. }));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn polkit_helper_success_is_required_and_denial_fails_closed() {
        let shutdown = AtomicBool::new(false);

        for (helper, expected) in [
            (std::path::Path::new("/usr/bin/true"), true),
            (std::path::Path::new("/usr/bin/false"), false),
        ] {
            assert_eq!(
                run_pkcheck(helper, 123, 456, 789, "cluster.read", &shutdown).await,
                expected
            );
        }
    }

    #[tokio::test]
    async fn unanswered_polkit_prompt_does_not_block_another_caller() {
        let gate = Arc::new(AuthorizationGate::default());
        let first_gate = Arc::clone(&gate);
        let (started_send, started_receive) = tokio::sync::oneshot::channel();
        let (finish_send, finish_receive) = tokio::sync::oneshot::channel();
        let first = tokio::spawn(async move {
            let permit = first_gate.begin(1000, 101, 5001).unwrap();
            started_send.send(()).unwrap();
            // Simulate a polkit dialog that the first caller leaves unanswered.
            let allowed = finish_receive.await.unwrap_or(false);
            permit.complete(allowed);
        });
        started_receive.await.unwrap();

        let second_gate = Arc::clone(&gate);
        let legitimate = tokio::time::timeout(Duration::from_millis(100), async move {
            let permit = second_gate.begin(1000, 202, 6002).unwrap();
            permit.complete(true);
            true
        })
        .await
        .expect("an unrelated caller should obtain an authorization slot");
        assert!(legitimate);

        finish_send.send(false).unwrap();
        first.await.unwrap();
        assert!(matches!(
            gate.begin(1000, 101, 5001),
            Err(AuthorizationAdmissionError::CoolingDown)
        ));
        // A denial cooldown is scoped to that exact process identity.
        gate.begin(1000, 202, 6002).unwrap().complete(true);
    }

    #[test]
    fn duplicate_prompts_for_one_peer_are_rejected_without_global_locking() {
        let gate = AuthorizationGate::default();
        let first = gate.begin(1000, 101, 5001).unwrap();
        assert!(matches!(
            gate.begin(1000, 101, 5001),
            Err(AuthorizationAdmissionError::AlreadyPending)
        ));
        gate.begin(1000, 202, 6002).unwrap().complete(true);
        drop(first);
    }

    #[test]
    fn authorization_gate_bounds_distinct_pending_prompts() {
        let gate = AuthorizationGate::default();
        let permits = (0..MAX_PENDING_AUTHORIZATIONS)
            .map(|index| {
                gate.begin(1000, 100 + index as u32, 5000 + index as u64)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            gate.begin(1000, 999, 9000),
            Err(AuthorizationAdmissionError::Busy)
        ));

        permits.into_iter().next().unwrap().complete(true);
        gate.begin(1000, 999, 9000).unwrap().complete(true);
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
                parameters: std::collections::HashMap::new(),
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
                parameters: std::collections::BTreeMap::new(),
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
                parameters: std::collections::HashMap::new(),
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
                parameters: std::collections::BTreeMap::new(),
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
                parameters: std::collections::HashMap::new(),
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
                parameters: std::collections::BTreeMap::new(),
            })
            .await
            .expect("large output should be drained without a timeout");
        let output = response.output().expect("noisy action should succeed");
        assert!(output.stdout_truncated);
        assert_eq!(output.stdout.len(), MAX_STREAM_OUTPUT_BYTES);
    }
}
