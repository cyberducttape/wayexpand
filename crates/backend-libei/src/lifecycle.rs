//! EIS connection, handshake, and initial device negotiation.

use super::*;

impl LibeiSession {
    /// Connect to EIS from the caller's dedicated session thread. The
    /// resulting session must stay on that thread for its entire lifetime.
    pub(super) fn connect_with_cancellation(
        options: LibeiOptions,
        cancellation: ConnectCancellation,
    ) -> Result<Self, LibeiError> {
        if cancellation.is_cancelled() {
            return Err(LibeiError::Cancelled);
        }
        let fcitx5_direct_commit = options.fcitx5_direct_commit;
        let (stream, portal) = if let Some(socket) = std::env::var_os("LIBEI_SOCKET") {
            let socket = PathBuf::from(socket);
            let socket = if socket.is_relative() {
                let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                    .ok_or(LibeiError::RelativeSocketNeedsRuntime)?;
                PathBuf::from(runtime).join(socket)
            } else {
                socket
            };
            (
                super::connection::connect_socket_with_cancellation(socket, &cancellation)?,
                None,
            )
        } else {
            connect_portal(options, &cancellation)?
        };
        if cancellation.is_cancelled() {
            drop(portal);
            return Err(LibeiError::Cancelled);
        }
        // Handshake and event polling use explicit deadlines. Keep later
        // protocol flushes from blocking indefinitely when an EIS server
        // stops consuming input; WouldBlock is classified as retryable.
        stream.set_nonblocking(true)?;
        let context = ei::Context::new(stream)?;
        let (connection, mut events) =
            handshake_with_cancellation(&context, EIS_HANDSHAKE_TIMEOUT, &cancellation)?;

        let device_deadline = Instant::now() + EIS_HANDSHAKE_TIMEOUT;
        let (device, mode, keyboard) = loop {
            if cancellation.is_cancelled() {
                return Err(LibeiError::Cancelled);
            }
            let remaining = device_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(LibeiError::MissingRequiredDevice);
            }
            let event = match events.next_with_cancellation(
                remaining.min(Duration::from_millis(50)),
                Some(&cancellation),
            ) {
                Ok(event) => event,
                Err(LibeiError::Handshake(reis::Error::Io(error)))
                    if error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
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
                    let Some(keyboard) = resumed.device.interface::<ei::Keyboard>() else {
                        continue;
                    };
                    if let Some(text) = resumed.device.interface::<ei::Text>() {
                        break (resumed.device, TextMode::Text(text), keyboard);
                    }
                    // No ei_text on this device: fall back to keysym
                    // synthesis if the server also gave us a keymap to
                    // synthesize against. A keyboard device without either
                    // is not usable and keeps waiting for another device.
                    let Some(keymap) = resumed.device.keymap() else {
                        continue;
                    };
                    let keymap_fd = rustix::io::dup(&keymap.fd)
                        .map_err(|error| LibeiError::Keymap(error.to_string()))?;
                    let xkb_keymap = decode_keymap(keymap_fd, keymap.size)?;
                    let typer = KeysymTyper::build(&xkb_keymap)?;
                    break (resumed.device, TextMode::Keysym(Box::new(typer)), keyboard);
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

        // Extract control keycodes from keymap if available (for Text mode to use)
        let (return_keycode, tab_keycode) = match &mode {
            TextMode::Keysym(typer) => (typer.return_keycode, typer.tab_keycode),
            TextMode::Text(_) => {
                // Try to get them from the device's keymap if available
                if let Some(keymap_data) = device.keymap() {
                    let keymap_fd = rustix::io::dup(&keymap_data.fd)
                        .map_err(|error| LibeiError::Keymap(error.to_string()))?;
                    let xkb_keymap = decode_keymap(keymap_fd, keymap_data.size)?;
                    let return_code = find_keycode_for_keysym(&xkb_keymap, xkeysym::key::Return)
                        .ok_or_else(|| LibeiError::Keymap("no Return key in keymap".into()))?;
                    let tab_code = find_keycode_for_keysym(&xkb_keymap, xkeysym::key::Tab)
                        .ok_or_else(|| LibeiError::Keymap("no Tab key in keymap".into()))?;
                    (return_code, tab_code)
                } else {
                    // Fallback: use standard Linux evdev keycodes
                    // KEY_RETURN = 28, KEY_TAB = 15
                    (28, 15)
                }
            }
        };

        Ok(Self {
            connection,
            events,
            device_unavailable: None,
            device,
            mode,
            keyboard,
            sequence: 1,
            started_at: Instant::now(),
            return_keycode,
            tab_keycode,
            _portal: portal,
            fcitx5: fcitx5_direct_commit.then(fcitx5::Client::connect).flatten(),
            checkpoint: wayexpand_core::OutputCheckpoint::default(),
        })
    }
}
