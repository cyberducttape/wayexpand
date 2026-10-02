//! Shared process-group lifecycle management.
//!
//! On Linux, child exit is observed with `waitid(..., WNOWAIT)` so the leader
//! remains a zombie until its process group has been terminated. This closes
//! the PID/PGID reuse window between observing exit and group cleanup.

use std::io;
use std::process::{Child, Command, ExitStatus};

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

pub fn kill_process_group_by_pid(pid: u32) {
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: callers pass the PID of a process group leader created by
        // `configure_process_group`.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

pub struct ChildSupervisor {
    child: Option<Child>,
    pid: Option<u32>,
    #[cfg(unix)]
    pidfd: Option<OwnedFd>,
    #[cfg(not(target_os = "linux"))]
    observed_status: Option<ExitStatus>,
}

impl ChildSupervisor {
    pub fn new(child: Child) -> Self {
        let pid = child.id();
        Self {
            child: Some(child),
            pid: Some(pid),
            #[cfg(unix)]
            pidfd: open_pidfd(pid),
            #[cfg(not(target_os = "linux"))]
            observed_status: None,
        }
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

    /// Terminate the complete process group and invalidate the group ID.
    pub fn kill_group(&mut self) {
        if let Some(pid) = self.pid.take() {
            if let Ok(pid) = libc::pid_t::try_from(pid) {
                // SAFETY: the negative PID targets the child-created process
                // group. The PID is invalidated before returning to prevent a
                // second kill from ever targeting a recycled process group.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
    }

    pub fn reap(&mut self) -> io::Result<ExitStatus> {
        #[cfg(not(target_os = "linux"))]
        if let Some(status) = self.observed_status.take() {
            self.child = None;
            return Ok(status);
        }
        self.child
            .take()
            .ok_or_else(|| io::Error::other("child already reaped"))?
            .wait()
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
    use std::{process::Command, thread, time::Duration};

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
}
