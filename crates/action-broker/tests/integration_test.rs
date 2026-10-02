//! Integration tests for the action broker.
//!
//! Tests end-to-end scenarios: server creation, client connection,
//! request/response cycles, timeout handling, and policy enforcement.

use action_broker::{
    ActionConfig, ActionOutput, ActionRequest, ActionResponse, BrokerClient, BrokerConfig,
    BrokerServer,
};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

fn test_socket(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wayexpand-broker-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    dir.join(name)
}

#[test]
fn broker_server_accepts_connections() {
    let socket_path = test_socket("accept");
    let _ = std::fs::remove_file(&socket_path);

    let server = BrokerServer::bind(&socket_path).expect("Failed to bind server");

    // Verify socket exists
    assert!(socket_path.exists());
    assert_eq!(server.socket_path(), socket_path.as_path());
}

#[tokio::test]
async fn broker_client_server_echo_request_response() {
    let socket_path = test_socket("echo");
    let _ = std::fs::remove_file(&socket_path);

    let server = BrokerServer::bind(&socket_path).expect("Failed to bind server");
    let socket_path = socket_path.to_string_lossy().into_owned();

    // Keep the blocking UnixListener accept off the Tokio runtime worker.
    let server_handle = std::thread::spawn(move || {
        let mut conn = server.accept().expect("Failed to accept");

        // Receive request
        let request = conn.read_request().expect("Failed to read request");
        assert_eq!(request.action_id, "test_action");
        assert_eq!(request.timeout_ms, 5000);

        // Send success response
        let response = ActionResponse::Success(ActionOutput {
            exit_code: 0,
            stdout: "Hello from broker".to_string(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 100,
        });
        conn.write_response(&response)
            .expect("Failed to write response");
    });

    // Give server time to bind
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client side
    let mut client = BrokerClient::connect(&socket_path).expect("Failed to connect");

    // Send request
    let request = ActionRequest {
        action_id: "test_action".to_string(),
        timeout_ms: 5000,
        inherit_env: false,
        env_vars: vec![],
        stdout_capture: true,
    };
    client.send_request(&request).expect("Failed to send");

    // Receive response
    let response = client.recv_response().expect("Failed to receive");
    assert!(response.is_success());

    if let ActionResponse::Success(output) = response {
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, "Hello from broker");
        assert_eq!(output.duration_ms, 100);
    }

    // Wait for server thread
    server_handle.join().expect("broker server thread panicked");
}

#[test]
fn broker_config_validation() {
    let mut config = BrokerConfig::default();

    // Valid action
    config.actions.insert(
        "valid".to_string(),
        ActionConfig {
            program: "/usr/bin/echo".to_string(),
            args: vec!["hello".to_string()],
            timeout_ms: 5000,
            server_env: vec![],
            client_forward_env: vec!["HOME".to_string()],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            enabled: true,
            description: None,
        },
    );

    // Should validate successfully
    assert!(config.validate().is_ok());
    assert!(config.get_action("valid").is_some());
}

#[test]
fn broker_config_rejects_relative_paths_when_required() {
    let mut config = BrokerConfig {
        require_absolute_paths: true,
        ..Default::default()
    };

    // Relative path action
    config.actions.insert(
        "relative".to_string(),
        ActionConfig {
            program: "echo".to_string(), // Not absolute
            args: vec![],
            timeout_ms: 5000,
            server_env: vec![],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            enabled: true,
            description: None,
        },
    );

    // Should fail validation
    assert!(config.validate().is_err());
}

#[test]
fn broker_config_from_toml() {
    let toml = r#"
require_absolute_paths = false
strict_env = true

[actions."test"]
program = "/usr/bin/echo"
args = ["hello"]
timeout_ms = 3000
client_forward_env = ["HOME"]
enabled = true
"#;

    let config = BrokerConfig::from_toml(toml).expect("Failed to parse TOML");
    assert!(!config.require_absolute_paths);
    assert!(config.strict_env);
    assert!(config.get_action("test").is_some());

    let action = config.get_action("test").unwrap();
    assert_eq!(action.program, "/usr/bin/echo");
    assert_eq!(action.args, vec!["hello"]);
    assert_eq!(action.timeout_ms, 3000);
}

#[tokio::test]
async fn action_request_with_environment_variables() {
    let request = ActionRequest {
        action_id: "test".to_string(),
        timeout_ms: 5000,
        inherit_env: false,
        env_vars: vec![
            "HOME=/home/user".to_string(),
            "PATH=/usr/bin:/bin".to_string(),
        ],
        stdout_capture: true,
    };

    // Should serialize/deserialize correctly
    let json = serde_json::to_string(&request).expect("Failed to serialize");
    let deserialized: ActionRequest = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.action_id, "test");
    assert_eq!(deserialized.env_vars.len(), 2);
    assert!(deserialized
        .env_vars
        .contains(&"HOME=/home/user".to_string()));
}

#[tokio::test]
async fn action_response_success() {
    let response = ActionResponse::Success(ActionOutput {
        exit_code: 0,
        stdout: "success".to_string(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        duration_ms: 50,
    });

    assert!(response.is_success());
    assert!(response.output().is_some());

    if let Some(output) = response.output() {
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, "success");
    }
}

#[tokio::test]
async fn action_response_error() {
    use action_broker::ActionError;

    let response = ActionResponse::Error(ActionError::ActionNotFound {
        action_id: "missing".to_string(),
    });

    assert!(!response.is_success());
    assert!(response.output().is_none());
}

#[test]
fn action_error_display() {
    use action_broker::ActionError;

    let errors = vec![
        (
            ActionError::ActionNotFound {
                action_id: "test".to_string(),
            },
            "action 'test' not found",
        ),
        (
            ActionError::ActionBlocked {
                action_id: "test".to_string(),
                reason: "disabled".to_string(),
            },
            "action 'test' blocked: disabled",
        ),
        (
            ActionError::Timeout {
                action_id: "test".to_string(),
                timeout_ms: 5000,
            },
            "action 'test' timed out after 5000 ms",
        ),
    ];

    for (error, expected_msg) in errors {
        assert_eq!(error.to_string(), expected_msg);
    }
}
