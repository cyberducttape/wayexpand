use std::time::{Duration, Instant};

use reis::{ei, event::EiEvent};

use super::LibeiError;

/// Converts the EIS connection's wire traffic into typed events while
/// servicing handshake requests that may arrive between application events.
pub(super) struct EventPump {
    context: ei::Context,
    converter: reis::event::EiEventConverter,
}

// SAFETY: `EiEventConverter` is `!Send` only because it can store
// `Box<dyn FnOnce(u64)>` request-completion callbacks, which are added solely
// through `EiEventConverter::add_callback_handler`. This crate never calls
// that method and `converter` is private to `EventPump`, so the callback map
// is always empty; every other field is built from `Arc`/`Mutex`-backed reis
// objects that the injector already moves between threads. The pump is used
// by one thread at a time through `&mut LibeiInjector` (no `Sync` is claimed).
unsafe impl Send for EventPump {}

impl EventPump {
    pub(super) fn next(&mut self, timeout: Duration) -> Result<EiEvent, LibeiError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.handle_pending_requests()?;
            if let Some(event) = self.converter.next_event() {
                return Ok(event);
            }
            if !poll_context(
                &self.context,
                deadline.saturating_duration_since(Instant::now()),
            )? {
                return Err(LibeiError::Handshake(reis::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "EIS event deadline expired",
                ))));
            }
            match self.context.read() {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(LibeiError::Disconnected("EIS socket closed".into()))
                }
                Err(error) => return Err(LibeiError::Handshake(error.into())),
            }
        }
    }

    fn handle_pending_requests(&mut self) -> Result<(), LibeiError> {
        while let Some(result) = self.context.pending_event() {
            match result {
                reis::PendingRequestResult::Request(request) => self
                    .converter
                    .handle_event(request)
                    .map_err(|error| LibeiError::Handshake(error.into()))?,
                reis::PendingRequestResult::ParseError(error) => {
                    return Err(LibeiError::Handshake(error.into()))
                }
                reis::PendingRequestResult::InvalidObject(object) => {
                    return Err(LibeiError::Handshake(
                        reis::handshake::HandshakeError::InvalidObject(object).into(),
                    ))
                }
            }
        }
        Ok(())
    }

    /// Every event the server has already sent, without blocking.
    pub(super) fn drain(&mut self) -> Result<Vec<EiEvent>, LibeiError> {
        let mut events = Vec::new();
        loop {
            self.handle_pending_requests()?;
            while let Some(event) = self.converter.next_event() {
                events.push(event);
            }
            if !poll_context(&self.context, Duration::ZERO)? {
                return Ok(events);
            }
            match self.context.read() {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(events),
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(LibeiError::Disconnected("EIS socket closed".into()))
                }
                Err(error) => return Err(LibeiError::Handshake(error.into())),
            }
        }
    }
}

pub(super) fn poll_context(context: &ei::Context, timeout: Duration) -> Result<bool, LibeiError> {
    let timeout = rustix::event::Timespec {
        tv_sec: timeout.as_secs().try_into().unwrap_or(i64::MAX),
        tv_nsec: timeout.subsec_nanos().into(),
    };
    let mut fds = [rustix::event::PollFd::new(
        context,
        rustix::event::PollFlags::IN
            | rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    )];
    if rustix::event::poll(&mut fds, Some(&timeout))
        .map_err(|error| LibeiError::Handshake(reis::Error::Io(error.into())))?
        == 0
    {
        return Ok(false);
    }
    let revents = fds[0].revents();
    if revents.intersects(
        rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    ) {
        return Err(LibeiError::Disconnected(format!(
            "EIS connection became unavailable ({revents:?})"
        )));
    }
    Ok(true)
}

pub(super) fn handshake_with_timeout(
    context: &ei::Context,
    timeout: Duration,
) -> Result<(reis::event::Connection, EventPump), LibeiError> {
    let mut handshaker =
        reis::handshake::EiHandshaker::new("wayexpand", ei::handshake::ContextType::Sender);
    let deadline = Instant::now() + timeout;
    loop {
        while let Some(result) = context.pending_event() {
            let request = match result {
                reis::PendingRequestResult::Request(request) => request,
                reis::PendingRequestResult::ParseError(error) => {
                    return Err(LibeiError::Handshake(error.into()))
                }
                reis::PendingRequestResult::InvalidObject(object) => {
                    return Err(LibeiError::Handshake(
                        reis::handshake::HandshakeError::InvalidObject(object).into(),
                    ))
                }
            };
            if let Some(response) = handshaker
                .handle_event(request)
                .map_err(|error| LibeiError::Handshake(error.into()))?
            {
                let converter = reis::event::EiEventConverter::new(context, response);
                let connection = converter.connection().clone();
                return Ok((
                    connection,
                    EventPump {
                        context: context.clone(),
                        converter,
                    },
                ));
            }
        }
        if !poll_context(context, deadline.saturating_duration_since(Instant::now()))? {
            return Err(LibeiError::Handshake(reis::Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "EIS handshake deadline expired",
            ))));
        }
        context
            .read()
            .map_err(|error| LibeiError::Handshake(reis::Error::Io(error)))?;
    }
}
