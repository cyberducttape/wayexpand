//! IPC Layer - Unix socket communication between daemon and broker.
//!
//! Uses JSON serialization over Unix stream sockets for platform independence
//! and debuggability (can inspect with netcat, socat, etc).

use crate::config::is_user_or_root_owner;
#[cfg(test)]
use crate::protocol::{ActionError, MAX_OUTPUT_BYTES};
use crate::protocol::{ActionRequest, ActionResponse};
use serde_json;
use std::fs::File;
#[cfg(test)]
use std::io::Write;
use std::io::{BufRead, BufReader, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{ffi::OsStr, ffi::OsString};
use thiserror::Error;

mod frame;

pub use frame::decode_request_frame;
use frame::{
    encode_bounded_response, encode_frame, is_socket_timeout, read_bounded_line, write_frame,
    MAX_MESSAGE_BYTES,
};

const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(30);
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const DEADLINE_POLL_INTERVAL: Duration = Duration::from_millis(100);
static SOCKET_PATH_SEQUENCE: AtomicU64 = AtomicU64::new(1);

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

/// Broker server listening on a Unix socket.
pub struct BrokerServer {
    listener: UnixListener,
    socket_path: PathBuf,
    socket_parent: File,
    socket_name: OsString,
    socket_identity: (u64, u64),
}

#[derive(Debug, Clone)]
pub struct PeerIdentity {
    pub pid: Option<u32>,
    /// `/proc/<pid>/stat` start time in clock ticks. Paired with PID to prevent
    /// PID-reuse races when polkit checks this exact Unix process.
    pub start_time: Option<u64>,
    pub uid: u32,
    pub executable: Option<String>,
}

#[cfg(target_os = "linux")]
fn process_start_time(stat: &str) -> Option<u64> {
    // The parenthesized comm field may itself contain spaces or `)`, so split
    // at its final closing delimiter. The remaining fields begin at field 3;
    // starttime is field 22 (index 19 in that suffix).
    let (_, fields) = stat.rsplit_once(") ")?;
    fields.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn read_process_start_time(pid: libc::pid_t) -> Result<u64, std::io::Error> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    process_start_time(&stat).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "could not determine peer process start time",
        )
    })
}

#[cfg(target_os = "linux")]
fn validate_peer_credential_length(length: libc::socklen_t) -> Result<(), std::io::Error> {
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unexpected peer credentials size",
        ));
    }
    Ok(())
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
    validate_peer_credential_length(length)?;
    // SO_PEERCRED supplies the identity polkit must authorize. UID equality
    // alone is not action authorization; every request receives a separate
    // action-specific polkit check before execution.
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
        start_time: Some(read_process_start_time(credentials.pid)?),
        uid: credentials.uid,
        executable,
    })
}

#[cfg(not(target_os = "linux"))]
fn peer_identity(_stream: &UnixStream) -> Result<PeerIdentity, std::io::Error> {
    Ok(PeerIdentity {
        pid: None,
        start_time: None,
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
        let socket_name = path
            .file_name()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?
            .to_os_string();
        let uid = rustix::process::geteuid().as_raw();
        let socket_parent = open_socket_directory(&parent, uid)?;
        let socket_path = parent.join(&socket_name);
        let operation_path = socket_operation_path(&socket_parent, &socket_name, &socket_path);

        if let Some(metadata) = socket_entry_stat(&socket_parent, &socket_name)? {
            if !stat_is_socket(&metadata) || !is_user_or_root_owner(metadata.st_uid, uid) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "refusing to replace a non-socket or untrusted broker path",
                )
                .into());
            }
            // Connecting is the ownership test. Never unlink a path while a
            // live broker is accepting it: a second same-user daemon must be
            // rejected instead of stealing the first daemon's endpoint.
            match UnixStream::connect(&operation_path) {
                Ok(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        "another broker is already listening on this socket",
                    )
                    .into())
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                    ) => {}
                Err(error) => return Err(error.into()),
            }
            let identity = stat_identity(&metadata);
            if !unlink_socket_if_same_at(&socket_parent, &socket_name, identity, uid, || {})? {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "broker socket path changed while checking stale socket",
                )
                .into());
            }
        }

        // Bind and set permissions inside a private staging directory. The
        // final name appears only after the socket is private, via an atomic
        // no-replace rename relative to the held parent directory.
        let staging = SocketStagingDirectory::create(&socket_parent, &socket_name)?;
        let staging_fallback = parent.join(&staging.name).join(&socket_name);
        let staging_path =
            socket_operation_path(&staging.directory, &socket_name, &staging_fallback);
        let listener = UnixListener::bind(&staging_path)?;
        let metadata = socket_entry_stat(&staging.directory, &socket_name)?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "staged broker socket disappeared during creation",
            )
        })?;
        let identity = stat_identity(&metadata);
        if !stat_is_socket(&metadata) || metadata.st_uid != uid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "staged broker socket was replaced during creation",
            )
            .into());
        }
        if let Err(error) =
            std::fs::set_permissions(&staging_path, std::fs::Permissions::from_mode(0o600))
        {
            return Err(error.into());
        }
        let verified =
            socket_entry_stat(&staging.directory, &socket_name)?.is_some_and(|current| {
                stat_is_socket(&current)
                    && stat_identity(&current) == identity
                    && current.st_uid == uid
            });
        if !verified {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "staged broker socket path changed while setting permissions",
            )
            .into());
        }
        rustix::fs::renameat_with(
            &staging.directory,
            &socket_name,
            &socket_parent,
            &socket_name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| {
            if error == rustix::io::Errno::EXIST {
                std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "another broker claimed the socket path during startup",
                )
            } else {
                error.into()
            }
        })?;
        let published = socket_entry_stat(&socket_parent, &socket_name)?.is_some_and(|current| {
            stat_is_socket(&current) && stat_identity(&current) == identity && current.st_uid == uid
        });
        if !published {
            let _ = unlink_socket_if_same_at(&socket_parent, &socket_name, identity, uid, || {});
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "broker socket changed during atomic publication",
            )
            .into());
        }
        drop(staging);
        Ok(Self {
            listener,
            socket_path,
            socket_parent,
            socket_name,
            socket_identity: identity,
        })
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
        let uid = rustix::process::geteuid().as_raw();
        let _ = unlink_socket_if_same_at(
            &self.socket_parent,
            &self.socket_name,
            self.socket_identity,
            uid,
            || {},
        );
    }
}

fn open_socket_directory(path: &Path, uid: u32) -> Result<File, IpcError> {
    use rustix::fs::{Mode, OFlags};

    let root = rustix::fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    let mut current = File::from(root);
    validate_open_socket_directory(&current, uid)?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            if matches!(component, Component::RootDir) {
                continue;
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "canonical broker socket directory contains an unsafe component",
            )
            .into());
        };
        let fd = rustix::fs::openat(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        current = File::from(fd);
        validate_open_socket_directory(&current, uid)?;
    }
    Ok(current)
}

fn validate_open_socket_directory(directory: &File, uid: u32) -> Result<(), IpcError> {
    let metadata = directory.metadata()?;
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
    Ok(())
}

fn socket_operation_path(directory: &File, name: &OsStr, fallback: &Path) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let _ = fallback;
        let mut path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        path.push(name);
        path
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (directory, name);
        fallback.to_path_buf()
    }
}

fn socket_entry_stat(
    directory: &File,
    name: &OsStr,
) -> Result<Option<rustix::fs::Stat>, std::io::Error> {
    match rustix::fs::statat(directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn stat_is_socket(metadata: &rustix::fs::Stat) -> bool {
    metadata.st_mode & libc::S_IFMT == libc::S_IFSOCK
}

fn stat_identity(metadata: &rustix::fs::Stat) -> (u64, u64) {
    (metadata.st_dev, metadata.st_ino)
}

fn temporary_socket_name(kind: &str) -> OsString {
    OsString::from(format!(
        ".wayexpand-{kind}-{}-{}",
        std::process::id(),
        SOCKET_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

struct SocketStagingDirectory {
    parent: File,
    name: OsString,
    directory: File,
    socket_name: OsString,
}

impl SocketStagingDirectory {
    fn create(parent: &File, socket_name: &OsStr) -> Result<Self, std::io::Error> {
        use rustix::fs::{Mode, OFlags};

        for _ in 0..16 {
            let name = temporary_socket_name("broker-stage");
            let parent_clone = parent.try_clone()?;
            match rustix::fs::mkdirat(parent, &name, Mode::from_raw_mode(0o700)) {
                Ok(()) => {
                    let directory = match rustix::fs::openat(
                        parent,
                        &name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                        Mode::empty(),
                    ) {
                        Ok(directory) => directory,
                        Err(error) => {
                            let _ =
                                rustix::fs::unlinkat(parent, &name, rustix::fs::AtFlags::REMOVEDIR);
                            return Err(error.into());
                        }
                    };
                    return Ok(Self {
                        parent: parent_clone,
                        name,
                        directory: File::from(directory),
                        socket_name: socket_name.to_os_string(),
                    });
                }
                Err(error) if error == rustix::io::Errno::EXIST => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique broker socket staging directory",
        ))
    }
}

impl Drop for SocketStagingDirectory {
    fn drop(&mut self) {
        let _ = rustix::fs::unlinkat(
            &self.directory,
            &self.socket_name,
            rustix::fs::AtFlags::empty(),
        );
        let _ = rustix::fs::unlinkat(&self.parent, &self.name, rustix::fs::AtFlags::REMOVEDIR);
    }
}

fn unlink_socket_if_same_at<F>(
    directory: &File,
    name: &OsStr,
    identity: (u64, u64),
    uid: u32,
    between_checks: F,
) -> Result<bool, std::io::Error>
where
    F: FnOnce(),
{
    let matches_expected = |metadata: &rustix::fs::Stat| {
        stat_is_socket(metadata)
            && stat_identity(metadata) == identity
            && is_user_or_root_owner(metadata.st_uid, uid)
    };
    let Some(first) = socket_entry_stat(directory, name)? else {
        return Ok(false);
    };
    if !matches_expected(&first) {
        return Ok(false);
    }
    // Linux has no compare-inode-and-unlink syscall. Atomically rename the
    // entry to a unique quarantine name without replacing another entry, then
    // inspect the moved inode before unlinking. If the original name was
    // swapped after the first stat, restore the unexpected entry instead.
    between_checks();
    for _ in 0..16 {
        let quarantine = temporary_socket_name("broker-remove");
        match rustix::fs::renameat_with(
            directory,
            name,
            directory,
            &quarantine,
            rustix::fs::RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                let moved = socket_entry_stat(directory, &quarantine)?;
                if moved.as_ref().is_some_and(&matches_expected) {
                    // Recheck the quarantined entry immediately before its
                    // removal. The directory is trusted and the name is
                    // unique; a same-UID process remains outside this IPC
                    // boundary.
                    if socket_entry_stat(directory, &quarantine)?
                        .as_ref()
                        .is_some_and(&matches_expected)
                    {
                        rustix::fs::unlinkat(directory, &quarantine, rustix::fs::AtFlags::empty())
                            .map_err(std::io::Error::from)?;
                        return Ok(true);
                    }
                }
                rustix::fs::renameat_with(
                    directory,
                    &quarantine,
                    directory,
                    name,
                    rustix::fs::RenameFlags::NOREPLACE,
                )
                .map_err(std::io::Error::from)?;
                return Ok(false);
            }
            Err(error) if error == rustix::io::Errno::EXIST => continue,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique broker socket quarantine name",
    ))
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

    /// A handle another thread can use to abort this connection's blocking
    /// reads and writes, e.g. when the broker shuts down.
    pub fn canceller(&self) -> Result<ConnectionCanceller, IpcError> {
        Ok(ConnectionCanceller(self.stream.try_clone()?))
    }

    /// Read an action request from the client.
    pub fn read_request(&mut self) -> Result<ActionRequest, IpcError> {
        decode_request_frame(&mut self.reader)
    }

    /// Send an action response to the client.
    pub fn write_response(&mut self, response: &ActionResponse) -> Result<(), IpcError> {
        match encode_frame(response) {
            Ok(json) => write_frame(&mut self.stream, &json),
            Err(IpcError::MessageTooLarge(_, _)) => {
                // Keep a successful action successful when JSON escaping or
                // an older/foreign client produces an oversized response.
                let json = encode_bounded_response(response);
                write_frame(&mut self.stream, &json)
            }
            Err(error) => Err(error),
        }
    }
}

/// Aborts or bounds blocking I/O on a [`ServerConnection`] from another
/// thread. It holds a duplicate of the connection's socket; socket shutdown
/// and timeouts apply to every descriptor of that socket.
pub struct ConnectionCanceller(UnixStream);

impl ConnectionCanceller {
    /// Close the connection: a blocked read or write returns immediately.
    pub fn cancel(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }

    /// Shorten the write timeout, so a client that stopped reading cannot
    /// hold a response write for the full default timeout.
    pub fn limit_writes(&self, timeout: Duration) {
        let _ = self.0.set_write_timeout(Some(timeout));
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

    #[cfg(target_os = "linux")]
    #[test]
    fn process_start_time_parses_comm_with_spaces_and_parentheses() {
        let mut fields = vec!["S".to_string()];
        fields.extend((3..=21).map(|value| value.to_string()));
        fields[19] = "987654".into(); // field 22, starttime
        let stat = format!("42 (worker ) with spaces) {}", fields.join(" "));
        assert_eq!(process_start_time(&stat), Some(987654));
        assert_eq!(process_start_time("42 (truncated) S"), None);
    }
    use crate::protocol::ActionOutput;
    use std::{
        process::Command,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn test_socket(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("wayexpand-ipc-test-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir.join(name)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn peer_credentials_require_the_exact_kernel_structure_size() {
        let expected = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        assert!(validate_peer_credential_length(expected).is_ok());
        for unexpected in [0, expected - 1, expected + 1] {
            let error = validate_peer_credential_length(unexpected).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), "unexpected peer credentials size");
        }
    }

    #[test]
    fn ipc_socket_creation() {
        let socket_path = test_socket("socket");
        let _ = std::fs::remove_file(&socket_path);

        let server = BrokerServer::bind(&socket_path).unwrap();
        assert_eq!(server.socket_path(), socket_path.as_path());

        // Socket file exists
        assert!(socket_path.exists());
        let second = BrokerServer::bind(&socket_path);
        assert!(matches!(
            second,
            Err(IpcError::Io(error)) if error.kind() == std::io::ErrorKind::AddrInUse
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ipc_bind_under_umask_child() {
        let Ok(mask) = std::env::var("WAYEXPAND_TEST_SOCKET_UMASK") else {
            return;
        };
        let mask = u32::from_str_radix(&mask, 8).unwrap();
        let socket_path = PathBuf::from(std::env::var_os("WAYEXPAND_TEST_SOCKET_PATH").unwrap());
        rustix::process::umask(rustix::fs::Mode::from_raw_mode(mask));

        let server = BrokerServer::bind(&socket_path).unwrap();
        assert_eq!(
            std::fs::metadata(&socket_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "socket must be private after bind with umask {mask:o}"
        );
        drop(server);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ipc_creation_remains_private_under_common_umasks() {
        for mask in [0o000, 0o022, 0o077] {
            let socket_path = test_socket(&format!("umask-{mask:o}"));
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "ipc::tests::ipc_bind_under_umask_child",
                    "--nocapture",
                ])
                .env("WAYEXPAND_TEST_SOCKET_UMASK", format!("{mask:o}"))
                .env("WAYEXPAND_TEST_SOCKET_PATH", &socket_path)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "socket bind failed with umask {mask:o}: {}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    #[test]
    fn ipc_concurrent_start_allows_only_one_owner() {
        let socket_path = test_socket("concurrent");
        let attempts = 8;
        let mut threads = Vec::with_capacity(attempts);
        for _ in 0..attempts {
            let path = socket_path.clone();
            threads.push(thread::spawn(move || BrokerServer::bind(path)));
        }

        let mut owners = Vec::new();
        let mut failures = Vec::new();
        for thread in threads {
            match thread.join().expect("broker bind thread should not panic") {
                Ok(server) => owners.push(server),
                Err(error) => failures.push(error),
            }
        }

        assert_eq!(owners.len(), 1, "exactly one concurrent broker may bind");
        assert_eq!(failures.len(), attempts - 1);
        assert!(failures.iter().all(|error| matches!(
            error,
            IpcError::Io(io_error) if io_error.kind() == std::io::ErrorKind::AddrInUse
        )));

        // Keep the winning listener alive while checking that failed starters
        // did not unlink or replace its endpoint.
        assert!(UnixStream::connect(&socket_path).is_ok());
        drop(owners);
        assert!(!socket_path.exists());
    }

    #[test]
    fn ipc_cleanup_preserves_socket_replaced_between_identity_checks() {
        let socket_path = test_socket("race");
        let original_path = socket_path.with_file_name("race-original");
        let server = BrokerServer::bind(&socket_path).unwrap();
        let uid = rustix::process::geteuid().as_raw();
        let mut replacement_listener = None;

        let removed = unlink_socket_if_same_at(
            &server.socket_parent,
            &server.socket_name,
            server.socket_identity,
            uid,
            || {
                std::fs::rename(&socket_path, &original_path).unwrap();
                replacement_listener = Some(UnixListener::bind(&socket_path).unwrap());
            },
        )
        .unwrap();

        assert!(!removed, "a replacement socket must fail the inode recheck");
        assert!(
            UnixStream::connect(&socket_path).is_ok(),
            "the replacement socket must remain available"
        );
        drop(server);
        assert!(
            UnixStream::connect(&socket_path).is_ok(),
            "server cleanup must not unlink a replacement socket"
        );
        drop(replacement_listener);
        std::fs::remove_file(&socket_path).unwrap();
        std::fs::remove_file(&original_path).unwrap();
    }

    #[test]
    fn ipc_drop_uses_the_original_directory_after_its_path_is_replaced() {
        let socket_path = test_socket("pinned-directory");
        let parent = socket_path.parent().unwrap().to_path_buf();
        let relocated_parent = parent.with_file_name(format!(
            "{}-relocated",
            parent.file_name().unwrap().to_string_lossy()
        ));
        let server = BrokerServer::bind(&socket_path).unwrap();

        std::fs::rename(&parent, &relocated_parent).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&socket_path, "replacement entry").unwrap();

        drop(server);

        assert!(
            !relocated_parent.join("pinned-directory").exists(),
            "cleanup should remove the original socket through its pinned directory fd"
        );
        assert_eq!(
            std::fs::read_to_string(&socket_path).unwrap(),
            "replacement entry",
            "cleanup must not follow the replaced parent pathname"
        );
        std::fs::remove_file(&socket_path).unwrap();
        std::fs::remove_dir(&parent).unwrap();
        std::fs::remove_dir(&relocated_parent).unwrap();
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
            parameters: std::collections::BTreeMap::new(),
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
                start_time: None,
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
                start_time: None,
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

    #[test]
    fn canceller_aborts_a_blocked_request_read() {
        let (server_stream, _client_stream) = UnixStream::pair().unwrap();
        server_stream
            .set_read_timeout(Some(SOCKET_READ_TIMEOUT))
            .unwrap();
        let mut server = ServerConnection {
            reader: BufReader::new(server_stream.try_clone().unwrap()),
            stream: server_stream,
            peer: PeerIdentity {
                pid: None,
                start_time: None,
                uid: 0,
                executable: None,
            },
        };
        let canceller = server.canceller().unwrap();
        let started = Instant::now();
        let reader = std::thread::spawn(move || server.read_request());
        std::thread::sleep(Duration::from_millis(50));
        canceller.cancel();
        assert!(reader.join().unwrap().is_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a cancelled read must not wait for the {SOCKET_READ_TIMEOUT:?} socket timeout"
        );
    }
}
