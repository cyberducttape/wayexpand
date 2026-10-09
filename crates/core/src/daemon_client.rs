//! Shared client for the daemon's bounded line-oriented control socket.

use crate::CONTROL_MAX_RESPONSE_BYTES;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;

const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(2);
const CONTROL_MAX_COMMAND_BYTES: usize = 1024;
const CONTROL_BUSY_RESPONSE: &[u8] = b"error=busy\nretryable=true\n";
const MAX_INSERT_TRIGGER_CHARS: usize = 128;
const MAX_EXPLAIN_TEXT_CHARS: usize = 256;

#[derive(Debug, Error)]
pub enum DaemonClientError {
    #[error("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required")]
    SocketPathMissing,
    #[error("connecting to daemon socket {path}: {source}")]
    Connect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("configuring daemon socket: {source}")]
    Configure { source: std::io::Error },
    #[error("sending daemon command: {source}")]
    Send { source: std::io::Error },
    #[error("reading daemon response: {source}")]
    Read { source: std::io::Error },
    #[error("daemon control response exceeded {CONTROL_MAX_RESPONSE_BYTES} bytes")]
    ResponseTooLarge,
    #[error("daemon returned a non-UTF-8 control response")]
    InvalidUtf8 { source: std::string::FromUtf8Error },
    #[error("daemon control plane is busy; retry the request")]
    Busy,
    #[error("invalid daemon control operation: {reason}")]
    InvalidOperation { reason: &'static str },
}

/// Typed operations supported by the daemon control socket.
///
/// Frontends should construct one of these operations instead of formatting
/// protocol lines themselves. The client owns validation and wire encoding so
/// limits and control-character rules cannot drift between CLI, TUI, and GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonOperation {
    Status,
    Focus,
    Reload,
    Pause,
    Resume,
    Stop,
    Insert {
        trigger: String,
    },
    InsertTarget {
        generation: u64,
        token: String,
        trigger: String,
    },
    Explain {
        text: String,
        json: bool,
    },
}

/// Bounded client for the daemon control socket, shared by all frontends.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    socket_path: PathBuf,
    io_timeout: Duration,
}

impl DaemonClient {
    /// Resolve `WAYEXPAND_SOCKET`, falling back to the standard runtime path.
    pub fn from_environment() -> Result<Self, DaemonClientError> {
        let socket_path = std::env::var_os("WAYEXPAND_SOCKET")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_RUNTIME_DIR")
                    .map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
            })
            .ok_or(DaemonClientError::SocketPathMissing)?;
        Ok(Self::new(socket_path))
    }

    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            io_timeout: CONTROL_IO_TIMEOUT,
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn request(&self, command: &str) -> Result<String, DaemonClientError> {
        if command.is_empty() || command.len() > CONTROL_MAX_COMMAND_BYTES {
            return Err(DaemonClientError::InvalidOperation {
                reason: "command is empty or exceeds the control limit",
            });
        }
        if command.chars().any(char::is_control) {
            return Err(DaemonClientError::InvalidOperation {
                reason: "command contains control characters",
            });
        }
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|source| {
            DaemonClientError::Connect {
                path: self.socket_path.clone(),
                source,
            }
        })?;
        stream
            .set_read_timeout(Some(self.io_timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.io_timeout)))
            .map_err(|source| DaemonClientError::Configure { source })?;
        stream
            .write_all(command.as_bytes())
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|source| DaemonClientError::Send { source })?;

        let mut response = Vec::with_capacity(CONTROL_MAX_RESPONSE_BYTES);
        if let Err(source) = stream
            .take((CONTROL_MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut response)
        {
            // An overloaded daemon replies without reading the request, so
            // the kernel may report a transport error after delivering the
            // full busy response. The payload is the protocol authority; do
            // not make the typed result depend on which errno the local
            // kernel chose for the peer's close/reset sequence. Only that
            // exact bounded response is accepted after an error.
            if response == CONTROL_BUSY_RESPONSE {
                return Err(DaemonClientError::Busy);
            }
            return Err(DaemonClientError::Read { source });
        }
        if response.len() > CONTROL_MAX_RESPONSE_BYTES {
            return Err(DaemonClientError::ResponseTooLarge);
        }
        if response == CONTROL_BUSY_RESPONSE {
            return Err(DaemonClientError::Busy);
        }
        String::from_utf8(response).map_err(|source| DaemonClientError::InvalidUtf8 { source })
    }

    pub fn status(&self) -> Result<String, DaemonClientError> {
        self.execute(DaemonOperation::Status)
    }

    pub fn execute(&self, operation: DaemonOperation) -> Result<String, DaemonClientError> {
        let command = encode_operation(&operation)?;
        self.request(&command)
    }
}

fn encode_operation(operation: &DaemonOperation) -> Result<String, DaemonClientError> {
    match operation {
        DaemonOperation::Status => Ok("status".into()),
        DaemonOperation::Focus => Ok("focus".into()),
        DaemonOperation::Reload => Ok("reload".into()),
        DaemonOperation::Pause => Ok("pause".into()),
        DaemonOperation::Resume => Ok("resume".into()),
        DaemonOperation::Stop => Ok("stop".into()),
        DaemonOperation::Insert { trigger } => {
            validate_text(trigger, MAX_INSERT_TRIGGER_CHARS, "insert trigger")?;
            Ok(format!("insert {trigger}"))
        }
        DaemonOperation::InsertTarget {
            generation,
            token,
            trigger,
        } => {
            if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(DaemonClientError::InvalidOperation {
                    reason: "insert target token must be hexadecimal",
                });
            }
            validate_text(trigger, MAX_INSERT_TRIGGER_CHARS, "insert trigger")?;
            Ok(format!("insert-target {generation} {token} {trigger}"))
        }
        DaemonOperation::Explain { text, json } => {
            validate_text(text, MAX_EXPLAIN_TEXT_CHARS, "explain text")?;
            Ok(format!(
                "explain{} {text}",
                if *json { "-json" } else { "" }
            ))
        }
    }
}

fn validate_text(
    text: &str,
    max_chars: usize,
    name: &'static str,
) -> Result<(), DaemonClientError> {
    if !(1..=max_chars).contains(&text.chars().count()) {
        return Err(DaemonClientError::InvalidOperation {
            reason: if name == "insert trigger" {
                "insert trigger must be 1-128 characters"
            } else {
                "explain text must be 1-256 characters"
            },
        });
    }
    if text.chars().any(char::is_control) {
        return Err(DaemonClientError::InvalidOperation {
            reason: "operation text contains control characters",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::net::UnixListener, thread};

    #[test]
    fn sends_command_and_reads_response() {
        let socket = std::env::temp_dir().join(format!(
            "wayexpand-daemon-client-{}-{}.sock",
            std::process::id(),
            thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind test socket");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test client");
            let mut command = [0_u8; 7];
            stream.read_exact(&mut command).expect("read command");
            assert_eq!(&command, b"status\n");
            stream
                .write_all(b"state=running\n")
                .expect("write response");
        });

        let response = DaemonClient::new(&socket)
            .status()
            .expect("status response");
        server.join().expect("server thread");
        fs::remove_file(socket).expect("remove test socket");
        assert_eq!(response, "state=running\n");
    }

    #[test]
    fn converts_machine_readable_busy_response_to_a_typed_error() {
        let socket =
            std::env::temp_dir().join(format!("wayexpand-busy-{}.sock", std::process::id()));
        let _ = fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind test socket");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test client");
            // Wait for the request to be queued but leave it unread, so the
            // close produces the ECONNRESET a real overloaded peer can cause.
            let mut readable = libc::pollfd {
                fd: std::os::fd::AsRawFd::as_raw_fd(&stream),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `readable` is a valid pollfd for a live descriptor.
            assert_eq!(unsafe { libc::poll(&mut readable, 1, 2_000) }, 1);
            stream
                .write_all(b"error=busy\nretryable=true\n")
                .expect("write busy response");
        });

        assert!(matches!(
            DaemonClient::new(&socket).status(),
            Err(DaemonClientError::Busy)
        ));
        server.join().expect("server thread");
        fs::remove_file(socket).expect("remove test socket");
    }

    #[test]
    fn encodes_typed_operations_with_validation() {
        assert_eq!(
            encode_operation(&DaemonOperation::Explain {
                text: "hello world".into(),
                json: true,
            })
            .unwrap(),
            "explain-json hello world"
        );
        assert_eq!(
            encode_operation(&DaemonOperation::InsertTarget {
                generation: 7,
                token: "deadbeef".into(),
                trigger: ":wave".into(),
            })
            .unwrap(),
            "insert-target 7 deadbeef :wave"
        );
        assert!(matches!(
            encode_operation(&DaemonOperation::Insert {
                trigger: "bad\ntrigger".into(),
            }),
            Err(DaemonClientError::InvalidOperation { .. })
        ));
    }

    #[test]
    fn raw_requests_reject_control_characters() {
        let client = DaemonClient::new("/does/not/exist");
        assert!(matches!(
            client.request("status\nstop"),
            Err(DaemonClientError::InvalidOperation { .. })
        ));
    }
}
