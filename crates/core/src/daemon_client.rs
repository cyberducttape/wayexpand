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
        stream
            .take((CONTROL_MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut response)
            .map_err(|source| DaemonClientError::Read { source })?;
        if response.len() > CONTROL_MAX_RESPONSE_BYTES {
            return Err(DaemonClientError::ResponseTooLarge);
        }
        String::from_utf8(response).map_err(|source| DaemonClientError::InvalidUtf8 { source })
    }

    pub fn status(&self) -> Result<String, DaemonClientError> {
        self.request("status")
    }
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
}
