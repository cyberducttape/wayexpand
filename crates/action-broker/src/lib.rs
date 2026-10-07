//! Action Broker - Secure command execution with fine-grained permission control.
//!
//! The action broker separates command execution from the keyboard capture daemon,
//! enabling per-action security policies without compromising daemon isolation.
//! Its Unix-socket authorization trusts the broker user's UID; it is not a
//! sandbox against compromised software running as that same user.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────┐
//! │ Keyboard Capture Daemon     │
//! │ (restricted: no network)    │
//! └────────────────┬────────────┘
//!                  │
//!        [Authenticated IPC]
//!                  │
//!                  ▼
//! ┌─────────────────────────────┐
//! │ Action Broker               │
//! │ (per-action permissions)    │
//! └─────────────────────────────┘
//! ```

pub mod audit;
mod child_process;
pub mod config;
pub mod executor;
pub mod ipc;
mod path_security;
pub mod protocol;

pub use audit::{policy_hash, AuditEvent, AuditHealth, AuditLogger, CallerIdentity};
pub use config::{ActionConfig, BrokerConfig};
pub use executor::ActionExecutor;
#[doc(hidden)]
pub use ipc::decode_request_frame;
pub use ipc::{BrokerServer, IpcError};
pub use protocol::{ActionError, ActionOutput, ActionRequest, ActionResponse};
pub use wayexpand_broker_client::BrokerClient;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_request_serialization() {
        let req = ActionRequest {
            action_id: "test_action".to_string(),
            timeout_ms: 5000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: ActionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.action_id, "test_action");
    }
}
