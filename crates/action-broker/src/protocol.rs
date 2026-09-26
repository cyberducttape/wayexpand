//! Action Broker Protocol - Message types for IPC communication.
//!
//! Defines the request/response format for daemon-to-broker communication.
//! Uses JSON serialization for platform-independence and debuggability.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Action execution request from daemon to broker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionRequest {
    /// Unique action identifier (must match configured action in broker policy).
    pub action_id: String,

    /// Maximum execution time in milliseconds.
    pub timeout_ms: u64,

    /// Whether to inherit environment variables from the daemon.
    /// If false, only explicitly allowed variables are passed.
    pub inherit_env: bool,

    /// Environment variables to pass to the action.
    /// Format: ["KEY=value", ...]. Only used if inherit_env is false.
    pub env_vars: Vec<String>,

    /// Whether to capture and return stdout/stderr from the action.
    /// If false, output is streamed to syslog/journald only.
    pub stdout_capture: bool,
}

impl ActionRequest {
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

/// Successful action execution response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionOutput {
    /// Exit code from the action process.
    pub exit_code: i32,

    /// Captured stdout (if requested).
    pub stdout: String,

    /// Captured stderr (if requested).
    pub stderr: String,

    /// Total execution time in milliseconds.
    pub duration_ms: u64,
}

/// Action execution result from broker to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionResponse {
    /// Action executed successfully.
    Success(ActionOutput),

    /// Action failed with an error.
    Error(ActionError),
}

impl ActionResponse {
    pub fn is_success(&self) -> bool {
        matches!(self, ActionResponse::Success(_))
    }

    pub fn output(&self) -> Option<&ActionOutput> {
        match self {
            ActionResponse::Success(output) => Some(output),
            ActionResponse::Error(_) => None,
        }
    }
}

/// Errors that can occur during action execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionError {
    /// Action not found in broker configuration.
    ActionNotFound { action_id: String },

    /// Action is disabled or blocked by policy.
    ActionBlocked { action_id: String, reason: String },

    /// Execution timed out.
    Timeout { action_id: String, timeout_ms: u64 },

    /// Failed to spawn the process.
    SpawnFailed {
        action_id: String,
        program: String,
        reason: String,
    },

    /// Process exited with non-zero status.
    ExitFailure {
        action_id: String,
        exit_code: i32,
        stderr: String,
    },

    /// Communication error (IPC failure).
    CommunicationError { reason: String },

    /// Internal broker error.
    Internal { reason: String },
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionError::ActionNotFound { action_id } => {
                write!(f, "action '{}' not found", action_id)
            }
            ActionError::ActionBlocked { action_id, reason } => {
                write!(f, "action '{}' blocked: {}", action_id, reason)
            }
            ActionError::Timeout {
                action_id,
                timeout_ms,
            } => {
                write!(
                    f,
                    "action '{}' timed out after {} ms",
                    action_id, timeout_ms
                )
            }
            ActionError::SpawnFailed {
                action_id,
                program,
                reason,
            } => {
                write!(
                    f,
                    "action '{}' (program: {}) spawn failed: {}",
                    action_id, program, reason
                )
            }
            ActionError::ExitFailure {
                action_id,
                exit_code,
                stderr,
            } => {
                write!(
                    f,
                    "action '{}' exited with code {}: {}",
                    action_id, exit_code, stderr
                )
            }
            ActionError::CommunicationError { reason } => {
                write!(f, "communication error: {}", reason)
            }
            ActionError::Internal { reason } => {
                write!(f, "internal broker error: {}", reason)
            }
        }
    }
}

impl std::error::Error for ActionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_request_with_timeout() {
        let req = ActionRequest {
            action_id: "kubectl_get_pods".to_string(),
            timeout_ms: 5000,
            inherit_env: false,
            env_vars: vec!["KUBECONFIG=/home/user/.kube/config".to_string()],
            stdout_capture: true,
        };
        assert_eq!(req.timeout(), Duration::from_millis(5000));
    }

    #[test]
    fn action_output_serialization() {
        let output = ActionOutput {
            exit_code: 0,
            stdout: "test output".to_string(),
            stderr: String::new(),
            duration_ms: 100,
        };
        let json = serde_json::to_string(&output).unwrap();
        let deserialized: ActionOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exit_code, 0);
        assert_eq!(deserialized.stdout, "test output");
    }

    #[test]
    fn action_error_display() {
        let error = ActionError::ActionNotFound {
            action_id: "nonexistent".to_string(),
        };
        assert_eq!(error.to_string(), "action 'nonexistent' not found");
    }
}
