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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessWaitOperation {
    Observe,
    Reap,
    TryWait,
}

impl std::fmt::Display for ProcessWaitOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Observe => "observing process status",
            Self::Reap => "reaping process",
            Self::TryWait => "checking process status",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessWaitFailure {
    Io { detail: String },
}

impl std::fmt::Display for ProcessWaitFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { detail } => write!(f, "I/O error: {detail}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerUnavailableReason {
    SocketNotConfigured,
    Connect { detail: String },
}

impl std::fmt::Display for BrokerUnavailableReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SocketNotConfigured => f.write_str("socket is not configured"),
            Self::Connect { detail } => write!(f, "{detail}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerOperation {
    SetIoTimeout,
    SendRequest,
    ReceiveResponse,
}

impl std::fmt::Display for BrokerOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SetIoTimeout => "setting broker I/O deadline",
            Self::SendRequest => "sending broker request",
            Self::ReceiveResponse => "receiving broker response",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerProtocolFailure {
    ConnectionClosed,
    MessageTooLarge { size: usize, limit: usize },
    InvalidJson { detail: String },
    Io { detail: String },
}

impl std::fmt::Display for BrokerProtocolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionClosed => f.write_str("connection closed"),
            Self::MessageTooLarge { size, limit } => {
                write!(
                    f,
                    "message is {size} bytes, exceeding the {limit}-byte limit"
                )
            }
            Self::InvalidJson { detail } => write!(f, "invalid JSON: {detail}"),
            Self::Io { detail } => write!(f, "I/O error: {detail}"),
        }
    }
}

/// Why a command-backed expansion's configured program did not produce usable
/// output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    SpawnFailed {
        detail: String,
    },
    Timeout,
    WaitFailed {
        operation: ProcessWaitOperation,
        reason: ProcessWaitFailure,
    },
    BrokerUnavailable {
        reason: BrokerUnavailableReason,
    },
    BrokerProtocol {
        operation: BrokerOperation,
        reason: BrokerProtocolFailure,
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
    /// The output route cannot verify that the trigger is still at the cursor,
    /// so a form completed later could erase text somewhere else.
    FormTargetUnverifiable,
    PolicyBlocked,
    QueueFull,
    WorkerUnavailable,
    InvalidUtf8,
    OutputChannelLost,
    IncompleteOutput,
}

/// Stable high-level classification for UI and diagnostics.
///
/// The full [`CommandError`] retains operation details, broker reasons, exit
/// status, and bounded stderr. Consumers that only need remediation guidance
/// can use this projection instead of parsing display strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandErrorKind {
    Spawn,
    Timeout,
    ProcessWait,
    BrokerUnavailable,
    BrokerProtocol,
    BrokerRejected,
    NonZeroExit,
    OutputTooLarge,
    Policy,
    StaleInput,
    WindowIdentity,
    QueueFull,
    WorkerUnavailable,
    InvalidOutput,
    OutputChannel,
    IncompleteOutput,
}

impl CommandError {
    /// Return the stable category of this failure without discarding its
    /// detailed context or requiring callers to inspect display text.
    pub fn kind(&self) -> CommandErrorKind {
        match self {
            Self::SpawnFailed { .. } => CommandErrorKind::Spawn,
            Self::Timeout => CommandErrorKind::Timeout,
            Self::WaitFailed { .. } => CommandErrorKind::ProcessWait,
            Self::BrokerUnavailable { .. } => CommandErrorKind::BrokerUnavailable,
            Self::BrokerProtocol { .. } => CommandErrorKind::BrokerProtocol,
            Self::BrokerRejected(_) => CommandErrorKind::BrokerRejected,
            Self::NonZeroExit { .. } => CommandErrorKind::NonZeroExit,
            Self::OutputTooLarge | Self::PolicyOutputTooLarge { .. } => {
                CommandErrorKind::OutputTooLarge
            }
            Self::PolicyBlocked => CommandErrorKind::Policy,
            Self::StaleInput => CommandErrorKind::StaleInput,
            Self::WindowIdentityUnavailable | Self::FormTargetUnverifiable => {
                CommandErrorKind::WindowIdentity
            }
            Self::QueueFull => CommandErrorKind::QueueFull,
            Self::WorkerUnavailable => CommandErrorKind::WorkerUnavailable,
            Self::InvalidUtf8 => CommandErrorKind::InvalidOutput,
            Self::OutputChannelLost => CommandErrorKind::OutputChannel,
            Self::IncompleteOutput => CommandErrorKind::IncompleteOutput,
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed { detail } => write!(f, "could not start the program: {detail}"),
            Self::Timeout => write!(f, "timed out before it produced output"),
            Self::WaitFailed { operation, reason } => {
                write!(f, "failed while {operation}: {reason}")
            }
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
            Self::FormTargetUnverifiable => write!(
                f,
                "cannot open a snippet form because the output route cannot verify the trigger at the cursor"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_error_kind_preserves_remediation_categories() {
        assert_eq!(
            CommandError::BrokerUnavailable {
                reason: BrokerUnavailableReason::SocketNotConfigured,
            }
            .kind(),
            CommandErrorKind::BrokerUnavailable
        );
        assert_eq!(
            CommandError::WaitFailed {
                operation: ProcessWaitOperation::Reap,
                reason: ProcessWaitFailure::Io {
                    detail: "waitid failed".into(),
                },
            }
            .kind(),
            CommandErrorKind::ProcessWait
        );
        assert_eq!(
            CommandError::PolicyOutputTooLarge { size: 9, limit: 8 }.kind(),
            CommandErrorKind::OutputTooLarge
        );
    }
}
