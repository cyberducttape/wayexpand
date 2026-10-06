//! One reactor turn of input for each capture source. Each step either
//! asks for the next turn or stops the daemon; shared maintenance, status,
//! and output recovery run before it in `main`.

use crate::*;

/// What the reactor loop does after an input step.
pub(crate) enum Step {
    /// Start the next turn.
    Next,
    /// Leave the loop and shut down.
    Stop,
}

impl Daemon {
    /// Reconnect the input-method source if needed, then wait for and process one event.
    pub(crate) fn input_method_step(&mut self, poll_interval: Duration) -> Result<Step> {
        if self.input_method.is_none() {
            match connect_input_method_session(
                &self.control,
                &self.path,
                self.config.healthy(),
                self.config.engine.libei_token_persistence(),
                self.portal_token_path.as_deref(),
                &self.policy,
            ) {
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
                    info!("input-method source reconnected");
                }
                Err(error) if error.is_retryable() => {
                    warn!(%error, "input-method unavailable; retrying");
                    if !wait_for_retry(&self.control.stop_requested, self.reconnect_delay) {
                        return Ok(Step::Stop);
                    }
                    self.reconnect_delay = next_retry_delay(self.reconnect_delay);
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
                    info!("evdev source reconnected");
                }
                Err(error) if error.is_retryable() => {
                    warn!(%error, "evdev source unavailable; retrying");
                    if !wait_for_retry(&self.control.stop_requested, self.reconnect_delay) {
                        return Ok(Step::Stop);
                    }
                    self.reconnect_delay = next_retry_delay(self.reconnect_delay);
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
                        let pending = self.config.engine.process_deferred(event);
                        let results = dispatch_pending_results(
                            &mut self.config.engine,
                            pending,
                            &self.policy,
                            self.active_backend,
                        );
                        if results.is_empty() {
                            Ok(())
                        } else {
                            let gating = apply_evdev_gating(results, &mut self.evdev);
                            restore_abandoned_results(&mut self.config.engine, gating.abandoned);
                            apply_results(
                                &mut self.config.engine,
                                gating.results,
                                Some(backend.as_mut()),
                                &self.policy,
                                self.active_backend,
                            )?;
                            replay_evdev_follow_up(
                                &mut self.config.engine,
                                gating.follow_up,
                                &self.policy,
                                self.active_backend,
                            )
                        }
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
                    Err(error) if error.expansion_rejected() => {
                        warn!(
                            error = %error,
                            trigger_chars = error.result.trigger.chars().count(),
                            insert_bytes = error.result.insert.len(),
                            "evdev rejected expansion; continuing"
                        );
                        self.reconnect_delay = Duration::from_millis(250);
                    }
                    Err(error) if error.retryable() => {
                        warn!(
                            error = %error,
                            trigger_chars = error.result.trigger.chars().count(),
                            insert_bytes = error.result.insert.len(),
                            "evdev output failed; current expansion is not replayed"
                        );
                        drop(self.injector.take());
                        process_event(
                            &mut self.config.engine,
                            InputEvent::EndOfInput,
                            None,
                            &self.policy,
                            self.active_backend,
                        )?;
                        self.connection_state = "reconnecting";
                        set_daemon_status(
                            &mut self.status_publisher,
                            &self.control,
                            self.active_source,
                            self.active_backend,
                            self.connection_state,
                            &self.path,
                            self.config.healthy(),
                        );
                        self.output_retry_at = Some(Instant::now());
                        self.output_retry_delay = Duration::from_millis(250);
                    }
                    Err(error) => return Err(error.into()),
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
                            drop(self.injector.take());
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
