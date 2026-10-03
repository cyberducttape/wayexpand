//! Event-driven wakeups for the reactor loop.
//!
//! The capture sources wait in `poll(2)`. Work that arrives from other
//! threads (finished commands, control-socket requests, shutdown signals)
//! signals this eventfd, which the sources include in their poll set, so the
//! loop runs as soon as there is something to do instead of polling for it.

use std::{
    os::fd::OwnedFd,
    sync::{Arc, OnceLock},
};

/// A cloneable handle to a non-blocking eventfd.
#[derive(Clone)]
pub struct Waker {
    fd: Arc<OwnedFd>,
}

impl Waker {
    pub fn new() -> std::io::Result<Self> {
        let fd = rustix::event::eventfd(
            0,
            rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
        )?;
        Ok(Self { fd: Arc::new(fd) })
    }

    /// Wake the reactor. Never blocks; a counter already at its limit means a
    /// wakeup is pending anyway.
    pub fn wake(&self) {
        let _ = rustix::io::write(&*self.fd, &1_u64.to_ne_bytes());
    }

    /// The descriptor capture sources add to their poll set. They drain it
    /// when it becomes readable.
    pub fn fd(&self) -> Arc<OwnedFd> {
        Arc::clone(&self.fd)
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
}
