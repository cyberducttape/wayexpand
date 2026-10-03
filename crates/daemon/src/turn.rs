//! Work done at the start of every reactor turn, before the capture
//! source's input step: control requests, reloads, hotkey and command
//! completions, picker inserts, status publication, and output recovery.

use std::ops::ControlFlow;

use crate::*;
use wayexpand_core::CheckStatus;

impl Daemon {
    /// Apply window, pause, reload, and stop transitions; report hotkey results and worker failures. Breaks when the daemon should stop.
    pub(crate) fn maintain(&mut self) -> Result<ControlFlow<(), CommandMetrics>> {
        drain_pending_window_events(
            &self.window_tracker,
            &mut self.config.engine,
            &self.policy,
            self.active_backend,
            &self.control,
            &mut self.focus_state,
        )?;
        let transition = reactor::ReactorTransition::sample(
            &self.control.stop_requested,
            &self.control.pause_requested,
            &self.control.reload_requested,
            self.paused,
        );
        if let Some(requested_pause) = transition.pause_changed() {
            process_event(
                &mut self.config.engine,
                InputEvent::PauseChanged(requested_pause),
                None,
                &self.policy,
                self.active_backend,
            )?;
            self.paused = requested_pause;
            info!(self.paused, "expansion processing policy changed");
        }
        if transition.reload_requested() {
            self.config.reload_now();
        }
        if transition.should_stop() {
            return Ok(ControlFlow::Break(()));
        }
        self.config.reload_if_changed();
        for (action, result) in self.config.engine.drain_completed_hotkeys() {
            match result {
                Ok(()) => info!(chord = %action.chord, "hotkey action completed"),
                Err(error) => warn!(chord = %action.chord, %error, "hotkey action failed"),
            }
        }
        let metrics = self.config.engine.command_metrics();
        let worker_failure = self
            .output_failures
            .as_ref()
            .and_then(|failures| failures.try_recv().ok());
        if let Some(failure) = worker_failure {
            warn!(
                retryable = failure.retryable,
                error = %failure.message,
                "serialized output worker failed"
            );
            drop(self.injector.take());
            process_event(
                &mut self.config.engine,
                InputEvent::EndOfInput,
                None,
                &self.policy,
                self.active_backend,
            )?;
            self.output_failures = None;
            if !failure.retryable {
                return Err(anyhow::anyhow!(
                    "output backend failed permanently: {}",
                    failure.message
                ));
            }
            self.connection_state = "reconnecting";
            self.output_retry_at = Some(Instant::now());
            self.output_retry_delay = Duration::from_millis(250);
        }
        if metrics.command_queue_rejected_total < self.logged_queue_rejections {
            // A successful configuration reload creates a fresh engine and
            // therefore starts a fresh counter interval.
            self.logged_queue_rejections = 0;
        }
        if metrics.command_queue_rejected_total > self.logged_queue_rejections {
            warn!(
                command_queue_depth = metrics.command_queue_depth,
                command_queue_rejected_total = metrics.command_queue_rejected_total,
                "command action rejected because the command queue was full or unavailable"
            );
            self.logged_queue_rejections = metrics.command_queue_rejected_total;
        }
        Ok(ControlFlow::Continue(metrics))
    }

    /// Apply command-backed expansions whose commands have finished.
    pub(crate) fn apply_completed_commands(&mut self) -> Result<()> {
        let completed_commands = self.config.engine.drain_completed_commands();
        if !completed_commands.is_empty() {
            // Apply evdev safety gating: ensure physical key-up was processed
            // and no competing input arrived during command execution.
            // This prevents the race condition where fast commands finish
            // before the trigger key's physical release event is processed.
            let gating = if self.evdev_mode {
                apply_evdev_gating(completed_commands, &mut self.evdev)
            } else {
                EvdevGatingOutcome {
                    results: completed_commands,
                    follow_up: Vec::new(),
                    abandoned: Vec::new(),
                }
            };
            restore_abandoned_results(&mut self.config.engine, gating.abandoned);

            if !gating.results.is_empty() {
                if self.input_method_mode {
                    if let Some(source) = self.input_method.as_mut() {
                        apply_results(
                            &mut self.config.engine,
                            gating.results,
                            Some(source),
                            &self.policy,
                            self.active_backend,
                        )?;
                    }
                } else if let Some(mut backend) = self.injector.take() {
                    let result = apply_results(
                        &mut self.config.engine,
                        gating.results,
                        Some(backend.as_mut()),
                        &self.policy,
                        self.active_backend,
                    );
                    self.injector = Some(backend);
                    result?;
                } else {
                    apply_results(
                        &mut self.config.engine,
                        gating.results,
                        None,
                        &self.policy,
                        self.active_backend,
                    )?;
                }
            }
            replay_evdev_follow_up(
                &mut self.config.engine,
                gating.follow_up,
                &self.policy,
                self.active_backend,
            )?;
        }
        Ok(())
    }

    /// Handle a quick-insert request. Returns true to end the turn early, as
    /// happens when the request is refused because focus changed.
    pub(crate) fn handle_insert_request(&mut self) -> Result<bool> {
        if let Some(request) = self.control.take_insert_request() {
            // An explicit insert (quick-insert picker, `wayexpand insert`)
            // types a snippet at the cursor through the same injector and
            // evdev safety gate as a typed expansion. It is a user action,
            // so a refusal or injection failure is logged, never fatal.
            if let Some(expected_token) = request.focus_token.as_deref() {
                let snapshot = self.control.focus_snapshot();
                let current_token = self.config.engine.current_window().map(focus_token);
                if current_token.as_deref() != Some(expected_token)
                    || snapshot.token.as_deref() != Some(expected_token)
                    || snapshot.generation != request.focus_generation.unwrap_or_default()
                {
                    warn!(
                        expected = expected_token,
                        actual = ?current_token,
                        "requested snippet insert refused because focus changed"
                    );
                    return Ok(true);
                }
            }
            match self.config.engine.prepare_insert(&request.trigger) {
                Ok(result) => {
                    let gating = if self.evdev_mode {
                        apply_evdev_gating(vec![result], &mut self.evdev)
                    } else {
                        EvdevGatingOutcome {
                            results: vec![result],
                            follow_up: Vec::new(),
                            abandoned: Vec::new(),
                        }
                    };
                    if !gating.abandoned.is_empty() {
                        warn!("requested snippet insert abandoned because input arrived first");
                    }
                    let outcome = if gating.results.is_empty() {
                        Ok(())
                    } else if self.input_method_mode {
                        match self.input_method.as_mut() {
                            Some(source) => apply_results(
                                &mut self.config.engine,
                                gating.results,
                                Some(source),
                                &self.policy,
                                self.active_backend,
                            ),
                            None => {
                                warn!(
                                    "requested snippet insert skipped: input method reconnecting"
                                );
                                Ok(())
                            }
                        }
                    } else if let Some(mut backend) = self.injector.take() {
                        let outcome = apply_results(
                            &mut self.config.engine,
                            gating.results,
                            Some(backend.as_mut()),
                            &self.policy,
                            self.active_backend,
                        );
                        self.injector = Some(backend);
                        outcome
                    } else {
                        warn!("requested snippet insert skipped: no injection backend");
                        Ok(())
                    };
                    if let Err(error) = outcome {
                        warn!(%error, "requested snippet insert failed");
                    }
                    replay_evdev_follow_up(
                        &mut self.config.engine,
                        gating.follow_up,
                        &self.policy,
                        self.active_backend,
                    )?;
                }
                Err(error) => warn!(%error, "requested snippet insert refused"),
            }
        }
        Ok(false)
    }

    /// Publish the current status, including runtime capabilities.
    pub(crate) fn publish_status(&mut self, metrics: CommandMetrics) {
        set_daemon_status_with_runtime_capabilities(
            &mut self.status_publisher,
            &self.control,
            self.active_source,
            self.active_backend,
            self.connection_state,
            &self.path,
            self.config.healthy(),
            metrics,
            self.input_method
                .as_ref()
                .map(TextInjector::status_detail)
                .or_else(|| {
                    self.injector
                        .as_ref()
                        .map(|backend| backend.status_detail())
                })
                .filter(|detail| !detail.is_empty())
                .unwrap_or("unknown"),
            self.input_method
                .as_ref()
                .map(InputSource::capabilities)
                .or_else(|| self.evdev.as_ref().map(InputSource::capabilities))
                .unwrap_or_default(),
            self.input_method
                .as_ref()
                .map(TextInjector::capabilities)
                .or_else(|| self.injector.as_ref().map(|backend| backend.capabilities()))
                .unwrap_or_default(),
            self.window_tracker
                .as_ref()
                .is_some_and(backend_lifecycle::WindowTrackerHandle::is_connected),
        );
    }

    /// Make at most one output reconnect attempt per turn.
    pub(crate) fn recover_output(&mut self) -> Result<()> {
        // Output recovery is deliberately one attempt per reactor turn. A
        // portal or compositor outage must not park control, reload, status,
        // or shutdown handling inside an exponential-backoff sleep.
        if self.injector.is_none()
            && !self.input_method_mode
            && self.backend_name != "none"
            && self
                .output_retry_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            match connect_output_backend(
                self.backend_name,
                self.config.engine.libei_token_persistence(),
                self.portal_token_path.as_deref(),
            ) {
                Ok(backend) => {
                    if self.evdev_mode && self.backend_name == "libei" {
                        let (backend, failures) = spawn_async_injector(backend);
                        self.injector = Some(backend);
                        self.output_failures = Some(failures);
                    } else {
                        self.injector = Some(backend);
                    }
                    self.output_retry_at = None;
                    self.output_retry_delay = Duration::from_millis(250);
                    self.connection_state = "connected";
                    set_daemon_status(
                        &mut self.status_publisher,
                        &self.control,
                        self.active_source,
                        self.backend_name,
                        self.connection_state,
                        &self.path,
                        self.config.healthy(),
                    );
                    info!(backend = self.backend_name, "output backend reconnected");
                }
                Err(error) if error.retryable => {
                    self.output_retry_at = Some(Instant::now() + self.output_retry_delay);
                    self.output_retry_delay = next_retry_delay(self.output_retry_delay);
                    warn!(%error, backend = self.backend_name, "output backend unavailable; retry scheduled");
                }
                Err(error) => return Err(anyhow::Error::new(error)),
            }
        }
        Ok(())
    }

    /// Answer a pending `explain` request from the live engine state, adding
    /// what only the daemon knows: whether the output route is connected.
    pub(crate) fn answer_explain_request(&mut self) {
        let Some(request) = self.control.take_explain_request() else {
            return;
        };
        let backend = wayexpand_core::policy_backend_name(self.active_source, self.active_backend);
        let mut explanation = self.config.engine.explain(&request.text, backend);
        explanation.push(
            "daemon",
            CheckStatus::Pass,
            format!(
                "running with {} capture and {} output",
                self.active_source, self.active_backend
            ),
        );
        let output_connected = if self.input_method_mode {
            self.input_method.is_some()
        } else {
            self.backend_name == "none" || self.injector.is_some()
        };
        if self.backend_name == "none" && !self.input_method_mode {
            explanation.push(
                "output backend",
                CheckStatus::Info,
                "no output backend (test mode): matches are reported, not typed",
            );
        } else if output_connected {
            explanation.push("output backend", CheckStatus::Pass, "connected");
        } else {
            explanation.push(
                "output backend",
                CheckStatus::Fail,
                format!("not connected ({})", self.connection_state),
            );
        }
        let answer = if request.json {
            let suppressed_by = explanation.suppressed_by().map(|check| check.name);
            format!(
                "{}\n",
                serde_json::json!({
                    "would_expand": suppressed_by.is_none(),
                    "suppressed_by": suppressed_by,
                    "typed": explanation.typed,
                    "snippet": explanation.snippet,
                    "checks": explanation.checks,
                })
            )
        } else {
            explanation.render_text()
        };
        let _ = request.reply.try_send(answer);
    }
}
