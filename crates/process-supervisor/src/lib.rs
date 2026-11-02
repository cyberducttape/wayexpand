//! Shared child-process lifecycle management.
//!
//! On Linux, child exit is observed with `waitid(..., WNOWAIT)` so the leader
//! remains a zombie until its owned descendants have been terminated. This closes
//! the PID/PGID reuse window between observing exit and group cleanup.

use std::io;
use std::process::{Child, ChildStdout, Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[cfg(unix)]
pub fn configure_process_group(command: &mut Command) {
    // SAFETY: setpgid is called in the child between fork and exec, where it
    // is safe and async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// A private cgroup v2 below a systemd-delegated service. Unlike a process
/// group, it also contains descendants which call `setsid()`.
#[cfg(target_os = "linux")]
pub struct CommandCgroup {
    path: PathBuf,
    _procs: std::fs::File,
    cleanup: CleanupState,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Default)]
struct CleanupState {
    kill_requested: bool,
    kill_confirmed: bool,
    removed: bool,
}

#[cfg(target_os = "linux")]
struct ReaperJob {
    path: PathBuf,
    state: CleanupState,
    logged: bool,
}

#[cfg(not(target_os = "linux"))]
pub struct CommandCgroup;

/// Backwards-compatible name emphasizing that the cgroup belongs to one child.
pub type ChildCgroup = CommandCgroup;

impl CommandCgroup {
    /// Require cgroup containment by default and fail closed if it cannot be
    /// set up. Process-group-only cleanup is available only through the
    /// explicit `WAYEXPAND_PROCESS_CGROUP=disabled` compatibility opt-out.
    pub fn prepare(command: &mut Command) -> io::Result<Option<Self>> {
        let configured = std::env::var_os("WAYEXPAND_PROCESS_CGROUP");
        if !cgroup_required(configured.as_deref())? {
            return Ok(None);
        }
        Self::prepare_required(command).map(Some)
    }

    /// Prepare cgroup containment regardless of environment. Primarily useful
    /// to integration tests and callers that enforce this policy themselves.
    #[cfg(target_os = "linux")]
    pub fn prepare_required(command: &mut Command) -> io::Result<Self> {
        let parent = delegated_cgroup_path()?;
        cleanup_reaper_sender()?;
        if REAPER_DEGRADED.load(Ordering::Acquire)
            || REAPER_OUTSTANDING.load(Ordering::Acquire) >= REAPER_MAX_OUTSTANDING
        {
            return Err(io::Error::other(
                "command cleanup is degraded because the cgroup reaper reached its bounded backlog; inspect wayexpand doctor and resolve stale command cgroups before retrying",
            ));
        }
        let name = format!(
            "wayexpand-command-{}-{}",
            std::process::id(),
            next_scope_id()
        );
        let path = parent.join(name);
        record_root_dedication(&parent);
        // Register before creating, so a concurrent sweep never mistakes this
        // cgroup for one an escaped process created.
        active_cgroups().insert(path.clone());
        if let Err(error) = std::fs::create_dir(&path) {
            active_cgroups().remove(&path);
            return Err(error);
        }
        let procs_path = path.join("cgroup.procs");
        let procs = match std::fs::OpenOptions::new().write(true).open(&procs_path) {
            Ok(file) => file,
            Err(error) => {
                remove_new_empty_cgroup(&path);
                return Err(error);
            }
        };
        let procs_fd = procs.as_raw_fd();
        // A command runs with the daemon's UID, and a delegated subtree is
        // writable by that UID, so the kernel lets a command migrate itself
        // back to the service cgroup (or elsewhere in the user's tree). That
        // cannot be prevented here; refusing such commands would refuse every
        // command under the bundled units. Commands are trusted executable
        // content: containment targets accidental leaks such as `setsid`
        // daemons, and cleanup also sweeps the service cgroup for escapees.
        // SAFETY: the child callback uses only getpid and write, performs no
        // allocation or locking, and the descriptor stays open until exec.
        unsafe {
            command.pre_exec(move || {
                let mut digits = [0u8; 20];
                let mut value = libc::getpid() as u32;
                let mut cursor = digits.len();
                loop {
                    cursor -= 1;
                    digits[cursor] = b'0' + (value % 10) as u8;
                    value /= 10;
                    if value == 0 {
                        break;
                    }
                }
                let bytes = &digits[cursor..];
                let written = libc::write(procs_fd, bytes.as_ptr().cast(), bytes.len());
                if written < 0 {
                    return Err(io::Error::last_os_error());
                }
                if written as usize != bytes.len() {
                    return Err(io::Error::from(io::ErrorKind::WriteZero));
                }
                Ok(())
            });
        }
        if !path.join("cgroup.kill").exists() {
            remove_new_empty_cgroup(&path);
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cgroup v2 cgroup.kill is unavailable (Linux 5.14 or newer is required)",
            ));
        }
        Ok(Self {
            path,
            _procs: procs,
            cleanup: CleanupState::default(),
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn prepare_required(_command: &mut Command) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cgroup command containment is Linux-only",
        ))
    }

    fn kill_and_remove(&mut self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let result = cleanup_cgroup(&self.path, &mut self.cleanup);
            if self.cleanup.removed {
                active_cgroups().remove(&self.path);
            }
            if let Some(root) = self.path.parent() {
                sweep_escaped_processes(root);
            }
            result
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(())
        }
    }
}

impl Drop for CommandCgroup {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if !self.cleanup.removed {
            if let Err(error) = self.kill_and_remove() {
                tracing::error!(%error, "command cgroup cleanup failed; handing it to the reaper");
                enqueue_reaper(ReaperJob {
                    path: self.path.clone(),
                    state: self.cleanup,
                    logged: false,
                });
            }
        }
    }
}

/// Command cgroups this process created and has not yet removed.
#[cfg(target_os = "linux")]
fn active_cgroups() -> std::sync::MutexGuard<'static, std::collections::HashSet<PathBuf>> {
    static ACTIVE: OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> = OnceLock::new();
    ACTIVE
        .get_or_init(std::sync::Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Service cgroups that held only this process and its direct children when
/// the first command cgroup was created under them. Escapee sweeps run only
/// there: in a shared cgroup (for example a daemon started by hand from a
/// shell with containment required) other members are not ours to kill.
#[cfg(target_os = "linux")]
fn dedicated_roots() -> std::sync::MutexGuard<'static, std::collections::HashMap<PathBuf, bool>> {
    static ROOTS: OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, bool>>> =
        OnceLock::new();
    ROOTS
        .get_or_init(std::sync::Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(target_os = "linux")]
fn record_root_dedication(root: &std::path::Path) {
    if dedicated_roots().contains_key(root) {
        return;
    }
    let me = std::process::id();
    let dedicated = std::fs::read_to_string(root.join("cgroup.procs")).is_ok_and(|procs| {
        procs
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .all(|pid| pid == me || parent_pid(pid).is_none_or(|parent| parent == me))
    });
    if !dedicated {
        tracing::warn!(
            cgroup = %root.display(),
            "the service cgroup is shared with other processes; command descendants that \
             leave their command cgroup will not be swept"
        );
    }
    dedicated_roots()
        .entry(root.to_path_buf())
        .or_insert(dedicated);
}

#[cfg(target_os = "linux")]
fn root_is_dedicated(root: &std::path::Path) -> bool {
    dedicated_roots().get(root).copied().unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn parent_pid(pid: u32) -> Option<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))
        .and_then(|value| value.trim().parse().ok())
}

/// Kill processes that moved themselves out of their command cgroup into the
/// service cgroup `root`, and cgroups under `root` that this process did not
/// create. Only this process and its direct children belong in `root`: the
/// supervisor kills and reaps each command's own process itself, so a
/// direct child is never an escapee, while helpers such as the form window
/// or `pkcheck` are direct children that must survive.
#[cfg(target_os = "linux")]
fn sweep_escaped_processes(root: &std::path::Path) {
    if !root_is_dedicated(root) {
        return;
    }
    let Ok(relative) = root.strip_prefix("/sys/fs/cgroup") else {
        return;
    };
    let expected = format!("0::/{}", relative.display());
    let me = std::process::id();
    if let Ok(procs) = std::fs::read_to_string(root.join("cgroup.procs")) {
        for pid in procs
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
        {
            if pid == me {
                continue;
            }
            kill_if_escaped(pid, me, &expected);
        }
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let path = entry.path();
        if active_cgroups().contains(&path) {
            continue;
        }
        let mut state = CleanupState::default();
        if let Err(error) = cleanup_cgroup(&path, &mut state) {
            tracing::warn!(%error, cgroup = %path.display(), "could not remove a stray cgroup under the service cgroup");
        } else {
            tracing::warn!(cgroup = %path.display(), "removed a cgroup created by a command under the service cgroup");
        }
    }
}

#[cfg(target_os = "linux")]
fn kill_if_escaped(pid: u32, me: u32, expected_cgroup: &str) {
    let Ok(pid_t) = libc::pid_t::try_from(pid) else {
        return;
    };
    // Pin the process first, then confirm through /proc that it is still in
    // the service cgroup and not our child. If the PID was reused after the
    // pidfd was opened, the signal targets the exited process and fails.
    // SAFETY: pidfd_open has no memory-safety preconditions.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid_t, 0) };
    if fd < 0 {
        return;
    }
    // SAFETY: the kernel returned a new descriptor that nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(fd as std::os::fd::RawFd) };
    let in_root = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .is_ok_and(|cgroup| cgroup.lines().any(|line| line == expected_cgroup));
    let parent = parent_pid(pid);
    if !in_root || parent.is_none_or(|parent| parent == me) {
        return;
    }
    tracing::warn!(
        pid,
        "killing a command descendant that left its command cgroup"
    );
    // SAFETY: the descriptor is a valid pidfd; the info pointer may be null.
    unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            libc::SIGKILL,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        );
    }
}

/// How commands started by this process are contained, for diagnostics:
/// `cgroup` (required, fail-closed), `process-group` (explicitly disabled),
/// or `invalid` when `WAYEXPAND_PROCESS_CGROUP` has an unknown value, in
/// which case every command fails to start.
pub fn containment_mode() -> &'static str {
    match cgroup_required(std::env::var_os("WAYEXPAND_PROCESS_CGROUP").as_deref()) {
        Ok(true) => "cgroup",
        Ok(false) => "process-group",
        Err(_) => "invalid",
    }
}

/// Configured child-process guarantee for diagnostics and `doctor --json`.
/// This describes policy; successful command startup additionally verifies
/// that the required cgroup can actually be created.
pub fn containment_policy_mode() -> &'static str {
    containment_policy(std::env::var_os("WAYEXPAND_PROCESS_CGROUP").as_deref())
}

fn containment_policy(configured: Option<&std::ffi::OsStr>) -> &'static str {
    match cgroup_required(configured) {
        Ok(true) => "required-cgroup",
        Ok(false) => "process-group-compatibility",
        Err(_) => "invalid",
    }
}

fn cgroup_required(configured: Option<&std::ffi::OsStr>) -> io::Result<bool> {
    match configured {
        None => Ok(true),
        Some(value) if value == "required" => Ok(true),
        Some(value) if value == "disabled" => Ok(false),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "WAYEXPAND_PROCESS_CGROUP must be 'required' or the explicit compatibility opt-out 'disabled'",
        )),
    }
}

#[cfg(target_os = "linux")]
fn cleanup_cgroup(path: &std::path::Path, state: &mut CleanupState) -> io::Result<()> {
    cleanup_cgroup_with(
        path,
        state,
        |path| std::fs::write(path.join("cgroup.kill"), b"1"),
        |path| std::fs::read_to_string(path.join("cgroup.events")),
        |path| std::fs::remove_dir(path),
    )
}

#[cfg(target_os = "linux")]
fn remove_new_empty_cgroup(path: &std::path::Path) {
    active_cgroups().remove(path);
    match std::fs::remove_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::error!(%error, cgroup = %path.display(), "could not remove unused command cgroup; handing it to the reaper");
            enqueue_reaper(ReaperJob {
                path: path.to_path_buf(),
                state: CleanupState {
                    kill_confirmed: true,
                    ..CleanupState::default()
                },
                logged: false,
            });
        }
    }
}

#[cfg(target_os = "linux")]
fn cleanup_cgroup_with(
    path: &std::path::Path,
    state: &mut CleanupState,
    mut kill: impl FnMut(&std::path::Path) -> io::Result<()>,
    mut events: impl FnMut(&std::path::Path) -> io::Result<String>,
    remove_root: impl FnOnce(&std::path::Path) -> io::Result<()>,
) -> io::Result<()> {
    if state.removed {
        return Ok(());
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "command cgroup path is not a directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            state.kill_confirmed = true;
            state.removed = true;
            return Ok(());
        }
        Err(error) => return Err(error),
    }

    if !state.kill_confirmed {
        if !state.kill_requested {
            kill(path)?;
            state.kill_requested = true;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let content = events(path)?;
            if content.lines().any(|line| line == "populated 0") {
                state.kill_confirmed = true;
                break;
            }
            if std::time::Instant::now() >= deadline {
                // Reissue cgroup.kill on the next attempt in case the first
                // request was interrupted or the task set changed meanwhile.
                state.kill_requested = false;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "command cgroup did not empty after cgroup.kill",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    remove_empty_nested_cgroups(path)?;
    remove_root(path)?;
    state.removed = true;
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_empty_nested_cgroups(path: &std::path::Path) -> io::Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let nested = entry.path();
        remove_empty_nested_cgroups(&nested)?;
        std::fs::remove_dir(nested)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
const REAPER_QUEUE_CAPACITY: usize = 32;
#[cfg(target_os = "linux")]
const REAPER_MAX_OUTSTANDING: usize = 128;
#[cfg(target_os = "linux")]
static REAPER_OUTSTANDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(target_os = "linux")]
static REAPER_FAILURES: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
static REAPER_DEGRADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Current cgroup cleanup health, suitable for `doctor` diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CleanupHealth {
    pub pending_jobs: usize,
    pub failed_attempts: u64,
    pub degraded: bool,
}

pub fn cleanup_health() -> CleanupHealth {
    #[cfg(target_os = "linux")]
    {
        let pending_jobs = REAPER_OUTSTANDING.load(Ordering::Acquire);
        CleanupHealth {
            pending_jobs,
            failed_attempts: REAPER_FAILURES.load(Ordering::Relaxed),
            degraded: REAPER_DEGRADED.load(Ordering::Acquire)
                || pending_jobs >= REAPER_MAX_OUTSTANDING,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        CleanupHealth::default()
    }
}

#[cfg(target_os = "linux")]
static CLEANUP_REAPER: OnceLock<Result<std::sync::mpsc::SyncSender<ReaperJob>, String>> =
    OnceLock::new();

#[cfg(target_os = "linux")]
fn cleanup_reaper_sender() -> io::Result<&'static std::sync::mpsc::SyncSender<ReaperJob>> {
    let result = CLEANUP_REAPER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<ReaperJob>(REAPER_QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("wayexpand-cgroup-reaper".into())
            .spawn(move || run_reaper(&receiver, cleanup_cgroup, true))
            .map(|_| sender)
            .map_err(|error| error.to_string())
    });
    result
        .as_ref()
        .map_err(|message| io::Error::other(message.clone()))
}

#[cfg(target_os = "linux")]
const REAPER_RETRY_MIN: std::time::Duration = std::time::Duration::from_millis(25);
#[cfg(target_os = "linux")]
const REAPER_RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(2);

/// Retry every outstanding cgroup in turn. One cgroup that cannot be removed
/// (for example a task stuck in uninterruptible sleep) must not starve later
/// jobs or let the bounded queue fill, which would make enqueueing fail.
#[cfg(target_os = "linux")]
fn run_reaper(
    receiver: &std::sync::mpsc::Receiver<ReaperJob>,
    mut cleanup: impl FnMut(&std::path::Path, &mut CleanupState) -> io::Result<()>,
    tracked: bool,
) {
    use std::sync::mpsc::RecvTimeoutError;
    let mut pending: Vec<ReaperJob> = Vec::new();
    let mut delay = REAPER_RETRY_MIN;
    loop {
        let received = if pending.is_empty() {
            receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            receiver.recv_timeout(delay)
        };
        match received {
            Ok(job) => {
                pending.push(job);
                pending.extend(receiver.try_iter());
                delay = REAPER_RETRY_MIN;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if pending.is_empty() => return,
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(delay),
        }
        let before = pending.len();
        let removed = retry_pending(&mut pending, &mut cleanup, tracked);
        if tracked && removed > 0 {
            REAPER_OUTSTANDING.fetch_sub(removed, Ordering::AcqRel);
        }
        if pending.len() == before {
            delay = (delay * 2).min(REAPER_RETRY_MAX);
        }
    }
}

/// Make one cleanup attempt for each job, keeping those that still fail.
/// Failures are logged once per job, on the first retry, to avoid log floods.
#[cfg(target_os = "linux")]
fn retry_pending(
    pending: &mut Vec<ReaperJob>,
    cleanup: &mut impl FnMut(&std::path::Path, &mut CleanupState) -> io::Result<()>,
    tracked: bool,
) -> usize {
    let mut removed = 0;
    pending.retain_mut(|job| match cleanup(&job.path, &mut job.state) {
        Ok(()) => {
            active_cgroups().remove(&job.path);
            removed += 1;
            false
        }
        Err(error) => {
            if tracked {
                REAPER_FAILURES.fetch_add(1, Ordering::Relaxed);
            }
            if !job.logged {
                job.logged = true;
                tracing::error!(
                    %error,
                    cgroup = %job.path.display(),
                    "command cgroup cleanup retry failed; continuing to retry"
                );
            }
            true
        }
    });
    removed
}

#[cfg(target_os = "linux")]
fn enqueue_reaper(job: ReaperJob) {
    use std::sync::mpsc::TrySendError;
    let reserved = reserve_reaper_slot(&REAPER_OUTSTANDING);
    if !reserved {
        REAPER_DEGRADED.store(true, Ordering::Release);
        tracing::error!(cgroup = %job.path.display(), "command cgroup reaper reached its bounded backlog; refusing further command execution");
        return;
    }
    match cleanup_reaper_sender() {
        // Never block the dropping thread: it may be a daemon worker on the
        // keyboard path.
        Ok(sender) => match sender.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                REAPER_OUTSTANDING.fetch_sub(1, Ordering::AcqRel);
                REAPER_DEGRADED.store(true, Ordering::Release);
                tracing::error!(
                    cgroup = %job.path.display(),
                    "command cgroup cleanup queue unavailable; the cgroup is left for the service manager"
                );
            }
        },
        Err(error) => {
            REAPER_OUTSTANDING.fetch_sub(1, Ordering::AcqRel);
            REAPER_DEGRADED.store(true, Ordering::Release);
            tracing::error!(%error, "could not start cgroup cleanup reaper");
        }
    }
}

#[cfg(target_os = "linux")]
fn reserve_reaper_slot(pending: &std::sync::atomic::AtomicUsize) -> bool {
    pending
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < REAPER_MAX_OUTSTANDING).then_some(count + 1)
        })
        .is_ok()
}

#[cfg(target_os = "linux")]
fn delegated_cgroup_path() -> io::Result<PathBuf> {
    let contents = std::fs::read_to_string("/proc/self/cgroup")?;
    let relative = contents
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "unified cgroup v2 is not active",
            )
        })?;
    let relative = relative.trim_start_matches('/');
    if relative
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid process cgroup path",
        ));
    }
    let path = PathBuf::from("/sys/fs/cgroup").join(relative);
    let probe = path.join("cgroup.procs");
    std::fs::OpenOptions::new()
        .write(true)
        .open(probe)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("current service cgroup is not delegated: {error}"),
            )
        })?;
    Ok(path)
}

#[cfg(target_os = "linux")]
static NEXT_SCOPE_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(target_os = "linux")]
fn next_scope_id() -> u64 {
    NEXT_SCOPE_ID.fetch_add(1, Ordering::Relaxed)
}

static KILL_FAILURES: AtomicU64 = AtomicU64::new(0);
/// Seconds (since `WARNING_EPOCH`) of the last logged failure, plus one, so
/// zero means "never logged".
static LAST_WARNING: AtomicU64 = AtomicU64::new(0);
static WARNING_EPOCH: OnceLock<Instant> = OnceLock::new();
const WARNING_INTERVAL_SECS: u64 = 60;

/// Process-group cleanups that failed for a reason other than the group
/// already being gone (for example `EPERM`), since process start. Cleanup
/// stays best effort for callers; this exists for metrics and diagnostics.
pub fn kill_failures_total() -> u64 {
    KILL_FAILURES.load(Ordering::Relaxed)
}

/// Send SIGKILL to a process group, classifying the result: `ESRCH` means
/// the group already exited; anything else is counted and logged at most
/// once per minute.
fn kill_group_by_leader(pid: libc::pid_t) {
    // kill(0, ..) signals the caller's own group and kill(-1, ..) every
    // process the user can reach. A child leader is never PID 0 or 1, so
    // refuse rather than ever turn a bad value into a mass kill.
    if pid <= 1 {
        record_kill_failure(pid, &io::Error::from_raw_os_error(libc::EINVAL));
        return;
    }
    // SAFETY: the negative PID targets a process group created by
    // `configure_process_group`; kill has no memory-safety preconditions.
    if unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
        return;
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return;
    }
    record_kill_failure(pid, &error);
}

fn record_kill_failure(pid: libc::pid_t, error: &io::Error) {
    let total = KILL_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
    let now = WARNING_EPOCH.get_or_init(Instant::now).elapsed().as_secs() + 1;
    let last = LAST_WARNING.load(Ordering::Relaxed);
    let due = last == 0 || now.saturating_sub(last) >= WARNING_INTERVAL_SECS;
    if due
        && LAST_WARNING
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        tracing::warn!(
            process_group = pid,
            %error,
            kill_failures_total = total,
            "could not terminate a command's process group; descendants may remain"
        );
    }
}

pub struct ChildSupervisor {
    child: Option<Child>,
    pid: Option<u32>,
    cgroup: Option<CommandCgroup>,
    cgroup_cleanup_error: Option<io::Error>,
    #[cfg(unix)]
    pidfd: Option<OwnedFd>,
    #[cfg(not(target_os = "linux"))]
    observed_status: Option<ExitStatus>,
}

impl ChildSupervisor {
    pub fn new(child: Child) -> Self {
        Self::new_with_cgroup(child, None)
    }

    pub fn new_with_cgroup(child: Child, cgroup: Option<CommandCgroup>) -> Self {
        let pid = child.id();
        Self {
            child: Some(child),
            pid: Some(pid),
            cgroup,
            cgroup_cleanup_error: None,
            #[cfg(unix)]
            pidfd: open_pidfd(pid),
            #[cfg(not(target_os = "linux"))]
            observed_status: None,
        }
    }

    /// Take the child's stdout without exposing lifecycle methods that could
    /// reap the leader before process-group cleanup.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.as_mut()?.stdout.take()
    }

    #[cfg(unix)]
    pub fn pidfd(&self) -> Option<&OwnedFd> {
        self.pidfd.as_ref()
    }

    /// Returns whether the leader has exited without reaping it on Linux.
    pub fn has_exited(&mut self) -> io::Result<bool> {
        #[cfg(target_os = "linux")]
        {
            let (id_type, id) = if let Some(pidfd) = self.pidfd.as_ref() {
                (libc::P_PIDFD, pidfd.as_raw_fd() as libc::id_t)
            } else {
                let pid = self
                    .pid
                    .ok_or_else(|| io::Error::other("child already cleaned up"))?;
                (libc::P_PID, pid as libc::id_t)
            };
            // SAFETY: siginfo_t is plain output storage for waitid.
            let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
            let result = unsafe {
                libc::waitid(
                    id_type,
                    id,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(unsafe { info.si_pid() } != 0)
        }

        #[cfg(not(target_os = "linux"))]
        {
            if self.observed_status.is_some() {
                return Ok(true);
            }
            let status = self
                .child
                .as_mut()
                .ok_or_else(|| io::Error::other("child already cleaned up"))?
                .try_wait()?;
            if let Some(status) = status {
                self.observed_status = Some(status);
                self.pid = None;
                return Ok(true);
            }
            Ok(false)
        }
    }

    /// Terminate the owned cgroup when present, then the process group, and
    /// invalidate the process-group identifier.
    pub fn kill_group(&mut self) {
        let cgroup_finished = if let Some(cgroup) = self.cgroup.as_mut() {
            if let Err(error) = cgroup.kill_and_remove() {
                let total = KILL_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::error!(%error, kill_failures_total = total, "could not empty command cgroup; descendants may remain");
                self.cgroup_cleanup_error = Some(error);
                false
            } else {
                true
            }
        } else {
            false
        };
        if cgroup_finished {
            self.cgroup.take();
        }
        if let Some(pid) = self.pid.take() {
            if let Ok(pid) = libc::pid_t::try_from(pid) {
                // The PID is invalidated above before signalling, so a second
                // kill can never target a recycled process group.
                kill_group_by_leader(pid);
            }
        }
    }

    /// Terminate any remaining members of the process group, invalidate its
    /// identifier, then reap the leader. Calling this directly is safe; callers
    /// do not need a separate `kill_group` first.
    pub fn reap(&mut self) -> io::Result<ExitStatus> {
        // Keep the leader's PID/PGID from becoming reusable before group
        // cleanup. On Linux, `has_exited` observes with WNOWAIT, so cleanup
        // still targets the original group. Invalidate before signalling.
        self.kill_group();
        #[cfg(not(target_os = "linux"))]
        if let Some(status) = self.observed_status.take() {
            self.child = None;
            return Ok(status);
        }
        let status = self
            .child
            .take()
            .ok_or_else(|| io::Error::other("child already reaped"))?
            .wait()?;
        if let Some(error) = self.cgroup_cleanup_error.take() {
            return Err(io::Error::new(
                error.kind(),
                format!("command cgroup cleanup failed: {error}"),
            ));
        }
        Ok(status)
    }
}

impl Drop for ChildSupervisor {
    fn drop(&mut self) {
        self.kill_group();
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

#[cfg(unix)]
fn open_pidfd(pid: u32) -> Option<OwnedFd> {
    #[cfg(target_os = "linux")]
    {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        if fd >= 0 {
            // SAFETY: the successful syscall returned a newly-owned fd.
            return Some(unsafe { OwnedFd::from_raw_fd(fd as std::os::fd::RawFd) });
        }
    }
    let _ = pid;
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, Read},
        process::{Command, Stdio},
        sync::Mutex,
        thread,
        time::Duration,
    };

    // These tests compare a process-global counter. Serialize them so the
    // deliberate invalid-PID increments in one test cannot make another
    // test's before/after assertion scheduler-dependent.
    static KILL_FAILURE_COUNTER_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn process_containment_requires_cgroups_unless_compatibility_is_explicit() {
        assert!(cgroup_required(None).expect("unset defaults to required cgroup containment"));
        assert!(cgroup_required(Some(std::ffi::OsStr::new("required")))
            .expect("required policy is valid"));
        assert!(!cgroup_required(Some(std::ffi::OsStr::new("disabled")))
            .expect("explicit compatibility opt-out is valid"));
        assert!(cgroup_required(Some(std::ffi::OsStr::new("typo"))).is_err());
        assert_eq!(containment_policy(None), "required-cgroup");
        assert_eq!(
            containment_policy(Some(std::ffi::OsStr::new("disabled"))),
            "process-group-compatibility"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn cgroup_cleanup_removes_nested_scopes_after_kill_confirmation() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-cgroup-nested-cleanup-{}-{}",
            std::process::id(),
            next_scope_id()
        ));
        let nested = root.join("nested").join("deeper");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("cgroup.kill"), b"initial").unwrap();
        std::fs::write(root.join("cgroup.events"), b"populated 0\n").unwrap();

        let mut state = CleanupState::default();
        cleanup_cgroup_with(
            &root,
            &mut state,
            |_| Ok(()),
            |_| Ok("populated 0\n".to_string()),
            |path| {
                std::fs::remove_file(path.join("cgroup.kill"))?;
                std::fs::remove_file(path.join("cgroup.events"))?;
                std::fs::remove_dir(path)
            },
        )
        .expect("nested empty groups are removed");

        assert!(state.kill_requested);
        assert!(state.kill_confirmed);
        assert!(state.removed);
        assert!(!root.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn cgroup_cleanup_state_retries_kill_event_and_removal_errors() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-cgroup-retry-{}-{}",
            std::process::id(),
            next_scope_id()
        ));
        std::fs::create_dir(&root).unwrap();
        let mut state = CleanupState::default();
        let mut kill_attempts = 0;
        let mut event_attempts = 0;

        assert!(cleanup_cgroup_with(
            &root,
            &mut state,
            |_| {
                kill_attempts += 1;
                if kill_attempts == 1 {
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                } else {
                    Ok(())
                }
            },
            |_| {
                event_attempts += 1;
                if event_attempts == 1 {
                    Err(io::Error::from(io::ErrorKind::WouldBlock))
                } else {
                    Ok("populated 0\n".to_string())
                }
            },
            |path| std::fs::remove_dir(path),
        )
        .is_err());
        assert!(!state.kill_requested);
        assert!(!state.kill_confirmed);

        assert!(cleanup_cgroup_with(
            &root,
            &mut state,
            |_| {
                kill_attempts += 1;
                Ok(())
            },
            |_| {
                event_attempts += 1;
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            },
            |path| std::fs::remove_dir(path),
        )
        .is_err());
        assert!(state.kill_requested);
        assert!(!state.kill_confirmed);

        assert!(cleanup_cgroup_with(
            &root,
            &mut state,
            |_| panic!("kill should not need repeating before confirmation"),
            |_| Ok("populated 0\n".to_string()),
            |_| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        )
        .is_err());
        assert!(state.kill_confirmed);
        assert!(!state.removed);

        cleanup_cgroup_with(
            &root,
            &mut state,
            |_| panic!("kill should not repeat after empty confirmation"),
            |_| panic!("events should not be reread after empty confirmation"),
            |path| std::fs::remove_dir(path),
        )
        .expect("directory removal retry completes cleanup");
        assert_eq!(kill_attempts, 2);
        assert_eq!(event_attempts, 1);
        assert!(state.kill_confirmed);
        assert!(state.removed);
        assert!(!root.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn cleanup_reaper_retries_a_transient_cgroup_failure() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(4);
        sender
            .send(ReaperJob {
                path: std::path::PathBuf::from("synthetic-cgroup"),
                state: CleanupState::default(),
                logged: false,
            })
            .unwrap();
        drop(sender);
        let mut attempts = 0;
        run_reaper(
            &receiver,
            |_, state| {
                attempts += 1;
                if attempts == 1 {
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                state.kill_requested = true;
                state.kill_confirmed = true;
                state.removed = true;
                Ok(())
            },
            false,
        );
        assert_eq!(attempts, 2);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_stuck_cgroup_does_not_starve_later_cleanup_jobs() {
        let job = |name: &str| ReaperJob {
            path: std::path::PathBuf::from(name),
            state: CleanupState::default(),
            logged: false,
        };
        let mut pending = vec![job("stuck"), job("healthy")];
        let mut cleanup = |path: &std::path::Path, state: &mut CleanupState| {
            if path == std::path::Path::new("stuck") {
                return Err(io::Error::from(io::ErrorKind::ResourceBusy));
            }
            state.removed = true;
            Ok(())
        };
        let removed = retry_pending(&mut pending, &mut cleanup, false);
        assert_eq!(removed, 1);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].path, std::path::Path::new("stuck"));
        assert!(pending[0].logged);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn reaper_backlog_reservations_are_bounded() {
        let outstanding = std::sync::atomic::AtomicUsize::new(0);
        let accepted = (0..400)
            .filter(|_| reserve_reaper_slot(&outstanding))
            .count();
        assert_eq!(accepted, REAPER_MAX_OUTSTANDING);
        assert_eq!(outstanding.load(Ordering::Acquire), REAPER_MAX_OUTSTANDING);
        assert!(!reserve_reaper_slot(&outstanding));

        let mut jobs = (0..REAPER_MAX_OUTSTANDING)
            .map(|index| ReaperJob {
                path: std::path::PathBuf::from(format!("permanent-{index}")),
                state: CleanupState::default(),
                logged: true,
            })
            .collect::<Vec<_>>();
        let mut attempts = 0;
        for _ in 0..4 {
            let mut fail_permanently = |_: &std::path::Path, _: &mut CleanupState| {
                attempts += 1;
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            };
            assert_eq!(retry_pending(&mut jobs, &mut fail_permanently, false), 0);
            assert_eq!(jobs.len(), REAPER_MAX_OUTSTANDING);
        }
        assert_eq!(attempts, REAPER_MAX_OUTSTANDING * 4);
    }

    #[test]
    fn already_exited_group_is_not_a_cleanup_failure() {
        let _guard = KILL_FAILURE_COUNTER_TEST_LOCK.lock().unwrap();
        let mut command = Command::new("/bin/true");
        configure_process_group(&mut command);
        let mut child = command.spawn().unwrap();
        let pid = libc::pid_t::try_from(child.id()).expect("test child PID fits pid_t");
        child.wait().unwrap();
        let before = kill_failures_total();
        kill_group_by_leader(pid); // ESRCH: the group is gone
        assert_eq!(kill_failures_total(), before);
    }

    #[test]
    fn leader_ids_that_would_signal_everything_are_refused_and_counted() {
        let _guard = KILL_FAILURE_COUNTER_TEST_LOCK.lock().unwrap();
        let before = kill_failures_total();
        kill_group_by_leader(0);
        kill_group_by_leader(1);
        assert_eq!(kill_failures_total(), before + 2);
    }

    #[test]
    fn permission_failure_is_counted_without_claiming_cleanup_succeeded() {
        let _guard = KILL_FAILURE_COUNTER_TEST_LOCK.lock().unwrap();
        let before = kill_failures_total();
        record_kill_failure(42, &io::Error::from_raw_os_error(libc::EPERM));
        assert_eq!(kill_failures_total(), before + 1);
    }

    fn wait_until_exited(supervisor: &mut ChildSupervisor) {
        for _ in 0..1000 {
            if supervisor
                .has_exited()
                .expect("exit observation should succeed")
            {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("child did not exit within the test deadline");
    }

    #[test]
    fn exit_observation_does_not_reap_before_group_cleanup() {
        let mut command = Command::new("true");
        configure_process_group(&mut command);
        let child = command.spawn().expect("spawn test child");
        let mut supervisor = ChildSupervisor::new(child);

        wait_until_exited(&mut supervisor);
        // A second observation must still work. On Linux this proves the
        // waitid call used WNOWAIT rather than reaping the process leader.
        assert!(supervisor
            .has_exited()
            .expect("repeated exit observation should succeed"));

        supervisor.kill_group();
        let status = supervisor.reap().expect("reap after group cleanup");
        assert!(status.success());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn exited_child_before_supervisor_initialization_is_still_observable_and_reaped() {
        let mut command = Command::new("true");
        configure_process_group(&mut command);
        let child = command.spawn().expect("spawn short-lived child");
        let pid = child.id();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !process_is_gone_or_zombie(pid) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }

        let mut supervisor = ChildSupervisor::new(child);
        assert!(supervisor
            .has_exited()
            .expect("observe child that exited before supervisor setup"));
        assert!(supervisor.reap().expect("reap observed child").success());
    }

    #[test]
    fn group_cleanup_invalidates_the_identifier_before_killing() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]);
        configure_process_group(&mut command);
        let child = command.spawn().expect("spawn process-group test child");
        let mut supervisor = ChildSupervisor::new(child);

        supervisor.kill_group();
        assert!(supervisor.pid.is_none());
        let status = supervisor.reap().expect("reap killed process group leader");
        assert!(!status.success());

        // A repeated cleanup call cannot target a recycled process-group ID.
        supervisor.kill_group();
    }

    #[cfg(target_os = "linux")]
    fn process_is_gone_or_zombie(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return true;
        };
        // The command name is parenthesized and may contain spaces. The first
        // field after the final ')' is the process state.
        stat.rsplit_once(')')
            .and_then(|(_, fields)| fields.split_whitespace().next())
            .is_some_and(|state| state == "Z")
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn reap_terminates_group_descendants_and_invalidates_identifier() {
        let _guard = KILL_FAILURE_COUNTER_TEST_LOCK.lock().unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & echo $!"])
            .stdout(Stdio::piped());
        configure_process_group(&mut command);
        let child = command.spawn().expect("spawn test child");
        let mut supervisor = ChildSupervisor::new(child);
        let mut stdout =
            std::io::BufReader::new(supervisor.take_stdout().expect("capture descendant PID"));
        let mut descendant_pid = String::new();
        stdout
            .read_line(&mut descendant_pid)
            .expect("read descendant PID");
        let descendant_pid = descendant_pid
            .trim()
            .parse::<u32>()
            .expect("valid descendant PID");
        wait_until_exited(&mut supervisor);

        let status = supervisor.reap().expect("reap and clean up group");
        assert!(status.success());
        assert!(supervisor.pid.is_none());

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !process_is_gone_or_zombie(descendant_pid) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            process_is_gone_or_zombie(descendant_pid),
            "process-group descendant {descendant_pid} survived reap()"
        );

        // Once reaped, dropping or cleaning up again cannot signal a reused
        // process-group identifier.
        let before = kill_failures_total();
        supervisor.kill_group();
        assert_eq!(kill_failures_total(), before);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn reap_kills_descendants_that_ignore_sigterm() {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "sh -c 'trap \"\" TERM; echo $$; exec sleep 30' & wait",
            ])
            .stdout(Stdio::piped());
        configure_process_group(&mut command);
        let child = command.spawn().expect("spawn test child");
        let mut supervisor = ChildSupervisor::new(child);
        let mut stdout =
            std::io::BufReader::new(supervisor.take_stdout().expect("capture child PID"));
        let mut child_pid = String::new();
        stdout.read_line(&mut child_pid).expect("read child PID");
        let child_pid = child_pid.trim().parse::<u32>().expect("valid child PID");

        // The descendant has installed its ignored disposition before it
        // prints its PID. Prove SIGTERM alone will not stop it.
        assert_eq!(
            unsafe { libc::kill(child_pid as libc::pid_t, libc::SIGTERM) },
            0
        );
        thread::sleep(Duration::from_millis(20));
        assert!(!process_is_gone_or_zombie(child_pid));

        let status = supervisor.reap().expect("kill and reap process group");
        assert!(!status.success());
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !process_is_gone_or_zombie(child_pid) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(process_is_gone_or_zombie(child_pid));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn group_cleanup_closes_pipes_inherited_by_descendants() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & echo ready; wait"])
            .stdout(Stdio::piped());
        configure_process_group(&mut command);
        let child = command.spawn().expect("spawn test child");
        let mut supervisor = ChildSupervisor::new(child);
        let mut stdout =
            std::io::BufReader::new(supervisor.take_stdout().expect("capture child stdout"));
        let mut ready = String::new();
        stdout
            .read_line(&mut ready)
            .expect("wait for descendant startup");
        assert_eq!(ready.trim(), "ready");

        let _ = supervisor.reap().expect("kill and reap process group");
        let mut remaining = String::new();
        stdout
            .read_to_string(&mut remaining)
            .expect("drain pipe after group cleanup");
        assert!(remaining.is_empty(), "descendant kept stdout open");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn delegated_cgroup_terminates_new_session_parent_migration() {
        let marker = std::env::temp_dir().join(format!(
            "wayexpand-cgroup-child-started-{}-{}",
            std::process::id(),
            next_scope_id()
        ));
        let nested_marker = marker.with_extension("nested-cgroup");
        let _ = std::fs::remove_file(&marker);
        let _ = std::fs::remove_file(&nested_marker);
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!(
                "echo started > '{}'; cg=/sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup); nested=\"$cg/wayexpand-test-$$\"; if mkdir \"$nested\" 2>/dev/null; then echo \"$nested\" > '{}'; printf '%s\\n' \"$$\" > \"$nested/cgroup.procs\"; fi; setsid sh -c 'path=$(sed -n '\''s/^0:://p'\'' /proc/self/cgroup); parent=${{path%/*}}; parent=${{parent%/*}}; parent=/sys/fs/cgroup$parent; if printf \"%s\\n\" \"$$\" > \"$parent/cgroup.procs\" 2>/dev/null; then echo migrated; else echo denied; fi; echo $$; exec sleep 30' & wait",
                marker.display(), nested_marker.display()
            ),
        ])
        .stdout(Stdio::piped());
        configure_process_group(&mut command);
        let cgroup = match CommandCgroup::prepare_required(&mut command) {
            Ok(cgroup) => Some(cgroup),
            Err(error) if std::env::var_os("WAYEXPAND_REQUIRE_CGROUP_TEST").is_some() => {
                panic!("required delegated cgroup containment unavailable: {error}");
            }
            Err(error) => {
                eprintln!("skipping delegated cgroup regression: {error}");
                return;
            }
        };
        let child = command.spawn().expect("spawn contained test child");
        let mut supervisor = ChildSupervisor::new_with_cgroup(child, cgroup);
        let mut stdout =
            std::io::BufReader::new(supervisor.take_stdout().expect("capture escaped PID"));
        let mut migration = String::new();
        stdout
            .read_line(&mut migration)
            .expect("read parent migration result");
        let migration = migration.trim();
        assert!(
            matches!(migration, "denied" | "migrated"),
            "unexpected parent migration result: {migration:?}"
        );
        let mut escaped_pid = String::new();
        stdout
            .read_line(&mut escaped_pid)
            .expect("read descendant PID");
        let escaped_pid = escaped_pid.trim().parse::<u32>().expect("valid PID");
        // The markers are written before the descendant prints its PID.
        assert!(marker.exists(), "test command did not start");
        let nested_cgroup = std::fs::read_to_string(&nested_marker)
            .expect("test command could not create its nested cgroup");
        let nested_cgroup = std::path::PathBuf::from(nested_cgroup.trim());

        eprintln!("descendant parent-cgroup migration: {migration}");
        let root = nested_cgroup
            .parent()
            .and_then(std::path::Path::parent)
            .expect("command cgroup has a service parent")
            .to_path_buf();
        let dedicated = root_is_dedicated(&root);
        if std::env::var_os("WAYEXPAND_REQUIRE_CGROUP_TEST").is_some() {
            assert!(
                dedicated,
                "test must run as its service cgroup's only process"
            );
        }
        // Whether or not the kernel let the descendant migrate back to the
        // service cgroup, cleanup must terminate it.
        let _ = supervisor.reap().expect("kill cgroup and reap command");
        if migration == "migrated" && !dedicated {
            // A shared cgroup (for example a terminal scope) is never swept,
            // so an escapee there is out of reach by design. Clean it up.
            // SAFETY: the PID was printed by this test's own descendant.
            unsafe {
                libc::kill(escaped_pid as libc::pid_t, libc::SIGKILL);
            }
            eprintln!("skipping escapee assertion: the service cgroup is shared");
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !process_is_gone_or_zombie(escaped_pid) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            process_is_gone_or_zombie(escaped_pid),
            "setsid descendant {escaped_pid} survived cgroup cleanup"
        );
        assert!(
            !nested_cgroup.exists(),
            "nested command cgroup was left behind: {}",
            nested_cgroup.display()
        );
        let _ = std::fs::remove_file(marker);
        let _ = std::fs::remove_file(nested_marker);
    }
}
