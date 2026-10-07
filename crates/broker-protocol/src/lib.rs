//! Stable wire types shared by the WayExpand daemon and Action Broker.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const MAX_OUTPUT_BYTES: usize = 128 * 1024;
pub const MAX_ACTION_ID_BYTES: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionRequest {
    pub action_id: String,
    pub timeout_ms: u64,
    pub inherit_env: bool,
    pub env_vars: Vec<String>,
    pub stdout_capture: bool,
}

impl ActionRequest {
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    #[serde(default)]
    pub stdout_truncated: bool,
    #[serde(default)]
    pub stderr_truncated: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionResponse {
    Success(ActionOutput),
    Error(ActionError),
}

impl ActionResponse {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }
    pub fn output(&self) -> Option<&ActionOutput> {
        match self {
            Self::Success(output) => Some(output),
            Self::Error(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionError {
    ActionNotFound {
        action_id: String,
    },
    ActionBlocked {
        action_id: String,
        reason: String,
    },
    Timeout {
        action_id: String,
        timeout_ms: u64,
    },
    SpawnFailed {
        action_id: String,
        program: String,
        reason: String,
    },
    ExitFailure {
        action_id: String,
        exit_code: i32,
        stderr: String,
    },
    CommunicationError {
        reason: String,
    },
    OutputTruncated {
        limit_bytes: usize,
    },
    Internal {
        reason: String,
    },
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActionNotFound { action_id } => write!(f, "action '{action_id}' not found"),
            Self::ActionBlocked { action_id, reason } => {
                write!(f, "action '{action_id}' blocked: {reason}")
            }
            Self::Timeout {
                action_id,
                timeout_ms,
            } => write!(f, "action '{action_id}' timed out after {timeout_ms} ms"),
            Self::SpawnFailed {
                action_id,
                program,
                reason,
            } => write!(
                f,
                "action '{action_id}' (program: {program}) spawn failed: {reason}"
            ),
            Self::ExitFailure {
                action_id,
                exit_code,
                stderr,
            } => write!(
                f,
                "action '{action_id}' exited with code {exit_code}: {stderr}"
            ),
            Self::CommunicationError { reason } => write!(f, "communication error: {reason}"),
            Self::OutputTruncated { limit_bytes } => write!(
                f,
                "action output exceeded the {limit_bytes}-byte IPC response limit and was omitted"
            ),
            Self::Internal { reason } => write!(f, "internal broker error: {reason}"),
        }
    }
}

impl std::error::Error for ActionError {}
