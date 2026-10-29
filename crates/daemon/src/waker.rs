//! Event-driven wakeups for the reactor loop.
//!
//! The capture sources wait in `poll(2)`. Work that arrives from other
//! threads (finished commands, control-socket requests, shutdown signals)
//! signals this eventfd (or a nonblocking pipe where eventfd is unavailable),
//! which the sources include in their poll set, so the loop runs as soon as
//! there is something to do instead of polling for it.

use std::{
    os::fd::OwnedFd,
    sync::{Arc, OnceLock},
};

/// A cloneable handle to a non-blocking reactor wake descriptor.
#[derive(Clone)]
pub struct Waker {
    read_fd: Arc<OwnedFd>,
    /// Present only when the fallback pipe is used. Eventfd is readable and
    /// writable through one descriptor; a pipe has separate ends.
    write_fd: Option<Arc<OwnedFd>>,
}

impl Waker {
    pub fn new() -> std::io::Result<Self> {
        Self::new_with_eventfd(|| {
            rustix::event::eventfd(
                0,
                rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
            )
            .map_err(std::io::Error::from)
        })
    }

    fn new_with_eventfd(
        eventfd: impl FnOnce() -> std::io::Result<OwnedFd>,
    ) -> std::io::Result<Self> {
        match eventfd() {
            Ok(fd) => Ok(Self {
                read_fd: Arc::new(fd),
                write_fd: None,
            }),
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::NOSYS.raw_os_error()) => {
                let (read_fd, write_fd) = rustix::pipe::pipe_with(
                    rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
                )?;
                tracing::warn!(
                    event = "reactor_waker_pipe_fallback",
                    "eventfd is unavailable; using a nonblocking pipe for reactor wakeups"
                );
                Ok(Self {
                    read_fd: Arc::new(read_fd),
                    write_fd: Some(Arc::new(write_fd)),
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Wake the reactor. Never blocks; a counter already at its limit means a
    /// wakeup is pending anyway.
    pub fn wake(&self) {
        let fd = self.write_fd.as_ref().unwrap_or(&self.read_fd);
        let _ = rustix::io::write(&**fd, &1_u64.to_ne_bytes());
    }

    /// The descriptor capture sources add to their poll set. They drain it
    /// when it becomes readable.
    pub fn fd(&self) -> Arc<OwnedFd> {
        Arc::clone(&self.read_fd)
    }

    /// Wait up to `timeout` for a wakeup and consume it. Used while the
    /// reactor is not polling a capture source.
    pub fn wait(&self, timeout: std::time::Duration) {
        let mut fds = [rustix::event::PollFd::new(
            &*self.read_fd,
            rustix::event::PollFlags::IN,
        )];
        let timeout = rustix::event::Timespec {
            tv_sec: i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: i64::from(timeout.subsec_nanos()),
        };
        if rustix::event::poll(&mut fds, Some(&timeout)).is_ok_and(|ready| ready > 0) {
            drain_wake_fd(&self.read_fd);
        }
    }
}

/// Eventfd requires reads of exactly eight bytes. Drain pipes through several
/// eight-byte reads so bursts work for either descriptor type.
fn drain_wake_fd(fd: &OwnedFd) {
    let mut buffer = [0_u8; 8];
    for _ in 0..64 {
        match rustix::io::read(fd, &mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => break,
            Err(_) => break,
        }
    }
}

/// A waker installed after construction, for components (such as the control
/// server) that are created before the reactor's waker exists.
pub type WakerSlot = Arc<OnceLock<Waker>>;

pub fn wake_slot(slot: &WakerSlot) {
    if let Some(waker) = slot.get() {
        waker.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readable(fd: &OwnedFd) -> bool {
        let mut fds = [rustix::event::PollFd::new(fd, rustix::event::PollFlags::IN)];
        let timeout = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        rustix::event::poll(&mut fds, Some(&timeout)).unwrap() > 0
    }

    #[test]
    fn wait_returns_on_wake_and_consumes_it() {
        let waker = Waker::new().unwrap();
        let started = std::time::Instant::now();
        waker.wait(std::time::Duration::from_millis(20));
        assert!(started.elapsed() >= std::time::Duration::from_millis(20));

        waker.wake();
        let started = std::time::Instant::now();
        waker.wait(std::time::Duration::from_secs(5));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(!readable(&waker.fd()));
    }

    #[test]
    fn wake_makes_the_descriptor_readable_until_drained() {
        let waker = Waker::new().unwrap();
        let fd = waker.fd();
        assert!(!readable(&fd));
        waker.wake();
        waker.wake();
        assert!(readable(&fd));
        let mut buffer = [0_u8; 8];
        rustix::io::read(&*fd, &mut buffer).unwrap();
        assert!(!readable(&fd));
    }

    #[test]
    fn unsupported_eventfd_uses_a_nonblocking_pipe() {
        let waker = Waker::new_with_eventfd(|| Err(std::io::Error::from(rustix::io::Errno::NOSYS)))
            .unwrap();
        let fd = waker.fd();
        assert!(!readable(&fd));
        waker.wake();
        assert!(readable(&fd));

        let mut buffer = [0_u8; 8];
        rustix::io::read(&*fd, &mut buffer).unwrap();
        assert!(!readable(&fd));
    }

    #[test]
    fn one_wait_drains_several_queued_pipe_wakes() {
        let waker = Waker::new_with_eventfd(|| Err(std::io::Error::from(rustix::io::Errno::NOSYS)))
            .unwrap();
        for _ in 0..8 {
            waker.wake();
        }
        waker.wait(std::time::Duration::ZERO);
        assert!(!readable(&waker.fd()));
    }

    #[test]
    fn pipe_wake_drain_is_bounded_and_a_later_wait_drains_the_rest() {
        let waker = Waker::new_with_eventfd(|| Err(std::io::Error::from(rustix::io::Errno::NOSYS)))
            .unwrap();
        for _ in 0..80 {
            waker.wake();
        }
        waker.wait(std::time::Duration::ZERO);
        assert!(readable(&waker.fd()));
        waker.wait(std::time::Duration::ZERO);
        assert!(!readable(&waker.fd()));
    }

    #[test]
    fn one_wait_drains_several_queued_eventfd_wakes() {
        let waker = Waker::new_with_eventfd(|| {
            rustix::event::eventfd(
                0,
                rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
            )
            .map_err(std::io::Error::from)
        })
        .unwrap();
        for _ in 0..8 {
            waker.wake();
        }
        waker.wait(std::time::Duration::ZERO);
        assert!(!readable(&waker.fd()));
    }

    #[test]
    fn eventfd_errors_other_than_enosys_are_preserved() {
        let error =
            match Waker::new_with_eventfd(|| Err(std::io::Error::from(rustix::io::Errno::PERM))) {
                Ok(_) => panic!("permission errors must not use the unsupported-syscall fallback"),
                Err(error) => error,
            };
        assert_eq!(
            error.raw_os_error(),
            Some(rustix::io::Errno::PERM.raw_os_error())
        );
    }
}
