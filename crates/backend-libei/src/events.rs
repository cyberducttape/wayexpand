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
//
// This rests on reis internals, so reis is pinned to an exact version and
// `send_invariant` (below) checks what can be checked: that this crate never
// registers a callback, that the converter's other field types are `Send`,
// and that the converter is still `!Send` (once reis makes it `Send`, delete
// this impl). Re-read `EiEventConverter`'s fields on every reis upgrade.
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

    /// Hand `handle` every event the server has already sent, without
    /// blocking, within `budget`. A server that keeps sending could otherwise
    /// hold the caller here, and the typing waiting on it, indefinitely; past
    /// the budget this fails closed (nothing is typed against state that is
    /// still arriving) and the rest stays queued for the next call.
    pub(super) fn drain(
        &mut self,
        budget: DrainBudget,
        mut handle: impl FnMut(EiEvent) -> Result<(), LibeiError>,
    ) -> Result<(), LibeiError> {
        let deadline = Instant::now() + budget.time;
        let mut handled = 0_usize;
        loop {
            self.handle_pending_requests()?;
            while let Some(event) = self.converter.next_event() {
                handle(event)?;
                handled += 1;
                budget.check(handled, deadline)?;
            }
            budget.check(handled, deadline)?;
            if !poll_context(&self.context, Duration::ZERO)? {
                return Ok(());
            }
            match self.context.read() {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(LibeiError::Disconnected("EIS socket closed".into()))
                }
                Err(error) => return Err(LibeiError::Handshake(error.into())),
            }
        }
    }
}

/// How much a single [`EventPump::drain`] may take on.
#[derive(Clone, Copy)]
pub(super) struct DrainBudget {
    pub(super) events: usize,
    pub(super) time: Duration,
}

impl DrainBudget {
    /// Generous for a layout switch or device change (a handful of events),
    /// small enough that a flooding server cannot stall typing.
    pub(super) const SYNC: Self = Self {
        events: 512,
        time: Duration::from_millis(50),
    };

    fn check(self, handled: usize, deadline: Instant) -> Result<(), LibeiError> {
        if handled >= self.events || Instant::now() >= deadline {
            return Err(LibeiError::EventBacklog);
        }
        Ok(())
    }
}

/// Wait until the EIS socket is readable or `timeout` has passed; `false`
/// means the time ran out.
pub(super) fn poll_context(context: &ei::Context, timeout: Duration) -> Result<bool, LibeiError> {
    let deadline = Instant::now() + timeout;
    let mut fds = [rustix::event::PollFd::new(
        context,
        rustix::event::PollFlags::IN
            | rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    )];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let remaining = rustix::event::Timespec {
            tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: remaining.subsec_nanos().into(),
        };
        match rustix::event::poll(&mut fds, Some(&remaining)) {
            Ok(0) => return Ok(false),
            Ok(_) => break,
            // A handled signal interrupted the wait. That is neither a broken
            // EIS connection nor the deadline, so wait out the time left.
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(LibeiError::Handshake(reis::Error::Io(error.into()))),
        }
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

#[cfg(test)]
mod send_invariant {
    use std::marker::PhantomData;

    /// `Probe::<T>::IS_SEND` is the inherent `true` when `T: Send`, and
    /// falls back to the trait's `false` otherwise.
    struct Probe<T>(PhantomData<T>);
    trait NotSend {
        const IS_SEND: bool = false;
    }
    impl<T> NotSend for Probe<T> {}
    impl<T: Send> Probe<T> {
        const IS_SEND: bool = true;
    }

    #[test]
    fn this_crate_never_registers_a_converter_callback() {
        // A call, not a mention; spelled in two halves so this test does not
        // match itself.
        let method = [".add_callback", "_handler("].concat();
        let sources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut checked = 0;
        for entry in std::fs::read_dir(&sources).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                assert!(
                    !source.contains(&method),
                    "{} registers a callback",
                    path.display()
                );
                checked += 1;
            }
        }
        assert!(checked >= 3, "found only {checked} source files");
    }

    #[test]
    fn the_converters_other_fields_are_send() {
        // The field types of reis 0.7.1's `EiEventConverter`, apart from the
        // callback map.
        const { assert!(Probe::<reis::ei::Seat>::IS_SEND) };
        const { assert!(Probe::<reis::ei::Device>::IS_SEND) };
        const { assert!(Probe::<reis::ei::Callback>::IS_SEND) };
        const { assert!(Probe::<reis::Object>::IS_SEND) };
        const { assert!(Probe::<reis::event::Seat>::IS_SEND) };
        const { assert!(Probe::<reis::event::Device>::IS_SEND) };
        const { assert!(Probe::<reis::event::EiEvent>::IS_SEND) };
        const { assert!(Probe::<reis::event::Connection>::IS_SEND) };
        const { assert!(Probe::<reis::ei::Context>::IS_SEND) };
    }

    #[test]
    fn the_converter_still_needs_the_manual_impl() {
        const {
            assert!(
                !Probe::<reis::event::EiEventConverter>::IS_SEND,
                "reis made EiEventConverter Send; remove `unsafe impl Send for EventPump`"
            )
        };
    }
}
