use crate::protocol::{
    ActionError, ActionRequest, ActionResponse, MAX_ACTION_ID_BYTES, MAX_OUTPUT_BYTES,
};
use serde_json;
use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;

use super::IpcError;

const MAX_STREAM_OUTPUT_BYTES: usize = MAX_OUTPUT_BYTES / 2;

pub(super) const MAX_MESSAGE_BYTES: usize = 1024 * 1024; // 1 MiB

pub(super) fn encode_frame<T: serde::Serialize>(message: &T) -> Result<Vec<u8>, IpcError> {
    let json = serde_json::to_vec(message)?;
    let frame_size = json.len().saturating_add(1); // Include the newline delimiter.
    if frame_size > MAX_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(frame_size, MAX_MESSAGE_BYTES));
    }
    Ok(json)
}

pub(super) fn write_frame(stream: &mut UnixStream, json: &[u8]) -> Result<(), IpcError> {
    stream.write_all(json)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

pub(super) fn truncate_utf8(value: &mut String, limit: usize) -> bool {
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

pub(super) fn bounded_response(response: &ActionResponse) -> ActionResponse {
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

pub(super) fn encode_bounded_response(response: &ActionResponse) -> Vec<u8> {
    let bounded = bounded_response(response);
    match encode_frame(&bounded) {
        Ok(json) => json,
        Err(_) => {
            // Never let attacker/configuration-controlled strings turn the
            // error path into a panic. This response has no external data.
            let fallback = ActionResponse::Error(ActionError::Internal {
                reason: "response could not be serialized within the IPC limit".to_owned(),
            });
            encode_frame(&fallback).unwrap_or_else(|_| {
                // The constant above is far below MAX_MESSAGE_BYTES; retain a
                // final protocol-valid fallback even if serialization changes.
                br#"{"Error":{"Internal":{"reason":"broker response unavailable"}}}"#.to_vec()
            })
        }
    }
}

pub(super) fn is_socket_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// Read a newline-terminated line with a size bound.
pub(super) fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    limit: usize,
) -> Result<String, IpcError> {
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

/// Decode one request frame exactly as the server does: a newline-terminated,
/// size-bounded, UTF-8 JSON line. Exposed for fuzzing.
#[doc(hidden)]
pub fn decode_request_frame<R: BufRead>(reader: &mut R) -> Result<ActionRequest, IpcError> {
    let line = read_bounded_line(reader, MAX_MESSAGE_BYTES)?;
    let request: ActionRequest = serde_json::from_str(line.trim()).map_err(IpcError::Json)?;
    if request.action_id.is_empty() || request.action_id.len() > MAX_ACTION_ID_BYTES {
        return Err(IpcError::InvalidFormat);
    }
    Ok(request)
}
