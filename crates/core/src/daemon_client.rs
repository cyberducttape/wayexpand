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
const CONTROL_MAX_COMMAND_BYTES: usize = 2048;
const CONTROL_BUSY_RESPONSE: &[u8] = b"error=busy\nretryable=true\n";
const MAX_INSERT_TRIGGER_CHARS: usize = 128;
const MAX_EXPLAIN_TEXT_CHARS: usize = 256;

fn decode_response(
    response: Vec<u8>,
    read_result: std::io::Result<()>,
) -> Result<String, DaemonClientError> {
    if let Err(source) = read_result {
        // An overloaded daemon replies without reading the request, so the
        // kernel may report a transport error after delivering the full busy
        // response. The payload is the protocol authority; do not make the
        // typed result depend on which errno the local kernel chose for the
        // peer's close/reset sequence.
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

/// The daemon's answer to [`DaemonOperation::Focus`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonFocus {
    pub generation: u64,
    /// Opaque hexadecimal identity of the focused window; empty when unknown.
    pub token: String,
    /// Whether the window tracker reports exact window identity.
    pub exact_identity: bool,
    /// Whether the connected output route reports atomic replacement.
    /// Missing values from older daemons fail closed.
    pub atomic_replace: bool,
    /// Whether the connected route verifies surrounding text before erase.
    /// Missing values from older daemons fail closed.
    pub verified_replace: bool,
}

impl DaemonFocus {
    /// Parse a `focus` response. Returns `None` when it is missing a
    /// required field, which proves nothing about the focused window. A
    /// withheld (empty) generation is represented as zero; it cannot produce
    /// a verified target because the associated token is also withheld.
    pub fn parse(response: &str) -> Option<Self> {
        let field = |name: &str| {
            response
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
        };
        let generation = match field("focus_generation")? {
            "" => 0,
            value => value.parse().ok()?,
        };
        Some(Self {
            generation,
            token: field("focus_token")?.to_owned(),
            exact_identity: field("focus_identity") == Some("exact"),
            atomic_replace: field("atomic_replace") == Some("true"),
            verified_replace: field("verified_replace") == Some("true"),
        })
    }

    /// The `(generation, token)` for a verified insert, available only when
    /// the window has a token and its identity is exact.
    pub fn verified_target(&self) -> Option<(u64, &str)> {
        (self.exact_identity && !self.token.is_empty())
            .then_some((self.generation, self.token.as_str()))
    }
}

/// Why a tracked insert did not reach the target window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertFailure {
    /// The target window lost focus or changed before the insert ran.
    FocusChanged,
    /// Organization policy blocked the snippet or the unverified insert.
    PolicyRefused,
    /// Keyboard input arrived first, so the insert was abandoned.
    InputArrived,
    /// No output backend was connected.
    NoBackend,
    /// The snippet could not be prepared (unknown, disabled or invalid).
    Refused,
    /// The output backend reported that nothing was typed.
    NotApplied,
    /// A newer insert request replaced this one before it ran.
    Superseded,
}

impl InsertFailure {
    const ALL: [Self; 7] = [
        Self::FocusChanged,
        Self::PolicyRefused,
        Self::InputArrived,
        Self::NoBackend,
        Self::Refused,
        Self::NotApplied,
        Self::Superseded,
    ];

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FocusChanged => "focus_changed",
            Self::PolicyRefused => "policy_refused",
            Self::InputArrived => "input_arrived",
            Self::NoBackend => "no_backend",
            Self::Refused => "refused",
            Self::NotApplied => "not_applied",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|failure| failure.as_str() == value)
    }
}

/// The state of a tracked insert, as answered to `insert-result <id>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertDelivery {
    /// Accepted and not yet resolved.
    Pending,
    /// The output backend acknowledged the text. That is not proof of the
    /// application's resulting document, which no route observes; the wire
    /// name `delivered` is kept for protocol compatibility.
    Delivered,
    /// The backend could not tell whether the text, or part of it, arrived.
    Uncertain,
    /// Nothing was typed.
    Failed(InsertFailure),
}

impl InsertDelivery {
    /// The daemon's reply line for this state.
    pub fn response_line(self, id: u64) -> String {
        match self {
            Self::Pending => format!("insert-result id={id} state=pending\n"),
            Self::Delivered => format!("insert-result id={id} state=delivered\n"),
            Self::Uncertain => format!("insert-result id={id} state=uncertain\n"),
            Self::Failed(reason) => format!(
                "insert-result id={id} state=failed reason={}\n",
                reason.as_str()
            ),
        }
    }

    /// The reply for an id the daemon no longer remembers.
    pub fn unknown_response_line(id: u64) -> String {
        format!("insert-result id={id} state=unknown\n")
    }

    /// Parse an `insert-result` reply for `id`. `Ok(None)` means the daemon
    /// does not know the id; `Err(())` means the reply is not an
    /// `insert-result` answer for `id`.
    #[allow(clippy::result_unit_err)]
    pub fn parse_response(response: &str, id: u64) -> Result<Option<Self>, ()> {
        let mut fields = response.trim_end().split(' ');
        if fields.next() != Some("insert-result") {
            return Err(());
        }
        if fields.next() != Some(format!("id={id}").as_str()) {
            return Err(());
        }
        let state = match fields.next().and_then(|field| field.strip_prefix("state=")) {
            Some("pending") => Some(Self::Pending),
            Some("delivered") => Some(Self::Delivered),
            Some("uncertain") => Some(Self::Uncertain),
            Some("unknown") => None,
            Some("failed") => {
                let reason = fields
                    .next()
                    .and_then(|field| field.strip_prefix("reason="))
                    .and_then(InsertFailure::parse)
                    .ok_or(())?;
                Some(Self::Failed(reason))
            }
            _ => return Err(()),
        };
        if fields.next().is_some() {
            return Err(());
        }
        Ok(state)
    }
}

/// The id in an `insert scheduled id=<n>` reply to a tracked insert.
pub fn parse_insert_scheduled_id(response: &str) -> Option<u64> {
    response
        .trim_end()
        .strip_prefix("insert scheduled id=")?
        .parse()
        .ok()
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
    FocusedAppId,
    Reload,
    Pause,
    Resume,
    Stop,
    ClearUsageStats {
        path: PathBuf,
    },
    Insert {
        trigger: String,
    },
    InsertTarget {
        generation: u64,
        token: String,
        trigger: String,
    },
    /// [`DaemonOperation::InsertTarget`] whose outcome can be queried with
    /// [`DaemonOperation::InsertResult`]. The daemon answers
    /// `insert scheduled id=<n>`; see [`parse_insert_scheduled_id`].
    InsertTargetTracked {
        generation: u64,
        token: String,
        trigger: String,
    },
    /// What became of a tracked insert; see [`InsertDelivery::parse_response`].
    InsertResult {
        id: u64,
    },
    Explain {
        text: String,
        json: bool,
    },
    /// Verify that the daemon's currently loaded configuration can match at
    /// least one enabled static expansion through terminal-only admission.
    /// This performs no insertion and never returns snippet contents.
    TerminalMatchSelfTest,
    ContextSet {
        ttl_seconds: u32,
        report: crate::SessionContextReport,
    },
    ContextGet {
        terminal_id: String,
        session_id: String,
    },
    ContextClear {
        terminal_id: String,
        session_id: String,
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
        let read_result = stream
            .take((CONTROL_MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut response)
            .map(|_| ());
        decode_response(response, read_result)
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
        DaemonOperation::FocusedAppId => Ok("focused-app-id".into()),
        DaemonOperation::Reload => Ok("reload".into()),
        DaemonOperation::Pause => Ok("pause".into()),
        DaemonOperation::Resume => Ok("resume".into()),
        DaemonOperation::Stop => Ok("stop".into()),
        DaemonOperation::ClearUsageStats { path } => {
            let path = path.to_str().ok_or(DaemonClientError::InvalidOperation {
                reason: "usage statistics path is not valid UTF-8",
            })?;
            if path.is_empty() || path.len() > 768 || path.chars().any(char::is_control) {
                return Err(DaemonClientError::InvalidOperation {
                    reason: "usage statistics path is empty, too long, or contains controls",
                });
            }
            Ok(format!("stats-clear {path}"))
        }
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
        DaemonOperation::InsertTargetTracked {
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
            Ok(format!(
                "insert-target-tracked {generation} {token} {trigger}"
            ))
        }
        DaemonOperation::InsertResult { id } => Ok(format!("insert-result {id}")),
        DaemonOperation::Explain { text, json } => {
            validate_text(text, MAX_EXPLAIN_TEXT_CHARS, "explain text")?;
            Ok(format!(
                "explain{} {text}",
                if *json { "-json" } else { "" }
            ))
        }
        DaemonOperation::TerminalMatchSelfTest => Ok("terminal-match-self-test".into()),
        DaemonOperation::ContextSet {
            ttl_seconds,
            report,
        } => {
            if !(1..=300).contains(ttl_seconds) {
                return Err(DaemonClientError::InvalidOperation {
                    reason: "context TTL must be between 1 and 300 seconds",
                });
            }
            report
                .validate()
                .map_err(|reason| DaemonClientError::InvalidOperation { reason })?;
            let payload =
                serde_json::to_string(report).map_err(|_| DaemonClientError::InvalidOperation {
                    reason: "context report could not be encoded",
                })?;
            Ok(format!("context-set {ttl_seconds} {payload}"))
        }
        DaemonOperation::ContextGet {
            terminal_id,
            session_id,
        } => {
            validate_context_target(terminal_id, session_id)?;
            Ok(format!("context-get {terminal_id} {session_id}"))
        }
        DaemonOperation::ContextClear {
            terminal_id,
            session_id,
        } => {
            validate_context_target(terminal_id, session_id)?;
            Ok(format!("context-clear {terminal_id} {session_id}"))
        }
    }
}

fn validate_context_target(terminal_id: &str, session_id: &str) -> Result<(), DaemonClientError> {
    if terminal_id.is_empty()
        || terminal_id.len() > 96
        || !terminal_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b":_-".contains(&byte))
    {
        return Err(DaemonClientError::InvalidOperation {
            reason: "context terminal identity is invalid",
        });
    }
    if session_id.len() != 32 || !session_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DaemonClientError::InvalidOperation {
            reason: "context session identifier must contain 32 hexadecimal characters",
        });
    }
    Ok(())
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
    fn focus_response_yields_a_target_only_for_exact_identity() {
        let exact = DaemonFocus::parse(
            "focus_generation=4\nfocus_token=abcd\nfocus_identity=exact\natomic_replace=true\nverified_replace=true\n",
        )
        .unwrap();
        assert_eq!(exact.verified_target(), Some((4, "abcd")));
        assert!(exact.atomic_replace);
        assert!(exact.verified_replace);

        let inexact = DaemonFocus::parse(
            "focus_generation=4\nfocus_token=abcd\nfocus_identity=unavailable\natomic_replace=false\nverified_replace=false\n",
        )
        .unwrap();
        assert_eq!(inexact.verified_target(), None);
        let unknown =
            DaemonFocus::parse("focus_generation=4\nfocus_token=\nfocus_identity=exact\n").unwrap();
        assert_eq!(unknown.verified_target(), None);

        let restricted =
            DaemonFocus::parse("focus_generation=\nfocus_token=\nfocus_identity=exact\n").unwrap();
        assert_eq!(restricted.generation, 0);
        assert_eq!(restricted.verified_target(), None);
        assert!(restricted.exact_identity);
        assert!(!restricted.atomic_replace);
        assert!(!restricted.verified_replace);

        assert_eq!(
            DaemonFocus::parse("focus_token=abcd\nfocus_identity=exact\n"),
            None
        );
        assert_eq!(DaemonFocus::parse("running\n"), None);
    }

    #[test]
    fn focused_app_id_operation_has_a_fixed_read_only_command() {
        assert_eq!(
            encode_operation(&DaemonOperation::FocusedAppId).unwrap(),
            "focused-app-id"
        );
    }

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
            // Wait for the request to be queued, then drain it before sending
            // the response. The daemon uses the same bounded drain so the
            // response is not dependent on platform-specific reset timing.
            let mut readable = libc::pollfd {
                fd: std::os::fd::AsRawFd::as_raw_fd(&stream),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `readable` is a valid pollfd for a live descriptor.
            assert_eq!(unsafe { libc::poll(&mut readable, 1, 2_000) }, 1);
            let mut command = [0_u8; 7];
            stream.read_exact(&mut command).expect("read request");
            assert_eq!(&command, b"status\n");
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
    fn busy_payload_wins_over_transport_error_variants() {
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            let result = decode_response(
                CONTROL_BUSY_RESPONSE.to_vec(),
                Err(std::io::Error::from(kind)),
            );
            assert!(matches!(result, Err(DaemonClientError::Busy)), "{kind:?}");
        }
    }

    #[test]
    fn encodes_typed_operations_with_validation() {
        assert_eq!(
            encode_operation(&DaemonOperation::TerminalMatchSelfTest).unwrap(),
            "terminal-match-self-test"
        );
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
        assert_eq!(
            encode_operation(&DaemonOperation::InsertTargetTracked {
                generation: 7,
                token: "deadbeef".into(),
                trigger: ":wave".into(),
            })
            .unwrap(),
            "insert-target-tracked 7 deadbeef :wave"
        );
        assert!(matches!(
            encode_operation(&DaemonOperation::InsertTargetTracked {
                generation: 7,
                token: "not-hex".into(),
                trigger: ":wave".into(),
            }),
            Err(DaemonClientError::InvalidOperation { .. })
        ));
        assert_eq!(
            encode_operation(&DaemonOperation::InsertResult { id: 42 }).unwrap(),
            "insert-result 42"
        );
        assert_eq!(
            encode_operation(&DaemonOperation::ClearUsageStats {
                path: PathBuf::from("/tmp/wayexpand usage-stats.json"),
            })
            .unwrap(),
            "stats-clear /tmp/wayexpand usage-stats.json"
        );
        assert!(matches!(
            encode_operation(&DaemonOperation::ClearUsageStats {
                path: PathBuf::from("/tmp/stats\nstop"),
            }),
            Err(DaemonClientError::InvalidOperation { .. })
        ));
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

    #[test]
    fn insert_delivery_round_trips_and_rejects_other_replies() {
        let mut states = vec![
            InsertDelivery::Pending,
            InsertDelivery::Delivered,
            InsertDelivery::Uncertain,
        ];
        states.extend(InsertFailure::ALL.map(InsertDelivery::Failed));
        for state in states {
            assert_eq!(
                InsertDelivery::parse_response(&state.response_line(9), 9),
                Ok(Some(state))
            );
            assert_eq!(
                InsertDelivery::parse_response(&state.response_line(9), 10),
                Err(())
            );
        }
        assert_eq!(
            InsertDelivery::parse_response(&InsertDelivery::unknown_response_line(3), 3),
            Ok(None)
        );
        for reply in [
            "unknown command; expected status\n",
            "insert-result id=3 state=failed\n",
            "insert-result id=3 state=failed reason=mystery\n",
            "insert-result id=3 state=delivered extra\n",
        ] {
            assert_eq!(
                InsertDelivery::parse_response(reply, 3),
                Err(()),
                "{reply:?}"
            );
        }
        assert_eq!(
            parse_insert_scheduled_id("insert scheduled id=12\n"),
            Some(12)
        );
        assert_eq!(parse_insert_scheduled_id("insert scheduled\n"), None);
    }
}
