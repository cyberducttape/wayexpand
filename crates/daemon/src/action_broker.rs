//! Action Broker Integration - Optional routing of commands to external broker.
//!
//! This is a tested helper for future action-broker routing. It is not wired
//! into the daemon's current command route; commands currently execute via
//! the mature in-daemon path.

use action_broker::{ActionRequest, ActionResponse, BrokerClient};
use std::path::PathBuf;
use tracing::{debug, warn};

/// Action broker client — reconnects per request since the broker server
/// handles one request per connection.
#[allow(dead_code)]
pub struct ActionBrokerManager {
    socket_path: PathBuf,
}

#[allow(dead_code)]
impl ActionBrokerManager {
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Execute an action through the broker (fresh connection per request).
    pub fn execute_action(
        &self,
        action_id: &str,
        timeout_ms: u64,
    ) -> Result<ActionResponse, Box<dyn std::error::Error>> {
        debug!(
            socket_path = %self.socket_path.display(),
            action_id,
            "connecting to action broker"
        );

        let mut client = BrokerClient::connect(&self.socket_path)?;

        let request = ActionRequest {
            action_id: action_id.to_string(),
            timeout_ms,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };

        client.send_request(&request)?;
        let response = client.recv_response()?;
        Ok(response)
    }

    /// Check if broker is available (attempt low-overhead probe).
    pub fn is_available(&self) -> bool {
        if BrokerClient::connect(&self.socket_path).is_ok() {
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
    use action_broker::{ActionOutput, BrokerServer};
    use std::os::unix::fs::PermissionsExt;
    use std::thread;

    #[test]
    fn action_broker_manager_creation() {
        let manager = ActionBrokerManager::new(PathBuf::from("/tmp/test_broker.sock"));
        assert_eq!(manager.socket_path, PathBuf::from("/tmp/test_broker.sock"));
    }

    #[test]
    fn manager_reconnects_for_each_single_request_connection() {
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-daemon-broker-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.join("broker.sock");
        let server = BrokerServer::bind(&socket).unwrap();
        let server_thread = thread::spawn(move || {
            for _ in 0..2 {
                let mut connection = server.accept().unwrap();
                let _request = connection.read_request().unwrap();
                connection
                    .write_response(&ActionResponse::Success(ActionOutput {
                        exit_code: 0,
                        stdout: "ok".into(),
                        stderr: String::new(),
                        duration_ms: 0,
                    }))
                    .unwrap();
            }
        });

        let manager = ActionBrokerManager::new(socket.clone());
        assert!(manager.execute_action("first", 1000).unwrap().is_success());
        assert!(manager.execute_action("second", 1000).unwrap().is_success());
        server_thread.join().unwrap();
        assert!(!socket.exists());
    }
}
