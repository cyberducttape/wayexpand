//! Action Broker Integration - Optional routing of commands to external broker.
//!
//! If action_broker is configured in organization policy, commands are routed
//! to the broker service instead of executing in the daemon process.

use action_broker::{ActionRequest, ActionResponse, BrokerClient};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Action broker client connection manager.
pub struct ActionBrokerManager {
    socket_path: PathBuf,
    client: Arc<Mutex<Option<BrokerClient>>>,
}

impl ActionBrokerManager {
    /// Create a new action broker manager for the given socket path.
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            client: Arc::new(Mutex::new(None)),
        }
    }

    /// Get or create a connection to the broker.
    async fn get_client(&self) -> Result<BrokerClient, Box<dyn std::error::Error>> {
        let mut guard = self.client.lock().await;

        // Try to reuse existing connection
        if let Some(client) = guard.take() {
            return Ok(client);
        }

        // Create new connection
        debug!(
            socket_path = %self.socket_path.display(),
            "connecting to action broker"
        );

        let client = BrokerClient::connect(&self.socket_path)?;
        Ok(client)
    }

    /// Execute an action through the broker.
    pub async fn execute_action(
        &self,
        action_id: &str,
        timeout_ms: u64,
    ) -> Result<ActionResponse, Box<dyn std::error::Error>> {
        let request = ActionRequest {
            action_id: action_id.to_string(),
            timeout_ms,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };

        let mut client = self.get_client().await?;

        client.send_request(&request)?;
        let response = client.recv_response()?;

        // Return client to pool for reuse
        let mut guard = self.client.lock().await;
        *guard = Some(client);

        Ok(response)
    }

    /// Check if broker is available (attempt low-overhead probe).
    pub async fn is_available(&self) -> bool {
        if let Ok(_client) = BrokerClient::connect(&self.socket_path) {
            debug!("action broker is available");
            return true;
        }
        warn!(
            socket_path = %self.socket_path.display(),
            "action broker is not available"
        );
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_broker_manager_creation() {
        let manager = ActionBrokerManager::new(PathBuf::from("/tmp/test_broker.sock"));
        assert_eq!(manager.socket_path, PathBuf::from("/tmp/test_broker.sock"));
    }
}
