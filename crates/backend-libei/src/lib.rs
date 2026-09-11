//! Output backend using the libei/EIS protocol.
//!
//! This backend uses the `ei_text` interface, so insertion is UTF-8 rather
//! than keyboard-layout-dependent key synthesis. It accepts a direct
//! `LIBEI_SOCKET` or the XDG RemoteDesktop portal, but portal access is only
//! attempted when this backend is explicitly selected.

use reis::{ei, enumflags2::BitFlags, event::DeviceCapability};
use std::{
    os::unix::net::UnixStream,
    path::PathBuf,
    time::{Duration, Instant},
};
use thiserror::Error;
use wayexpand_core::{InjectorError, TextInjector};

const BACKEND_NAME: &str = "libei";
const KEY_BACKSPACE: u32 = 14;
const EI_TEXT_MAX_UTF8_BYTES: usize = 254;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const EIS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum LibeiError {
    #[error("relative LIBEI_SOCKET requires XDG_RUNTIME_DIR")]
    RelativeSocketNeedsRuntime,
    #[error("could not connect to EIS socket: {0}")]
    Connect(#[from] std::io::Error),
    #[error("portal connection failed: {0}")]
    Portal(String),
    #[error("libei handshake failed: {0}")]
    Handshake(#[from] reis::Error),
    #[error("libei server disconnected: {0}")]
    Disconnected(String),
    #[error("libei connection flush failed: {0}")]
    Flush(String),
    #[error("EIS server did not provide a device with ei_text and ei_keyboard")]
    MissingRequiredDevice,
    #[error("text is {length} bytes; maximum is {maximum}")]
    TextTooLarge { length: usize, maximum: usize },
}

impl LibeiError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Connect(error) => matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::AddrNotAvailable
                    | std::io::ErrorKind::BrokenPipe
            ),
            Self::Disconnected(_) | Self::Flush(_) => true,
            Self::Handshake(reis::Error::Io(_)) => true,
            _ => false,
        }
    }
}

struct PortalKeepalive {
    _proxy: ashpd::desktop::remote_desktop::RemoteDesktop<'static>,
    _session:
        ashpd::desktop::Session<'static, ashpd::desktop::remote_desktop::RemoteDesktop<'static>>,
    // Keep the Tokio reactor alive until the portal proxies have been dropped.
    _runtime: tokio::runtime::Runtime,
}

pub struct LibeiInjector {
    connection: reis::event::Connection,
    device: reis::event::Device,
    text: ei::Text,
    keyboard: ei::Keyboard,
    sequence: u32,
    started_at: Instant,
    _portal: Option<PortalKeepalive>,
}

struct EventPump {
    context: ei::Context,
    converter: reis::event::EiEventConverter,
}

impl EventPump {
    fn next(&mut self, timeout: Duration) -> Result<reis::event::EiEvent, LibeiError> {
        let deadline = Instant::now() + timeout;
        loop {
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
}

fn poll_context(context: &ei::Context, timeout: Duration) -> Result<bool, LibeiError> {
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

fn handshake_with_timeout(
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

impl LibeiInjector {
    /// Connect to a direct EIS socket from `LIBEI_SOCKET`, or use the XDG
    /// RemoteDesktop portal when that variable is absent.
    ///
    /// Portal use is deliberately explicit because it may display a consent
    /// dialog and grants desktop input-control capability for the session.
    pub fn connect() -> Result<Self, LibeiError> {
        let (stream, portal) = if let Some(socket) = std::env::var_os("LIBEI_SOCKET") {
            let socket = PathBuf::from(socket);
            let socket = if socket.is_relative() {
                let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                    .ok_or(LibeiError::RelativeSocketNeedsRuntime)?;
                PathBuf::from(runtime).join(socket)
            } else {
                socket
            };
            (UnixStream::connect(socket)?, None)
        } else {
            connect_portal()?
        };
        // Handshake and event polling use explicit deadlines. Keep later
        // protocol flushes from blocking indefinitely when an EIS server
        // stops consuming input; WouldBlock is classified as retryable.
        stream.set_nonblocking(true)?;
        let context = ei::Context::new(stream)?;
        let (connection, mut events) = handshake_with_timeout(&context, EIS_HANDSHAKE_TIMEOUT)?;

        let device_deadline = Instant::now() + EIS_HANDSHAKE_TIMEOUT;
        let (device, text, keyboard) = loop {
            let event = match events.next(device_deadline.saturating_duration_since(Instant::now()))
            {
                Ok(event) => event,
                Err(LibeiError::Handshake(reis::Error::Io(error)))
                    if error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Err(LibeiError::MissingRequiredDevice)
                }
                Err(error) => return Err(error),
            };
            match event {
                reis::event::EiEvent::SeatAdded(seat) => {
                    seat.seat.bind_capabilities(
                        BitFlags::from(DeviceCapability::Text)
                            | BitFlags::from(DeviceCapability::Keyboard),
                    );
                    connection
                        .flush()
                        .map_err(|error| LibeiError::Flush(error.to_string()))?;
                }
                reis::event::EiEvent::DeviceResumed(resumed) => {
                    if let (Some(text), Some(keyboard)) = (
                        resumed.device.interface::<ei::Text>(),
                        resumed.device.interface::<ei::Keyboard>(),
                    ) {
                        break (resumed.device, text, keyboard);
                    }
                }
                reis::event::EiEvent::Disconnected(disconnected) => {
                    return Err(LibeiError::Disconnected(
                        disconnected
                            .explanation
                            .unwrap_or_else(|| "no explanation".into()),
                    ));
                }
                _ => {}
            }
        };

        Ok(Self {
            connection,
            device,
            text,
            keyboard,
            sequence: 1,
            started_at: Instant::now(),
            _portal: portal,
        })
    }

    fn send_text_unflushed(&mut self, text: &str) {
        for chunk in split_text_chunks(text) {
            let serial = self.connection.serial();
            self.device.device().start_emulating(serial, self.sequence);
            self.sequence = self.sequence.checked_add(1).unwrap_or(1);
            self.text.utf8(chunk);
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.device.device().stop_emulating(serial);
        }
    }

    fn send_text(&mut self, text: &str) -> Result<(), LibeiError> {
        validate_text(text)?;
        self.send_text_unflushed(text);
        if !text.is_empty() {
            self.connection
                .flush()
                .map_err(|error| LibeiError::Flush(error.to_string()))?;
        }
        Ok(())
    }

    fn send_backspaces_unflushed(&mut self, chars: usize) {
        let serial = self.connection.serial();
        for _ in 0..chars {
            self.device.device().start_emulating(serial, self.sequence);
            self.sequence = self.sequence.checked_add(1).unwrap_or(1);
            // EI keyboard keycodes are Linux evdev codes. KEY_BACKSPACE is 14.
            self.keyboard
                .key(KEY_BACKSPACE, ei::keyboard::KeyState::Press);
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.keyboard
                .key(KEY_BACKSPACE, ei::keyboard::KeyState::Released);
            self.device
                .device()
                .frame(serial, self.started_at.elapsed().as_micros() as u64);
            self.device.device().stop_emulating(serial);
        }
    }

    fn send_backspaces(&mut self, chars: usize) -> Result<(), LibeiError> {
        if chars == 0 {
            return Ok(());
        }
        self.send_backspaces_unflushed(chars);
        self.connection
            .flush()
            .map_err(|error| LibeiError::Flush(error.to_string()))
    }
}

fn split_text_chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = start;
        for (offset, character) in text[start..].char_indices() {
            let candidate = start + offset + character.len_utf8();
            if candidate - start > EI_TEXT_MAX_UTF8_BYTES {
                break;
            }
            end = candidate;
        }
        debug_assert!(end > start, "a Unicode scalar must fit in an EI text chunk");
        chunks.push(&text[start..end]);
        start = end;
    }
    chunks
}

fn connect_portal() -> Result<(UnixStream, Option<PortalKeepalive>), LibeiError> {
    use ashpd::desktop::{
        remote_desktop::{DeviceType, RemoteDesktop},
        PersistMode,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| LibeiError::Portal(error.to_string()))?;
    let (stream, proxy, session) = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), async {
            let proxy: RemoteDesktop<'static> = RemoteDesktop::new()
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;
            let session = proxy
                .create_session()
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;
            proxy
                .select_devices(
                    &session,
                    DeviceType::Keyboard.into(),
                    None,
                    PersistMode::DoNot,
                )
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;
            proxy
                .start(&session, None)
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?
                .response()
                .map_err(|error| LibeiError::Portal(error.to_string()))?;
            let fd = proxy
                .connect_to_eis(&session)
                .await
                .map_err(|error| LibeiError::Portal(error.to_string()))?;
            Ok::<_, LibeiError>((UnixStream::from(fd), proxy, session))
        })
        .await
        .map_err(|_| LibeiError::Portal("portal session timed out after 30 seconds".into()))?
    })?;
    Ok((
        stream,
        Some(PortalKeepalive {
            _proxy: proxy,
            _session: session,
            _runtime: runtime,
        }),
    ))
}

impl TextInjector for LibeiInjector {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.send_backspaces(trigger.chars().count())
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.send_text(text).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        validate_text(text).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })?;
        self.send_backspaces_unflushed(trigger.chars().count());
        self.send_text_unflushed(text);
        if !trigger.is_empty() || !text.is_empty() {
            self.connection.flush().map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: LibeiError::Flush(error.to_string()).to_string(),
                retryable: true,
            })?;
        }
        Ok(())
    }
}

fn validate_text(text: &str) -> Result<(), LibeiError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(LibeiError::TextTooLarge {
            length: text.len(),
            maximum: MAX_TEXT_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{os::unix::net::UnixStream, time::Duration};

    #[test]
    fn backend_name_is_stable() {
        assert_eq!(super::BACKEND_NAME, "libei");
        assert_eq!(super::KEY_BACKSPACE, 14);
        assert_eq!(super::EIS_HANDSHAKE_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn event_poll_has_a_bounded_deadline() {
        let (client, _server) = UnixStream::pair().unwrap();
        let context = super::ei::Context::new(client).unwrap();
        assert!(!super::poll_context(&context, Duration::from_millis(10)).unwrap());
    }

    #[test]
    fn handshake_has_a_bounded_deadline() {
        let (client, _server) = UnixStream::pair().unwrap();
        let context = super::ei::Context::new(client).unwrap();
        let error = match super::handshake_with_timeout(&context, Duration::from_millis(10)) {
            Err(error) => error,
            Ok(_) => panic!("idle EIS endpoint must time out"),
        };
        assert!(matches!(error, super::LibeiError::Handshake(_)));
        assert!(error.to_string().contains("deadline expired"));
    }

    #[test]
    fn text_chunks_are_utf8_safe_and_protocol_sized() {
        let text = "🙂".repeat(200);
        let chunks = super::split_text_chunks(&text);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| {
            chunk.len() <= super::EI_TEXT_MAX_UTF8_BYTES
                && std::str::from_utf8(chunk.as_bytes()).is_ok()
        }));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn empty_text_has_no_protocol_chunk() {
        assert!(super::split_text_chunks("").is_empty());
    }

    #[test]
    fn text_size_is_bounded_before_injection() {
        assert!(super::validate_text(&"a".repeat(super::MAX_TEXT_BYTES)).is_ok());
        assert!(matches!(
            super::validate_text(&"a".repeat(super::MAX_TEXT_BYTES + 1)),
            Err(super::LibeiError::TextTooLarge { .. })
        ));
    }

    #[test]
    fn transport_failures_are_retryable_but_validation_is_not() {
        assert!(super::LibeiError::Connect(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "not ready",
        ))
        .is_retryable());
        assert!(!super::LibeiError::Connect(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "not allowed",
        ))
        .is_retryable());
        assert!(super::LibeiError::Disconnected("closed".into()).is_retryable());
        assert!(super::LibeiError::Flush("closed".into()).is_retryable());
        assert!(
            super::LibeiError::Handshake(reis::Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timeout"
            ),))
            .is_retryable()
        );
        assert!(!super::LibeiError::Handshake(reis::Error::Handshake(
            reis::handshake::HandshakeError::MissingInterface,
        ))
        .is_retryable());
        assert!(!super::LibeiError::MissingRequiredDevice.is_retryable());
        assert!(!super::LibeiError::Portal("permission denied".into()).is_retryable());
        assert!(!super::LibeiError::TextTooLarge {
            length: 2,
            maximum: 1,
        }
        .is_retryable());
    }
}
