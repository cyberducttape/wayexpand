//! Command and hotkey result types shared by the expansion engine and hosts.
//!
//! Process execution remains in `command_runtime`; this module keeps the
//! command-facing data contracts separate from the engine state machine.

use crate::{CommandConfig, InjectorError, KeyChord};
use wayexpand_broker_client::ActionError;

use super::MAX_COMMAND_OUTPUT_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyResult {
    pub chord: KeyChord,
    pub description: String,
    pub command: CommandConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ExpansionError {
    #[error("injection failed: {0}")]
    Injection(#[from] InjectorError),
}

#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    #[error("could not start hotkey action")]
    Spawn(#[source] std::io::Error),
    #[error("hotkey action timed out after {0} ms")]
    Timeout(u64),
    #[error("hotkey action failed with status {0}")]
    Failed(String),
    #[error("hotkey action queue is full")]
    QueueFull,
    #[error("hotkey action worker is unavailable")]
    WorkerUnavailable,
}

/// Runtime counters for command-backed expansions and hotkey actions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandMetrics {
    pub command_queue_depth: usize,
    pub command_in_flight: usize,
    pub expansion_command_queue_depth: usize,
    pub expansion_command_in_flight: usize,
    pub hotkey_queue_depth: usize,
    pub hotkey_in_flight: usize,
    pub command_queue_rejected_total: u64,
    pub command_timeout_total: u64,
    pub command_failure_total: u64,
}

/// Why a command-backed expansion's configured program did not produce usable
/// output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    SpawnFailed,
    Timeout,
    WaitFailed(String),
    BrokerUnavailable {
        reason: String,
    },
    BrokerProtocol {
        operation: String,
        reason: String,
    },
    BrokerRejected(ActionError),
    NonZeroExit {
        code: Option<i32>,
        stderr: Option<String>,
    },
    OutputTooLarge,
    PolicyOutputTooLarge {
        size: usize,
        limit: usize,
    },
    StaleInput,
    WindowIdentityUnavailable,
    PolicyBlocked,
    QueueFull,
    WorkerUnavailable,
    InvalidUtf8,
    OutputChannelLost,
    IncompleteOutput,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed => write!(f, "could not start the program"),
            Self::Timeout => write!(f, "timed out before it produced output"),
            Self::WaitFailed(error) => write!(f, "failed while obtaining process status: {error}"),
            Self::BrokerUnavailable { reason } => {
                write!(f, "action broker unavailable: {reason}")
            }
            Self::BrokerProtocol { operation, reason } => {
                write!(f, "action broker {operation} failed: {reason}")
            }
            Self::BrokerRejected(error) => write!(f, "action broker rejected the action: {error}"),
            Self::NonZeroExit { code, stderr } => {
                match code {
                    Some(code) => write!(f, "exited with status {code}")?,
                    None => write!(f, "was terminated by a signal")?,
                }
                if let Some(stderr) = stderr {
                    write!(f, ": {stderr}")?;
                }
                Ok(())
            }
            Self::OutputTooLarge => write!(
                f,
                "produced more than {} bytes of output",
                MAX_COMMAND_OUTPUT_BYTES
            ),
            Self::PolicyOutputTooLarge { size, limit } => write!(
                f,
                "produced {size} bytes, exceeding the organization limit of {limit} bytes"
            ),
            Self::StaleInput => write!(f, "input changed before the expansion completed"),
            Self::WindowIdentityUnavailable => write!(
                f,
                "cannot open a snippet form because this backend does not provide an exact window identity"
            ),
            Self::PolicyBlocked => write!(f, "command execution is disabled by policy"),
            Self::QueueFull => write!(f, "command queue is full"),
            Self::WorkerUnavailable => write!(f, "command workers are unavailable"),
            Self::InvalidUtf8 => write!(f, "produced output that was not valid UTF-8"),
            Self::OutputChannelLost => write!(f, "output could not be read back"),
            Self::IncompleteOutput => write!(
                f,
                "output may be incomplete because the output stream did not close"
            ),
        }
    }
}

impl std::error::Error for CommandError {}
