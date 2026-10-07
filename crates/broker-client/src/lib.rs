//! Client-only Unix-socket transport for the WayExpand Action Broker.

use serde::Serialize;
use serde_json::from_str;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;
pub use wayexpand_broker_protocol::{ActionError, ActionOutput, ActionRequest, ActionResponse};
pub use wayexpand_broker_protocol::{MAX_ACTION_ID_BYTES, MAX_OUTPUT_BYTES};

const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(30);
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const DEADLINE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Connection closed")]
    ConnectionClosed,
    #[error("Message too large ({0} bytes, limit {1})")]
    MessageTooLarge(usize, usize),
    #[error("IPC deadline exceeded")]
    DeadlineExceeded,
    #[error("IPC operation cancelled")]
    Cancelled,
}

impl IpcError {
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Io(error) if matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock))
    }
}

fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, IpcError> {
    let json = serde_json::to_vec(message)?;
    let size = json.len().saturating_add(1);
    if size > MAX_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(size, MAX_MESSAGE_BYTES));
    }
    Ok(json)
}

/// Decode one bounded request frame. Kept public for protocol fuzzing and
/// compatibility with the broker's server-side validation.
pub fn decode_request_frame<R: BufRead>(reader: &mut R) -> Result<ActionRequest, IpcError> {
    let mut line = Vec::new();
    let size = reader.read_until(b'\n', &mut line)?;
    if size == 0 {
        return Err(IpcError::ConnectionClosed);
    }
    if line.len() > MAX_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(line.len(), MAX_MESSAGE_BYTES));
    }
    let request: ActionRequest = serde_json::from_slice(&line)?;
    if request.action_id.is_empty() || request.action_id.len() > MAX_ACTION_ID_BYTES {
        return Err(IpcError::ConnectionClosed);
    }
    Ok(request)
}

pub struct BrokerClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl BrokerClient {
    pub fn connect<P: AsRef<Path>>(path: P) -> Result<Self, IpcError> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(SOCKET_READ_TIMEOUT))?;
        stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self { stream, reader })
    }

    pub fn send_request(&mut self, request: &ActionRequest) -> Result<(), IpcError> {
        let json = encode(request)?;
        self.stream.write_all(&json)?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;
        Ok(())
    }

    pub fn set_io_timeout(&self, timeout: Duration) -> Result<(), IpcError> {
        self.stream.set_read_timeout(Some(timeout))?;
        self.stream.set_write_timeout(Some(timeout))?;
        Ok(())
    }

    pub fn recv_response(&mut self) -> Result<ActionResponse, IpcError> {
        let mut frame = Vec::new();
        let size = self.reader.read_until(b'\n', &mut frame)?;
        if size == 0 {
            return Err(IpcError::ConnectionClosed);
        }
        if frame.len() > MAX_MESSAGE_BYTES {
            return Err(IpcError::MessageTooLarge(frame.len(), MAX_MESSAGE_BYTES));
        }
        Ok(serde_json::from_slice(&frame)?)
    }

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
            let budget = (MAX_MESSAGE_BYTES + 1).saturating_sub(frame.len()) as u64;
            match (&mut self.reader)
                .take(budget)
                .read_until(b'\n', &mut frame)
            {
                Ok(0) => return Err(IpcError::ConnectionClosed),
                Ok(_) if frame.len() > MAX_MESSAGE_BYTES => {
                    return Err(IpcError::MessageTooLarge(frame.len(), MAX_MESSAGE_BYTES))
                }
                Ok(_) if frame.last() == Some(&b'\n') => {
                    return Ok(from_str(
                        std::str::from_utf8(&frame)
                            .map_err(|_| IpcError::ConnectionClosed)?
                            .trim(),
                    )?)
                }
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(IpcError::Io(error)),
            }
        }
    }
}
