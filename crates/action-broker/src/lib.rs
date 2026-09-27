//! Action Broker - Secure command execution with fine-grained permission control.
//!
//! The action broker separates command execution from the keyboard capture daemon,
//! enabling per-action security policies and an audit trail without compromising
//! daemon isolation.
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

pub mod config;
pub mod executor;
pub mod ipc;
pub mod protocol;

pub use config::{ActionConfig, BrokerConfig};
pub use executor::ActionExecutor;
pub use ipc::{BrokerClient, BrokerServer};
pub use protocol::{ActionError, ActionOutput, ActionRequest, ActionResponse};

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
