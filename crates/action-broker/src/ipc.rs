//! IPC Layer - Unix socket communication between daemon and broker.
//!
//! Uses JSON serialization over Unix stream sockets for platform independence
//! and debuggability (can inspect with netcat, socat, etc).

use crate::protocol::{ActionRequest, ActionResponse};
use serde_json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::error::Error),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Invalid message format")]
    InvalidFormat,
}

/// Broker server listening on a Unix socket.
pub struct BrokerServer {
    listener: UnixListener,
    socket_path: std::path::PathBuf,
}

impl BrokerServer {
    /// Create a new broker server at the given socket path.
    pub fn bind<P: AsRef<Path>>(path: P) -> Result<Self, IpcError> {
        let path = path.as_ref();

        // Remove existing socket if present
        let _ = std::fs::remove_file(path);

        let listener = UnixListener::bind(path)?;
        Ok(Self {
            listener,
            socket_path: path.to_path_buf(),
        })
    }

    /// Accept a new client connection.
    pub fn accept(&self) -> Result<ServerConnection, IpcError> {
        let (stream, _addr) = self.listener.accept()?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(ServerConnection { stream, reader })
    }

    /// Get the socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for BrokerServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Server-side connection to a client.
pub struct ServerConnection {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl ServerConnection {
    /// Read an action request from the client.
    pub fn read_request(&mut self) -> Result<ActionRequest, IpcError> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(IpcError::ConnectionClosed);
        }
        serde_json::from_str(line.trim()).map_err(IpcError::Json)
    }

    /// Send an action response to the client.
    pub fn write_response(&mut self, response: &ActionResponse) -> Result<(), IpcError> {
        let json = serde_json::to_string(response)?;
        writeln!(self.stream, "{}", json)?;
        self.stream.flush()?;
        Ok(())
    }
}

/// Client connection to the broker.
pub struct BrokerClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl BrokerClient {
    /// Connect to a broker server at the given socket path.
    pub fn connect<P: AsRef<Path>>(path: P) -> Result<Self, IpcError> {
        let stream = UnixStream::connect(path)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self { stream, reader })
    }

    /// Send an action request to the broker.
    pub fn send_request(&mut self, request: &ActionRequest) -> Result<(), IpcError> {
        let json = serde_json::to_string(request)?;
        writeln!(self.stream, "{}", json)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Receive an action response from the broker.
    pub fn recv_response(&mut self) -> Result<ActionResponse, IpcError> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(IpcError::ConnectionClosed);
        }
        serde_json::from_str(line.trim()).map_err(IpcError::Json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn ipc_socket_creation() {
        let socket_path = "/tmp/wayexpand_test_ipc_socket";
        let _ = std::fs::remove_file(socket_path);

        let server = BrokerServer::bind(socket_path).unwrap();
        assert_eq!(server.socket_path(), Path::new(socket_path));

        // Socket file exists
        assert!(std::path::Path::new(socket_path).exists());
    }

    #[test]
    fn ipc_client_server_communication() {
        let socket_path = "/tmp/wayexpand_test_ipc_comm";
        let _ = std::fs::remove_file(socket_path);

        let server = BrokerServer::bind(socket_path).unwrap();
        let socket_path_clone = socket_path.to_string();

        // Spawn server thread
        let server_thread = thread::spawn(move || {
            let mut conn = server.accept().unwrap();

            // Receive request
            let request = conn.read_request().unwrap();
            assert_eq!(request.action_id, "test_action");

            // Send response
            let response = ActionResponse::Success(crate::protocol::ActionOutput {
                exit_code: 0,
                stdout: "test output".to_string(),
                stderr: String::new(),
                duration_ms: 100,
            });
            conn.write_response(&response).unwrap();
        });

        // Connect from client side
        thread::sleep(std::time::Duration::from_millis(100));
        let mut client = BrokerClient::connect(&socket_path_clone).unwrap();

        // Send request
        let request = ActionRequest {
            action_id: "test_action".to_string(),
            timeout_ms: 5000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };
        client.send_request(&request).unwrap();

        // Receive response
        let response = client.recv_response().unwrap();
        assert!(response.is_success());

        server_thread.join().unwrap();
    }
}
