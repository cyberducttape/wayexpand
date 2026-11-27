//! Focused-window trackers for compositors that publish focus over their own
//! IPC socket: Sway (i3-compatible IPC on `$SWAYSOCK`) and Hyprland (the
//! line-based event socket under `$XDG_RUNTIME_DIR/hypr`).
//!
//! Both trackers follow the same contract as the KWin and GNOME bridges:
//! they send an initial snapshot on connect, then one update per focus
//! change; `Some(None)` means focus left every application window (an empty
//! workspace, for example). Window titles are read only when the caller asks
//! for them, because ordinary `app_filter` matching needs only the app ID.
//! Neither tracker ever reports a title or app ID in an error message.

use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use thiserror::Error;
use wayexpand_core::{WindowContext, WindowTrackerError};

pub mod hyprland;
pub mod sway;

pub use hyprland::HyprlandWindowTracker;
pub use sway::SwayWindowTracker;

/// Bound on every request/reply exchange used for probes and snapshots.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Error)]
pub enum IpcWindowError {
    #[error("{0} is not set; this does not look like a {1} session")]
    NotInSession(&'static str, &'static str),
    #[error("could not reach the compositor IPC socket: {0}")]
    Io(#[from] io::Error),
    #[error("the compositor sent a reply WayExpand does not understand: {0}")]
    Protocol(String),
    #[error("the compositor refused the event subscription")]
    SubscriptionRefused,
    #[error("cancelled")]
    Cancelled,
}

pub(crate) fn check_cancelled(cancelled: Option<&AtomicBool>) -> Result<(), IpcWindowError> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(IpcWindowError::Cancelled)
    } else {
        Ok(())
    }
}

pub(crate) fn lost(backend: &'static str, message: impl Into<String>) -> WindowTrackerError {
    WindowTrackerError {
        backend,
        message: message.into(),
        retryable: true,
    }
}

/// What a read attempt against a deadline produced.
pub(crate) enum Filled {
    /// At least one byte was appended.
    Data,
    /// The deadline passed with nothing new.
    TimedOut,
}

/// Append whatever the stream has to `buffer`, waiting at most until
/// `deadline`. A closed stream is an error: the compositor exited or
/// dropped this client.
pub(crate) fn fill_until(
    stream: &mut UnixStream,
    buffer: &mut Vec<u8>,
    deadline: Instant,
    limit: usize,
) -> io::Result<Filled> {
    let now = Instant::now();
    if now >= deadline {
        return Ok(Filled::TimedOut);
    }
    // A zero read timeout means "block forever" to the standard library.
    let wait = deadline.duration_since(now).max(Duration::from_millis(1));
    stream.set_read_timeout(Some(wait))?;
    let mut chunk = [0_u8; 16 * 1024];
    match stream.read(&mut chunk) {
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the compositor closed the IPC connection",
        )),
        Ok(count) => {
            if buffer.len().saturating_add(count) > limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the compositor sent an oversized IPC message",
                ));
            }
            buffer.extend_from_slice(&chunk[..count]);
            Ok(Filled::Data)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(Filled::TimedOut)
        }
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(Filled::Data),
        Err(error) => Err(error),
    }
}

/// Build the context the engine sees. Empty strings mean "unknown".
pub(crate) fn window_context(
    app_id: Option<&str>,
    title: Option<&str>,
    instance_id: Option<String>,
) -> Option<WindowContext> {
    let app_id = app_id.map(str::trim).filter(|value| !value.is_empty());
    let title = title.filter(|value| !value.is_empty());
    if app_id.is_none() && title.is_none() {
        return None;
    }
    Some(WindowContext {
        app_id: app_id.map(str::to_owned),
        title: title.map(str::to_owned),
        instance_id,
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal self-cleaning temporary directory for socket paths.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "wx-ipc-window-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
