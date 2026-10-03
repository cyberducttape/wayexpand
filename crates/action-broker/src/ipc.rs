//! IPC Layer - Unix socket communication between daemon and broker.
//!
//! Uses JSON serialization over Unix stream sockets for platform independence
//! and debuggability (can inspect with netcat, socat, etc).

use crate::config::is_user_or_root_owner;
use crate::protocol::{ActionError, ActionRequest, ActionResponse, MAX_OUTPUT_BYTES};
use serde_json;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_MESSAGE_BYTES: usize = 1024 * 1024; // 1 MiB
const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(30);
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const DEADLINE_POLL_INTERVAL: Duration = Duration::from_millis(100);

const MAX_STREAM_OUTPUT_BYTES: usize = MAX_OUTPUT_BYTES / 2;

fn encode_frame<T: serde::Serialize>(message: &T) -> Result<Vec<u8>, IpcError> {
    let json = serde_json::to_vec(message)?;
    let frame_size = json.len().saturating_add(1); // Include the newline delimiter.
    if frame_size > MAX_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(frame_size, MAX_MESSAGE_BYTES));
    }
    Ok(json)
}

fn write_frame(stream: &mut UnixStream, json: &[u8]) -> Result<(), IpcError> {
    stream.write_all(json)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn truncate_utf8(value: &mut String, limit: usize) -> bool {
    if value.len() <= limit {
        return false;
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    true
}

fn bounded_response(response: &ActionResponse) -> ActionResponse {
    match response {
        ActionResponse::Success(output) => {
            let mut output = output.clone();
            output.stdout_truncated |= truncate_utf8(&mut output.stdout, MAX_STREAM_OUTPUT_BYTES);
            output.stderr_truncated |= truncate_utf8(&mut output.stderr, MAX_STREAM_OUTPUT_BYTES);
            ActionResponse::Success(output)
        }
        ActionResponse::Error(ActionError::ExitFailure {
            action_id,
            exit_code,
            stderr,
        }) => {
            let mut stderr = stderr.clone();
            truncate_utf8(&mut stderr, MAX_STREAM_OUTPUT_BYTES);
            ActionResponse::Error(ActionError::ExitFailure {
                action_id: action_id.clone(),
                exit_code: *exit_code,
                stderr,
            })
        }
        _ => ActionResponse::Error(ActionError::OutputTruncated {
            limit_bytes: MAX_MESSAGE_BYTES,
        }),
    }
}

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

    #[error("IPC deadline exceeded")]
    DeadlineExceeded,

    #[error("IPC operation cancelled")]
    Cancelled,
}

impl IpcError {
    pub fn is_timeout(&self) -> bool {
        // Unix socket timeouts surface as EAGAIN (WouldBlock); other
        // platforms report TimedOut.
        matches!(self, Self::Io(error) if is_socket_timeout(error))
    }
}

fn is_socket_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// Read a newline-terminated line with a size bound.
fn read_bounded_line(reader: &mut BufReader<UnixStream>, limit: usize) -> Result<String, IpcError> {
    // Accumulate bytes before decoding. A UTF-8 code point can straddle two
    // reads; decoding each fill_buf() chunk independently would reject valid
    // JSON whenever that happened at a buffer boundary.
    let mut line = Vec::new();
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
            line.extend_from_slice(&available[..consume_len]);
            reader.consume(consume_len);
            break;
        }

        let available_len = available.len();
        total += available_len;
        if total > limit {
            return Err(IpcError::MessageTooLarge(total, limit));
        }
        line.extend_from_slice(available);
        reader.consume(available_len);
    }

    String::from_utf8(line).map_err(|_| IpcError::InvalidFormat)
}

/// Broker server listening on a Unix socket.
pub struct BrokerServer {
    listener: UnixListener,
    socket_path: PathBuf,
    socket_identity: (u64, u64),
}

#[derive(Debug, Clone)]
pub struct PeerIdentity {
    pub pid: Option<u32>,
    pub uid: u32,
    pub executable: Option<String>,
}

#[cfg(target_os = "linux")]
fn peer_identity(stream: &UnixStream) -> Result<PeerIdentity, std::io::Error> {
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
    // SO_PEERCRED authenticates the Unix identity, not the calling process's
    // intent. Same-UID applications are inside the broker's trust boundary.
    if credentials.uid != rustix::process::geteuid().as_raw() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "broker client is not running as the broker user",
        ));
    }
    let executable = std::fs::read_link(format!("/proc/{}/exe", credentials.pid))
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    Ok(PeerIdentity {
        pid: u32::try_from(credentials.pid).ok(),
        uid: credentials.uid,
        executable,
    })
}

#[cfg(not(target_os = "linux"))]
fn peer_identity(_stream: &UnixStream) -> Result<PeerIdentity, std::io::Error> {
    Ok(PeerIdentity {
        pid: None,
        uid: rustix::process::geteuid().as_raw(),
        executable: None,
    })
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
            if !metadata.file_type().is_socket() || !is_user_or_root_owner(metadata.uid(), uid) {
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
                || !is_user_or_root_owner(current.uid(), uid)
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
            if !is_user_or_root_owner(metadata.uid(), uid) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "broker socket ancestor is not owned by the current user or root",
                )
                .into());
            }
            let group_or_other_writable = metadata.mode() & 0o022 != 0;
            let is_sticky_directory = metadata.mode() & 0o1000 != 0;
            let is_trusted_sticky_directory =
                is_sticky_directory && is_user_or_root_owner(metadata.uid(), uid);
            if group_or_other_writable && !is_trusted_sticky_directory {
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
        let peer = peer_identity(&stream)?;
        stream.set_read_timeout(Some(SOCKET_READ_TIMEOUT))?;
        stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(ServerConnection {
            stream,
            reader,
            peer,
        })
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
    peer: PeerIdentity,
}

impl ServerConnection {
    pub fn peer_identity(&self) -> &PeerIdentity {
        &self.peer
    }

    /// Read an action request from the client.
    pub fn read_request(&mut self) -> Result<ActionRequest, IpcError> {
        let line = read_bounded_line(&mut self.reader, MAX_MESSAGE_BYTES)?;
        serde_json::from_str(line.trim()).map_err(IpcError::Json)
    }

    /// Send an action response to the client.
    pub fn write_response(&mut self, response: &ActionResponse) -> Result<(), IpcError> {
        match encode_frame(response) {
            Ok(json) => write_frame(&mut self.stream, &json),
            Err(IpcError::MessageTooLarge(_, _)) => {
                // Keep a successful action successful when JSON escaping or
                // an older/foreign client produces an oversized response.
                let bounded = bounded_response(response);
                let json = encode_frame(&bounded).expect("bounded response must fit IPC frame");
                write_frame(&mut self.stream, &json)
            }
            Err(error) => Err(error),
        }
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
        let json = encode_frame(request)?;
        write_frame(&mut self.stream, &json)
    }

    /// Bound both directions of the client connection to the remaining
    /// command deadline. The broker has its own action timeout, but the
    /// client must not wait behind a wedged broker longer than WayExpand's
    /// command contract permits.
    pub fn set_io_timeout(&self, timeout: Duration) -> Result<(), IpcError> {
        self.stream.set_read_timeout(Some(timeout))?;
        self.stream.set_write_timeout(Some(timeout))?;
        Ok(())
    }

    /// Receive an action response from the broker.
    pub fn recv_response(&mut self) -> Result<ActionResponse, IpcError> {
        let line = read_bounded_line(&mut self.reader, MAX_MESSAGE_BYTES)?;
        serde_json::from_str(line.trim()).map_err(IpcError::Json)
    }

    /// Receive a response while checking a command deadline and cancellation
    /// flag in short intervals. A stalled broker therefore cannot hold a
    /// command worker past its configured timeout or shutdown.
    pub fn recv_response_until(
        &mut self,
        deadline: Instant,
        cancelled: Option<&AtomicBool>,
    ) -> Result<ActionResponse, IpcError> {
        let mut frame = Vec::new();
        loop {
            if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                return Err(IpcError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(IpcError::DeadlineExceeded);
            }
            self.stream
                .set_read_timeout(Some(remaining.min(DEADLINE_POLL_INTERVAL)))?;
            // Bound each read so a peer streaming bytes without a newline
            // cannot grow the frame past the message limit inside one call.
            let budget = (MAX_MESSAGE_BYTES + 1).saturating_sub(frame.len()) as u64;
            match (&mut self.reader)
                .take(budget)
                .read_until(b'\n', &mut frame)
            {
                // EOF, with or without a partial frame: the broker hung up.
                Ok(0) => return Err(IpcError::ConnectionClosed),
                Ok(_) => {
                    if frame.len() > MAX_MESSAGE_BYTES {
                        return Err(IpcError::MessageTooLarge(frame.len(), MAX_MESSAGE_BYTES));
                    }
                    if frame.last() == Some(&b'\n') {
                        let line = std::str::from_utf8(&frame).map_err(|error| {
                            IpcError::Io(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                error,
                            ))
                        })?;
                        return serde_json::from_str(line.trim()).map_err(IpcError::Json);
                    }
                }
                Err(error) if is_socket_timeout(&error) => continue,
                Err(error) => return Err(IpcError::Io(error)),
            }
        }
    }
}

#[cfg(test)]
mod bounded_frame_tests {
    use super::*;
    use std::io::Write;

    fn client_pair() -> (BrokerClient, UnixStream) {
        let (stream, peer) = UnixStream::pair().expect("socket pair");
        let reader = BufReader::new(stream.try_clone().expect("clone stream"));
        (BrokerClient { stream, reader }, peer)
    }

    #[test]
    fn deadline_receive_waits_past_the_poll_interval() {
        // Unix socket read timeouts surface as WouldBlock; they must be
        // treated as a poll tick, not a transport failure.
        let (mut client, mut peer) = client_pair();
        let writer_thread = std::thread::spawn(move || {
            std::thread::sleep(DEADLINE_POLL_INTERVAL * 3);
            let response = ActionResponse::Error(crate::protocol::ActionError::ActionNotFound {
                action_id: "slow".into(),
            });
            write_frame(&mut peer, &encode_frame(&response).expect("encode"))
                .expect("write response");
            peer
        });
        let response = client
            .recv_response_until(Instant::now() + Duration::from_secs(5), None)
            .expect("slow broker response should arrive before the deadline");
        assert!(matches!(response, ActionResponse::Error(_)));
        drop(writer_thread.join().expect("writer thread"));
    }

    #[test]
    fn deadline_receive_reports_hangup_mid_frame() {
        let (mut client, mut peer) = client_pair();
        peer.write_all(b"{\"partial\":")
            .expect("write partial frame");
        drop(peer);
        let started = Instant::now();
        assert!(matches!(
            client.recv_response_until(Instant::now() + Duration::from_secs(5), None),
            Err(IpcError::ConnectionClosed)
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn deadline_receive_times_out() {
        let (mut client, _peer) = client_pair();
        assert!(matches!(
            client.recv_response_until(Instant::now() + Duration::from_millis(150), None),
            Err(IpcError::DeadlineExceeded)
        ));
    }

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

    #[test]
    fn bounded_reader_accepts_utf8_split_across_reads() {
        let (mut writer, reader) = UnixStream::pair().expect("socket pair");
        let mut reader = BufReader::with_capacity(1, reader);
        writer
            .write_all("{\"action_id\":\"café\"}\n".as_bytes())
            .expect("write UTF-8 frame");

        let line = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES).expect("valid UTF-8 frame");
        assert_eq!(line, "{\"action_id\":\"café\"}\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ActionOutput;
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
    fn ipc_rejects_non_sticky_writable_ancestor() {
        let socket_path = test_socket("insecure/socket");
        let parent = socket_path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o777)).unwrap();

        let result = BrokerServer::bind(&socket_path);
        assert!(matches!(
            result,
            Err(IpcError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied
        ));

        std::fs::remove_dir(parent).unwrap();
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
                stdout_truncated: false,
                stderr_truncated: false,
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

    fn assert_oversized_success_is_bounded(response: ActionResponse) {
        let (server_stream, client_stream) = UnixStream::pair().unwrap();
        let mut server = ServerConnection {
            reader: BufReader::new(server_stream.try_clone().unwrap()),
            stream: server_stream,
            peer: PeerIdentity {
                pid: None,
                uid: 0,
                executable: None,
            },
        };
        let mut client = BrokerClient {
            reader: BufReader::new(client_stream.try_clone().unwrap()),
            stream: client_stream,
        };

        server.write_response(&response).unwrap();
        match client.recv_response().unwrap() {
            ActionResponse::Success(output) => {
                assert!(output.stdout_truncated || output.stderr_truncated);
                assert!(output.stdout.len() + output.stderr.len() <= MAX_OUTPUT_BYTES);
            }
            other => panic!("oversized success must remain successful: {other:?}"),
        }
    }

    #[test]
    fn ipc_bounds_response_with_large_stdout() {
        assert_oversized_success_is_bounded(ActionResponse::Success(ActionOutput {
            exit_code: 0,
            stdout: "x".repeat(MAX_MESSAGE_BYTES),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        }));
    }

    #[test]
    fn ipc_bounds_response_with_large_stderr() {
        let (server_stream, client_stream) = UnixStream::pair().unwrap();
        let mut server = ServerConnection {
            reader: BufReader::new(server_stream.try_clone().unwrap()),
            stream: server_stream,
            peer: PeerIdentity {
                pid: None,
                uid: 0,
                executable: None,
            },
        };
        let mut client = BrokerClient {
            reader: BufReader::new(client_stream.try_clone().unwrap()),
            stream: client_stream,
        };
        server
            .write_response(&ActionResponse::Error(ActionError::ExitFailure {
                action_id: "large-output".to_string(),
                exit_code: 1,
                stderr: "e".repeat(MAX_MESSAGE_BYTES),
            }))
            .unwrap();
        match client.recv_response().unwrap() {
            ActionResponse::Error(ActionError::ExitFailure { stderr, .. }) => {
                assert!(stderr.len() <= MAX_OUTPUT_BYTES / 2);
            }
            other => panic!("oversized failure must remain a failure: {other:?}"),
        }
    }

    #[test]
    fn ipc_bounds_response_after_json_escaping_expands_output() {
        let escape_heavy = "\n\"\\\u{0001}".repeat(MAX_MESSAGE_BYTES / 8);
        assert!(escape_heavy.len() < MAX_MESSAGE_BYTES);
        assert!(serde_json::to_vec(&escape_heavy).unwrap().len() > MAX_MESSAGE_BYTES);

        assert_oversized_success_is_bounded(ActionResponse::Success(ActionOutput {
            exit_code: 0,
            stdout: escape_heavy,
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        }));
    }
}
