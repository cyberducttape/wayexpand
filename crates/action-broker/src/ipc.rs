//! IPC Layer - Unix socket communication between daemon and broker.
//!
//! Uses JSON serialization over Unix stream sockets for platform independence
//! and debuggability (can inspect with netcat, socat, etc).

use crate::protocol::{ActionRequest, ActionResponse};
use serde_json;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

const MAX_MESSAGE_BYTES: usize = 1024 * 1024; // 1 MiB
const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(30);
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

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

    #[error("Message too large ({0} bytes, limit {1})")]
    MessageTooLarge(usize, usize),
}

/// Read a newline-terminated line with a size bound.
fn read_bounded_line(reader: &mut BufReader<UnixStream>, limit: usize) -> Result<String, IpcError> {
    let mut line = String::new();
    let mut total = 0;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if total == 0 {
                return Err(IpcError::ConnectionClosed);
            }
            break;
        }

        if let Some(newline_pos) = available.iter().position(|&b| b == b'\n') {
            let consume_len = newline_pos + 1;
            total += consume_len;
            if total > limit {
                return Err(IpcError::MessageTooLarge(total, limit));
            }
            let chunk = std::str::from_utf8(&available[..consume_len])
                .map_err(|_| IpcError::InvalidFormat)?;
            line.push_str(chunk);
            reader.consume(consume_len);
            break;
        }

        let available_len = available.len();
        total += available_len;
        if total > limit {
            return Err(IpcError::MessageTooLarge(total, limit));
        }
        let chunk = std::str::from_utf8(available).map_err(|_| IpcError::InvalidFormat)?;
        line.push_str(chunk);
        reader.consume(available_len);
    }

    Ok(line)
}

/// Broker server listening on a Unix socket.
pub struct BrokerServer {
    listener: UnixListener,
    socket_path: PathBuf,
    socket_identity: (u64, u64),
}

#[cfg(target_os = "linux")]
fn validate_peer(stream: &UnixStream) -> Result<(), std::io::Error> {
    let fd = stream.as_raw_fd();
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if credentials.uid != rustix::process::geteuid().as_raw() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "broker client is not running as the broker user",
        ));
    }
    Ok(())
}

impl BrokerServer {
    /// Create a new broker server at the given socket path.
    pub fn bind<P: AsRef<Path>>(path: P) -> Result<Self, IpcError> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let parent = parent.canonicalize()?;
        let uid = rustix::process::geteuid().as_raw();
        Self::validate_socket_ancestors(&parent, uid)?;

        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if !metadata.file_type().is_socket() || (metadata.uid() != uid && metadata.uid() != 0) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "refusing to replace a non-socket or untrusted broker path",
                )
                .into());
            }
            let identity = (metadata.dev(), metadata.ino());
            let current = std::fs::symlink_metadata(path)?;
            if !current.file_type().is_socket()
                || (current.dev(), current.ino()) != identity
                || (current.uid() != uid && current.uid() != 0)
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "broker socket path changed while checking stale socket",
                )
                .into());
            }
            std::fs::remove_file(path)?;
        }

        let previous_umask = rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
        let listener_result = UnixListener::bind(path);
        rustix::process::umask(previous_umask);
        let listener = listener_result?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            listener,
            socket_path: path.to_path_buf(),
            socket_identity: (metadata.dev(), metadata.ino()),
        })
    }

    fn validate_socket_ancestors(path: &Path, uid: u32) -> Result<(), IpcError> {
        let mut current = path;
        loop {
            let metadata = std::fs::metadata(current)?;
            if !metadata.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "broker socket ancestor is not a directory",
                )
                .into());
            }
            if metadata.uid() != uid && metadata.uid() != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "broker socket ancestor is not owned by the current user or root",
                )
                .into());
            }
            if metadata.mode() & 0o022 != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "broker socket ancestor is writable by group or other users",
                )
                .into());
            }
            if current == Path::new("/") {
                break;
            }
            current = current.parent().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "broker socket ancestor traversal failed",
                )
            })?;
        }
        Ok(())
    }

    /// Accept a new client connection.
    pub fn accept(&self) -> Result<ServerConnection, IpcError> {
        let (stream, _addr) = self.listener.accept()?;
        #[cfg(target_os = "linux")]
        validate_peer(&stream)?;
        stream.set_read_timeout(Some(SOCKET_READ_TIMEOUT))?;
        stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))?;
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
        if let Ok(metadata) = std::fs::metadata(&self.socket_path) {
            if metadata.file_type().is_socket()
                && (metadata.dev(), metadata.ino()) == self.socket_identity
            {
                let _ = std::fs::remove_file(&self.socket_path);
            }
        }
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
        let line = read_bounded_line(&mut self.reader, MAX_MESSAGE_BYTES)?;
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
        stream.set_read_timeout(Some(SOCKET_READ_TIMEOUT))?;
        stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))?;
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
        let line = read_bounded_line(&mut self.reader, MAX_MESSAGE_BYTES)?;
        serde_json::from_str(line.trim()).map_err(IpcError::Json)
    }
}

#[cfg(test)]
mod bounded_frame_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn bounded_reader_rejects_a_line_without_waiting_for_newline() {
        let (mut writer, reader) = UnixStream::pair().expect("socket pair");
        let mut reader = BufReader::new(reader);
        let oversized = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        let writer_thread = std::thread::spawn(move || {
            writer.write_all(&oversized).expect("write oversized frame");
        });

        assert!(matches!(
            read_bounded_line(&mut reader, MAX_MESSAGE_BYTES),
            Err(IpcError::MessageTooLarge(_, MAX_MESSAGE_BYTES))
        ));
        writer_thread.join().expect("writer thread");
    }

    #[test]
    fn bounded_reader_rejects_invalid_utf8_before_json_parsing() {
        let (mut writer, reader) = UnixStream::pair().expect("socket pair");
        let mut reader = BufReader::new(reader);
        writer.write_all(b"{\xff}\n").expect("write invalid frame");

        assert!(matches!(
            read_bounded_line(&mut reader, MAX_MESSAGE_BYTES),
            Err(IpcError::InvalidFormat)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn test_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wayexpand-ipc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir.join(name)
    }

    #[test]
    fn ipc_socket_creation() {
        let socket_path = test_socket("socket");
        let _ = std::fs::remove_file(&socket_path);

        let server = BrokerServer::bind(&socket_path).unwrap();
        assert_eq!(server.socket_path(), socket_path.as_path());

        // Socket file exists
        assert!(socket_path.exists());
    }

    #[test]
    fn ipc_client_server_communication() {
        let socket_path = test_socket("comm");
        let _ = std::fs::remove_file(&socket_path);

        let server = BrokerServer::bind(&socket_path).unwrap();
        let socket_path_clone = socket_path.clone();

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

    #[test]
    fn ipc_rejects_oversized_message() {
        let socket_path = test_socket("oversize");
        let _ = std::fs::remove_file(&socket_path);

        let server = BrokerServer::bind(&socket_path).unwrap();
        let socket_path_clone = socket_path.clone();

        let server_thread = thread::spawn(move || {
            let mut conn = server.accept().unwrap();
            let result = conn.read_request();
            assert!(result.is_err(), "should reject oversized message");
        });

        thread::sleep(std::time::Duration::from_millis(100));
        let mut stream = UnixStream::connect(&socket_path_clone).unwrap();
        // Send a message larger than 1 MiB without a newline
        let huge = vec![b'a'; MAX_MESSAGE_BYTES + 100];
        let _ = stream.write_all(&huge);
        let _ = stream.flush();
        drop(stream);

        server_thread.join().unwrap();
    }
}
