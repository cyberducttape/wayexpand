//! One reactor turn of input for each capture source. Each step either
//! asks for the next turn or stops the daemon; shared maintenance, status,
//! and output recovery run before it in `main`.

use crate::input_loop::{
    attach_key_pass_through, check_input_method_policy, key_pass_through_failure,
};
use crate::output_connect::{BackgroundConnect, ConnectRequest};
use crate::*;

/// What the reactor loop does after an input step.
pub(crate) enum Step {
    /// Start the next turn.
    Next,
    /// Leave the loop and shut down.
    Stop,
}

impl Daemon {
    /// Wait out a scheduled input reconnect on the waker rather than in a
    /// sleep, so stop, reload, and control requests keep their turns.
    /// Returns whether the retry is still pending.
    fn waiting_for_input_retry(&mut self, poll_interval: Duration) -> bool {
        let Some(deadline) = self.input_retry_at else {
            return false;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.input_retry_at = None;
            return false;
        }
        self.waker.wait(remaining.min(poll_interval));
        true
    }

    /// Schedule the next input reconnect with exponential backoff.
    fn schedule_input_retry(&mut self) {
        self.input_retry_at = Some(Instant::now() + self.reconnect_delay);
        self.reconnect_delay = next_retry_delay(self.reconnect_delay);
    }

    /// Reconnect the input-method source if needed, then wait for and process one event.
    pub(crate) fn input_method_step(&mut self, poll_interval: Duration) -> Result<Step> {
        if self.input_method.is_none() {
            // The libei key pass-through goes through the portal, which can
            // take up to 30 seconds, so it connects on a worker and this step
            // only starts the attempt or collects its result.
            let connected = if let Some(connect) = &self.input_connect {
                let Some(result) = connect.poll() else {
                    self.waker.wait(poll_interval);
                    return Ok(Step::Next);
                };
                self.input_connect = None;
                let healthy = self.config.healthy();
                result
                    .map_err(|error| {
                        key_pass_through_failure(&self.control, &self.path, healthy, error)
                    })
                    .and_then(|key_injector| {
                        attach_key_pass_through(&self.control, &self.path, healthy, key_injector)
                    })
            } else {
                if self.waiting_for_input_retry(poll_interval) {
                    return Ok(Step::Next);
                }
                if let Err(error) = check_input_method_policy(&self.policy) {
                    return Err(anyhow::anyhow!(
                        "input-method reconnect failed permanently: {error}"
                    ));
                }
                self.input_connect = Some(BackgroundConnect::start(
                    ConnectRequest {
                        backend: "libei",
                        persist_portal_token: self.config.engine.libei_token_persistence(),
                        fcitx5_direct_commit: false,
                        portal_token_path: self.portal_token_path.clone(),
                    },
                    self.waker.clone(),
                ));
                return Ok(Step::Next);
            };
            match connected {
                Ok(source) => {
                    let mut source = source;
                    source.set_wake_fd(Some(self.waker.fd()));
                    self.input_method = Some(source);
                    self.reconnect_delay = Duration::from_millis(250);
                    self.connection_state = "connected";
                    set_daemon_status(
                        &mut self.status_publisher,
                        &self.control,
                        self.active_source,
                        self.active_backend,
                        self.connection_state,
                        &self.path,
                        self.config.healthy(),
                    );
                    crate::counters::note_input_reconnect();
                    info!("input-method source reconnected");
                }
                Err(error) if error.is_retryable() => {
                    warn!(%error, "input-method unavailable; retrying");
                    self.schedule_input_retry();
                }
                Err(error) => {
                    return Err(anyhow::anyhow!(
                        "input-method reconnect failed permanently: {error}"
                    ));
                }
            }
            return Ok(Step::Next);
        }
        let Some(source) = self.input_method.as_mut() else {
            return Err(anyhow::anyhow!("input-method mode lost its input source"));
        };
        let event_result = source.next_event_timeout(poll_interval);
        match event_result {
            Ok(Some(event)) => {
                drain_pending_window_events(
                    &self.window_tracker,
                    &mut self.config.engine,
                    &self.policy,
                    self.active_backend,
                    &self.control,
                    &mut self.focus_state,
                )?;
                let result = match self.input_method.as_mut() {
                    Some(source) => process_event(
                        &mut self.config.engine,
                        event,
                        Some(source),
                        &self.policy,
                        self.active_backend,
                    ),
                    None => {
                        return Err(anyhow::anyhow!(
                            "input-method source disappeared while processing an event"
                        ));
                    }
                };
                match result {
                    Ok(()) => self.reconnect_delay = Duration::from_millis(250),
                    Err(error) if error.expansion_rejected() => {
                        self.note_unsupported_output(&error);
                        warn!(
                            error = %error,
                            trigger_chars = error.result.trigger.chars().count(),
                            insert_bytes = error.result.insert.len(),
                            "input-method rejected expansion; continuing"
                        );
                        self.reconnect_delay = Duration::from_millis(250);
                    }
                    Err(error) if error.retryable() => {
                        warn!(
                            error = %error,
                            trigger_chars = error.result.trigger.chars().count(),
                            insert_bytes = error.result.insert.len(),
                            "input-method output failed; current expansion is not replayed"
                        );
                        self.input_method = None;
                        self.connection_state = "reconnecting";
                        process_event(
                            &mut self.config.engine,
                            InputEvent::FocusChanged { sensitive: true },
                            None,
                            &self.policy,
                            self.active_backend,
                        )?;
                        set_daemon_status(
                            &mut self.status_publisher,
                            &self.control,
                            self.active_source,
                            self.active_backend,
                            self.connection_state,
                            &self.path,
                            self.config.healthy(),
                        );
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(None) => {}
            Err(error) if error.retryable => {
                warn!(%error, "input-method connection lost; reconnecting");
                self.input_method = None;
                self.connection_state = "reconnecting";
                process_event(
                    &mut self.config.engine,
                    InputEvent::FocusChanged { sensitive: true },
                    None,
                    &self.policy,
                    self.active_backend,
                )?;
                set_daemon_status(
                    &mut self.status_publisher,
                    &self.control,
                    self.active_source,
                    self.active_backend,
                    self.connection_state,
                    &self.path,
                    self.config.healthy(),
                );
            }
            Err(error) => {
                return Err(anyhow::anyhow!("input source failed: {error}"));
            }
        }
        Ok(Step::Next)
    }

    /// Reconnect the evdev source if needed, then wait for and process one event.
    pub(crate) fn evdev_step(&mut self, poll_interval: Duration) -> Result<Step> {
        if self.evdev.is_none() {
            if self.waiting_for_input_retry(poll_interval) {
                return Ok(Step::Next);
            }
            match EvdevSource::connect() {
                Ok(source) => {
                    let mut source = source;
                    source.set_wake_fd(Some(self.waker.fd()));
                    self.evdev = Some(source);
                    self.reconnect_delay = Duration::from_millis(250);
                    self.connection_state = "connected";
                    set_daemon_status(
                        &mut self.status_publisher,
                        &self.control,
                        self.active_source,
                        self.active_backend,
                        self.connection_state,
                        &self.path,
                        self.config.healthy(),
                    );
                    crate::counters::note_input_reconnect();
                    info!("evdev source reconnected");
                }
                Err(error) if error.is_retryable() => {
                    warn!(%error, "evdev source unavailable; retrying");
                    self.schedule_input_retry();
                }
                Err(error) => {
                    return Err(anyhow::anyhow!(
                        "evdev reconnect failed permanently: {error}"
                    ));
                }
            }
            return Ok(Step::Next);
        }
        let Some(source) = self.evdev.as_mut() else {
            return Err(anyhow::anyhow!("evdev mode lost its input source"));
        };
        let event_result = source.next_event_timeout(poll_interval);
        match event_result {
            Ok(Some(event)) => {
                drain_pending_window_events(
                    &self.window_tracker,
                    &mut self.config.engine,
                    &self.policy,
                    self.active_backend,
                    &self.control,
                    &mut self.focus_state,
                )?;
                if self.serialized_output() {
                    self.start_evdev_output(event)?;
                    return Ok(Step::Next);
                }
                let result = if let Some(mut backend) = self.injector.take() {
                    // Match immediately. The release/quiet gates are only
                    // needed if the matcher actually produced text that
                    // will modify the focused application.
                    let result = if matches!(event, InputEvent::Key(_)) {
                        process_event(
                            &mut self.config.engine,
                            event,
                            Some(backend.as_mut()),
                            &self.policy,
                            self.active_backend,
                        )
                    } else {
                        // The outcome is a value: the injector is restored
                        // below and failures reach the recovery match, never
                        // an early return out of the event loop.
                        apply_deferred_evdev_event(
                            &mut self.config.engine,
                            event,
                            backend.as_mut(),
                            &mut self.evdev,
                            &self.policy,
                            self.active_backend,
                        )
                    };
                    self.injector = Some(backend);
                    result
                } else {
                    process_event(
                        &mut self.config.engine,
                        event,
                        None,
                        &self.policy,
                        self.active_backend,
                    )
                };
                match result {
                    Ok(()) => self.reconnect_delay = Duration::from_millis(250),
                    Err(error) => {
                        if error.expansion_rejected() {
                            self.reconnect_delay = Duration::from_millis(250);
                        }
                        self.recover_from_output_error(*error)?;
                    }
                }
            }
            Ok(None) => {}
            Err(error) if error.retryable => {
                warn!(%error, "evdev connection lost; reconnecting");
                self.evdev = None;
                self.connection_state = "reconnecting";
                let _ = process_event(
                    &mut self.config.engine,
                    InputEvent::EndOfInput,
                    None,
                    &self.policy,
                    self.active_backend,
                );
                set_daemon_status(
                    &mut self.status_publisher,
                    &self.control,
                    self.active_source,
                    self.active_backend,
                    self.connection_state,
                    &self.path,
                    self.config.healthy(),
                );
            }
            Err(error) => {
                return Err(anyhow::anyhow!("input source failed: {error}"));
            }
        }
        Ok(Step::Next)
    }

    /// Wait for and process one line from the stdin input stream.
    pub(crate) fn stdin_step(&mut self, poll_interval: Duration) -> Result<Step> {
        if self.stdin_closed {
            thread::sleep(poll_interval);
            return Ok(Step::Next);
        }
        let Some(lines) = self.receiver.as_ref() else {
            return Ok(Step::Stop);
        };
        match lines.recv_timeout(poll_interval) {
            Ok(line) => {
                drain_pending_window_events(
                    &self.window_tracker,
                    &mut self.config.engine,
                    &self.policy,
                    self.active_backend,
                    &self.control,
                    &mut self.focus_state,
                )?;
                if self.injector.is_some() {
                    for character in line.chars() {
                        let event = InputEvent::Text(character.to_string());
                        let (result, backend) = if let Some(mut backend) = self.injector.take() {
                            let result = process_event(
                                &mut self.config.engine,
                                event,
                                Some(backend.as_mut()),
                                &self.policy,
                                self.active_backend,
                            );
                            (result, Some(backend))
                        } else {
                            (
                                process_event(
                                    &mut self.config.engine,
                                    event,
                                    None,
                                    &self.policy,
                                    self.active_backend,
                                ),
                                None,
                            )
                        };
                        self.injector = backend;
                        if let Err(error) = result {
                            if error.expansion_rejected() {
                                warn!(
                                    error = %error,
                                    trigger_chars = error.result.trigger.chars().count(),
                                    insert_bytes = error.result.insert.len(),
                                    "output backend rejected expansion; continuing"
                                );
                                continue;
                            }
                            if !error.retryable() {
                                return Err(error.into());
                            }
                            warn!(
                                error = %error,
                                trigger_chars = error.result.trigger.chars().count(),
                                insert_bytes = error.result.insert.len(),
                                "output session failed; current expansion is not replayed"
                            );
                            self.retire_output_injector();
                            let _ = process_event(
                                &mut self.config.engine,
                                InputEvent::EndOfInput,
                                None,
                                &self.policy,
                                self.active_backend,
                            );
                            self.connection_state = "reconnecting";
                            set_daemon_status(
                                &mut self.status_publisher,
                                &self.control,
                                self.active_source,
                                wayexpand_backend_selection::injection_status_label(
                                    self.backend_name,
                                ),
                                self.connection_state,
                                &self.path,
                                self.config.healthy(),
                            );
                            self.output_retry_at = Some(Instant::now());
                            self.output_retry_delay = Duration::from_millis(250);
                        }
                    }
                    if let Some(backend) = self.injector.as_deref_mut() {
                        process_event(
                            &mut self.config.engine,
                            InputEvent::EndOfInput,
                            Some(backend),
                            &self.policy,
                            self.active_backend,
                        )?;
                    }
                } else {
                    process_event(
                        &mut self.config.engine,
                        InputEvent::Text(line),
                        None,
                        &self.policy,
                        self.active_backend,
                    )?;
                    process_event(
                        &mut self.config.engine,
                        InputEvent::EndOfInput,
                        None,
                        &self.policy,
                        self.active_backend,
                    )?;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(Step::Next),
            Err(mpsc::RecvTimeoutError::Disconnected) if self.managed => {
                warn!("stdin input source ended; daemon remains idle under control socket");
                self.stdin_closed = true;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Step::Stop),
        }
        Ok(Step::Next)
    }
}
