//! Work done at the start of every reactor turn, before the capture
//! source's input step: control requests, reloads, hotkey and command
//! completions, picker inserts, status publication, and output recovery.

use std::ops::ControlFlow;

use crate::control::InsertRequest;
use crate::output_connect::{BackgroundConnect, ConnectRequest, ConnectResult};
use crate::pending_output::{OutputJob, OutputOrigin};
use crate::status_publisher::{
    set_daemon_status_with_config_state,
    set_daemon_status_with_runtime_capabilities_and_config_state,
};
use crate::*;

fn compute_effective_pause(user_paused: bool, profile_switch_pending: bool) -> bool {
    user_paused || profile_switch_pending
}

fn insert_focus_validation_failed(
    request: &InsertRequest,
    current_token: Option<&str>,
    snapshot: &FocusSnapshot,
) -> bool {
    if insert_target_matches_current_focus(request, current_token, snapshot) {
        return false;
    }
    warn!(reason = "focus_changed", "requested snippet insert refused");
    true
}

fn input_method_injection_capabilities(
    mut text: InjectorCapabilities,
    key_pass_through: Option<InjectorCapabilities>,
) -> InjectorCapabilities {
    if let Some(key_pass_through) = key_pass_through {
        // The input-method text protocol supplies atomic Unicode text while
        // the separately attached keyboard injector supplies physical key
        // pass-through. Publish the union of those two negotiated paths.
        text.key_passthrough = key_pass_through.key_passthrough;
    }
    text
}

impl Daemon {
    /// Refresh the diagnostic status at a bounded cadence on ordinary input
    /// turns. Event handling and output failures use `publish_status` directly
    /// so important transitions remain immediate.
    pub(crate) fn publish_status_if_due(&mut self, metrics: CommandMetrics) {
        if self.status_publisher.reactor_refresh_due() {
            self.publish_status(metrics);
        }
    }

    /// Apply window, pause, reload, and stop transitions; report hotkey results and worker failures. Breaks when the daemon should stop.
    pub(crate) fn maintain(&mut self) -> Result<ControlFlow<(), CommandMetrics>> {
        self.config.engine.expire_command_cache();
        // Before any reload replaces the engine and its pending events.
        self.usage.collect(&mut self.config.engine);
        drain_pending_window_events(
            &self.window_tracker,
            &mut self.config.engine,
            &self.policy,
            self.active_backend,
            &self.control,
            &mut self.focus_state,
        )?;
        self.update_profile_for_focus()?;
        let transition = reactor::ReactorTransition::sample(
            &self.control.stop_requested,
            &self.control.pause_requested,
            &self.control.reload_requested,
            self.paused,
        );
        if let Some(requested_pause) = transition.pause_changed() {
            let effective_pause =
                compute_effective_pause(requested_pause, self.profile_switch_pending);
            process_event(
                &mut self.config.engine,
                InputEvent::PauseChanged(effective_pause),
                None,
                &self.policy,
                self.active_backend,
            )?;
            self.paused = requested_pause;
            info!(
                self.paused,
                effective_pause, "expansion processing policy changed"
            );
        }
        if transition.reload_requested() {
            self.reload_profile_assignments();
            self.config.reload_now();
        }
        if transition.should_stop() {
            return Ok(ControlFlow::Break(()));
        }
        self.config.reload_if_changed();
        let backend_policy_changed = self
            .status_publisher
            .update_backend_policy_blocked(self.config.engine.backend_policy_blocked());
        if let Some(tracker) = &self.window_tracker {
            tracker.set_title_matching_required(self.config.engine.requires_window_titles());
            tracker.set_process_identity_required(
                self.config.engine.requires_process_executable_identity(),
            );
        }
        self.update_profile_for_focus()?;
        // After any reload this turn: outputs use the consent now in effect.
        crate::assurance::GuardState::global().set_mode(self.config.engine.compatibility_mode());
        // Read by keystroke routes as they type, so no reconnect is needed.
        wayexpand_core::set_keystroke_max_chars(self.config.engine.keystroke_max_chars());
        for (action, result) in self.config.engine.drain_completed_hotkeys() {
            match result {
                Ok(()) => info!(chord = %action.chord, "hotkey action completed"),
                Err(error) => warn!(chord = %action.chord, %error, "hotkey action failed"),
            }
        }
        let metrics = self.config.engine.command_metrics();
        if backend_policy_changed {
            self.publish_status(metrics);
        }
        let worker_failure = self
            .output_failures
            .as_ref()
            .and_then(|failures| failures.try_recv().ok());
        if let Some(failure) = worker_failure {
            crate::counters::note_output_disconnected("output_transaction_failed");
            warn!(
                retryable = failure.retryable,
                error = %failure.message,
                "serialized output worker failed"
            );
            self.retire_output_injector();
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
            crate::counters::note_output_backoff(self.output_retry_delay);
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
        self.declare_form_replacement_guarantee();
        let completed_commands = self.config.engine.drain_completed_commands();
        for failure in self.config.engine.drain_command_failures() {
            failure_trace::record_command_failure(self.active_backend, failure);
        }
        if !completed_commands.is_empty() {
            // Apply evdev safety gating: ensure physical key-up was processed
            // and no competing input arrived during command execution.
            // This prevents the race condition where fast commands finish
            // before the trigger key's physical release event is processed.
            let gating = if self.evdev_mode {
                apply_evdev_gating(completed_commands, &mut self.evdev, self.active_backend)
            } else {
                EvdevGatingOutcome {
                    results: completed_commands,
                    follow_up: Vec::new(),
                    abandoned: Vec::new(),
                }
            };
            restore_abandoned_results(&mut self.config.engine, gating.abandoned);

            if self.serialized_output() {
                return self.start_output(
                    OutputOrigin::Command,
                    gating
                        .results
                        .into_iter()
                        .map(OutputJob::Expansion)
                        .collect(),
                    gating.follow_up,
                );
            }
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
                    if let Err(error) = result {
                        self.recover_from_output_error(*error)?;
                    }
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
            // Window-tracker events may have arrived while this reactor turn
            // was handling command completions. Drain again immediately
            // before the target check so it uses the freshest observed
            // KWin identity.
            crate::focus::drain_pending_window_events(
                &self.window_tracker,
                &mut self.config.engine,
                &self.policy,
                self.active_backend,
                &self.control,
                &mut self.focus_state,
            )?;
            self.update_profile_for_focus()?;
            // An explicit insert (quick-insert picker, `wayexpand insert`)
            // types a snippet at the cursor through the same injector and
            // evdev safety gate as a typed expansion. It is a user action,
            // so a refusal or injection failure is logged, never fatal.
            if let Some(violation) = unverified_insert_violation(&self.policy, &request) {
                crate::policy::log_violation(&self.policy, violation);
                // The control socket already refuses these in safe mode;
                // this backstop keeps the reactor authoritative.
                if self
                    .policy
                    .effective_enforcement_policy()
                    .require_verified_insert_target
                {
                    return Ok(true);
                }
            }
            let snapshot = self.control.focus_snapshot();
            let current_token = self.config.engine.current_window().and_then(focus_token);
            if insert_focus_validation_failed(&request, current_token.as_deref(), &snapshot) {
                return Ok(true);
            }
            match self.config.engine.prepare_insert(&request.trigger) {
                Ok(result) => {
                    let gating = if self.evdev_mode {
                        apply_evdev_gating(vec![result], &mut self.evdev, self.active_backend)
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
                    if self.serialized_output() {
                        self.start_output(
                            OutputOrigin::Insert,
                            gating
                                .results
                                .into_iter()
                                .map(OutputJob::Expansion)
                                .collect(),
                            gating.follow_up,
                        )?;
                        return Ok(false);
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
                        self.recover_from_insert_error(&error);
                    }
                    replay_evdev_follow_up(
                        &mut self.config.engine,
                        gating.follow_up,
                        &self.policy,
                        self.active_backend,
                    )?;
                }
                Err(error) => warn!(error_kind = ?error, "requested snippet insert refused"),
            }
        }
        Ok(false)
    }

    /// Publish the current status, including runtime capabilities.
    pub(crate) fn publish_status(&mut self, metrics: CommandMetrics) {
        self.status_publisher
            .update_backend_policy_blocked(self.config.engine.backend_policy_blocked());
        let (backend_mode, capture_capabilities, mut injection_capabilities) =
            if let Some(input_method) = self.input_method.as_ref() {
                let capture_capabilities = InputSource::capabilities(input_method);
                let backend_mode = input_method
                    .key_pass_through_status_detail()
                    .or_else(|| {
                        let detail = input_method.status_detail();
                        (!detail.is_empty()).then_some(detail)
                    })
                    .unwrap_or("unknown");
                let injection_capabilities = TextInjector::capabilities(input_method);
                (backend_mode, capture_capabilities, injection_capabilities)
            } else {
                let injection_capabilities = self
                    .injector
                    .as_ref()
                    .map(|backend| backend.capabilities())
                    .unwrap_or_default();
                (
                    self.injector
                        .as_ref()
                        .map(|backend| backend.status_detail())
                        .filter(|detail| !detail.is_empty())
                        .unwrap_or("unknown"),
                    self.evdev
                        .as_ref()
                        .map(InputSource::capabilities)
                        .unwrap_or_default(),
                    injection_capabilities,
                )
            };
        if let Some(input_method) = self.input_method.as_ref() {
            injection_capabilities = input_method_injection_capabilities(
                injection_capabilities,
                input_method.key_pass_through_capabilities(),
            );
        }
        set_daemon_status_with_runtime_capabilities_and_config_state(
            &mut self.status_publisher,
            &self.control,
            self.active_source,
            self.active_backend,
            self.connection_state,
            self.config.path(),
            self.config.status_state(),
            metrics,
            backend_mode,
            capture_capabilities,
            injection_capabilities,
            self.window_tracker
                .as_ref()
                .is_some_and(backend_lifecycle::WindowTrackerHandle::is_connected),
        );
    }

    /// Make at most one output reconnect attempt per turn.
    pub(crate) fn recover_output(&mut self) -> Result<()> {
        self.retiring_output_workers
            .retain(OutputWorkerState::is_running);
        self.publish_retiring_output_workers();
        if !output_loop::output_reconnect_allowed(
            self.retiring_output_workers
                .iter()
                .map(OutputWorkerState::is_running),
        ) {
            // The previous actor may still be inside an uninterruptible
            // compositor/portal call. Do not create a replacement route until
            // its backend resources and any uncertain operation are retired.
            self.output_retry_at = Some(Instant::now() + Duration::from_millis(50));
            return Ok(());
        }
        // Output recovery is deliberately one attempt per reactor turn. A
        // portal or compositor outage must not park control, reload, status,
        // or shutdown handling inside an exponential-backoff sleep.
        // The connection itself runs on a worker (the portal can take up to
        // 30 seconds), so this turn only starts an attempt or collects one.
        if let Some(connect) = &self.output_connect {
            let Some(result) = connect.poll() else {
                return Ok(());
            };
            self.output_connect = None;
            return self.finish_output_connect(result);
        }
        if self.injector.is_none()
            && !self.input_method_mode
            && self.backend_name != "none"
            && self
                .output_retry_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            crate::counters::note_output_reconnecting();
            self.output_connect = Some(BackgroundConnect::start(
                ConnectRequest {
                    backend: self.backend_name,
                    persist_portal_token: self.config.engine.libei_token_persistence(),
                    fcitx5_direct_commit: self.config.engine.fcitx5_direct_commit(),
                    portal_token_path: self.portal_token_path.clone(),
                },
                self.waker.clone(),
            ));
        }
        Ok(())
    }

    /// Install a finished output connection, or schedule the next attempt.
    fn finish_output_connect(&mut self, result: ConnectResult) -> Result<()> {
        match result {
            Ok(backend) => {
                let source_capabilities = self
                    .input_method
                    .as_ref()
                    .map(InputSource::capabilities)
                    .or_else(|| self.evdev.as_ref().map(InputSource::capabilities))
                    .unwrap_or_default();
                self.config
                    .set_route_capabilities(backend.capabilities(), source_capabilities);
                if (self.evdev_mode && self.backend_name == "libei")
                    || self.active_source == "stdin"
                {
                    match output_loop::spawn_async_injector_with_stdin_monitor(
                        backend,
                        Some(self.waker.clone()),
                        (self.active_source == "stdin").then(|| self.stdin_input_monitor.clone()),
                    ) {
                        Ok((backend, failures, worker_state)) => {
                            self.injector = Some(backend);
                            self.output_failures = Some(failures);
                            self.output_worker = Some(worker_state);
                        }
                        Err(error) => {
                            crate::counters::note_output_connection_failure();
                            crate::counters::note_output_backoff(self.output_retry_delay);
                            self.output_retry_at = Some(Instant::now() + self.output_retry_delay);
                            self.output_retry_delay = next_retry_delay(self.output_retry_delay);
                            warn!(%error, backend = self.backend_name, "output worker unavailable; retry scheduled");
                            return Ok(());
                        }
                    }
                } else {
                    self.injector = Some(backend);
                }
                self.output_retry_at = None;
                self.output_retry_delay = Duration::from_millis(250);
                // The output session has been renegotiated and is usable.
                self.connection_state = "connected";
                self.publish_status(self.config.engine.command_metrics());
                crate::counters::note_output_reconnect();
                info!(backend = self.backend_name, "output backend reconnected");
            }
            Err(error) if error.retryable => {
                crate::counters::note_output_connection_failure();
                crate::counters::note_output_backoff(self.output_retry_delay);
                self.output_retry_at = Some(Instant::now() + self.output_retry_delay);
                self.output_retry_delay = next_retry_delay(self.output_retry_delay);
                warn!(%error, backend = self.backend_name, "output backend unavailable; retry scheduled");
            }
            Err(error) => return Err(anyhow::Error::new(error)),
        }
        Ok(())
    }

    /// Classify a failed expansion transaction from the injector route:
    /// rejected expansions are skipped, retryable backend failures retire the
    /// output actor and schedule a reconnect, and only permanent failures end
    /// the event loop. The expansion is never replayed.
    pub(crate) fn recover_from_output_error(&mut self, error: EventError) -> Result<()> {
        if error.applied_with_cursor_position_failure() && !error.retryable() {
            warn!(
                outcome = error.diagnostic_category(),
                trigger_chars = error.result.trigger.chars().count(),
                insert_bytes = error.result.insert.len(),
                "replacement was applied but cursor placement failed; matcher context was reset and the daemon will continue"
            );
            return Ok(());
        }
        if error.source.source().is_some_and(|source| {
            source.kind() == wayexpand_core::InjectorErrorKind::InputInterrupted
        }) {
            warn!(
                outcome = error.diagnostic_category(),
                trigger_chars = error.result.trigger.chars().count(),
                insert_bytes = error.result.insert.len(),
                "user input overlapped non-atomic output; the expansion was not committed or replayed"
            );
            return Ok(());
        }
        if error.expansion_rejected() {
            self.note_unsupported_output(&error);
            warn!(
                outcome = error.diagnostic_category(),
                trigger_chars = error.result.trigger.chars().count(),
                insert_bytes = error.result.insert.len(),
                "output backend rejected expansion; continuing"
            );
            return Ok(());
        }
        if !error.retryable() {
            return Err(error.into());
        }
        crate::counters::note_output_disconnected("output_transaction_failed");
        warn!(
            outcome = error.diagnostic_category(),
            trigger_chars = error.result.trigger.chars().count(),
            insert_bytes = error.result.insert.len(),
            "output failed; current expansion is not replayed"
        );
        self.retire_output_injector();
        process_event(
            &mut self.config.engine,
            InputEvent::EndOfInput,
            None,
            &self.policy,
            self.active_backend,
        )?;
        self.connection_state = "reconnecting";
        set_daemon_status_with_config_state(
            &mut self.status_publisher,
            &self.control,
            self.active_source,
            self.active_backend,
            self.connection_state,
            self.config.path(),
            self.config.status_state(),
        );
        self.output_retry_at = Some(Instant::now());
        self.output_retry_delay = Duration::from_millis(250);
        crate::counters::note_output_backoff(self.output_retry_delay);
        Ok(())
    }

    /// Handle a failed explicit insert. It is a user action, so a failure is
    /// logged and never fatal; the insert is never replayed.
    pub(crate) fn recover_from_insert_error(&mut self, error: &EventError) {
        warn!(
            outcome = error.diagnostic_category(),
            "requested snippet insert failed"
        );
        if error.retryable() {
            // A timed-out or partially applied picker insert has an unknown
            // outcome. Retire this actor and wait for its worker to stop
            // before reconnecting; never submit another request to it or
            // replay this insert against a new focused window.
            self.retire_output_injector();
            let _ = process_event(
                &mut self.config.engine,
                InputEvent::EndOfInput,
                None,
                &self.policy,
                self.active_backend,
            );
            self.connection_state = "reconnecting";
            self.output_retry_at = Some(Instant::now());
            self.output_retry_delay = Duration::from_millis(250);
            crate::counters::note_output_disconnected("output_transaction_failed");
            crate::counters::note_output_backoff(self.output_retry_delay);
        }
    }

    /// Tell the engine what the live output route guarantees about erasing
    /// the trigger, so snippet forms are opened and applied only on routes
    /// that verify the trigger is still at the cursor.
    pub(crate) fn declare_form_replacement_guarantee(&mut self) {
        let guarantee = if self.input_method_mode {
            self.input_method
                .as_ref()
                .map(|source| TextInjector::capabilities(source).replacement_guarantee)
        } else {
            self.injector
                .as_ref()
                .map(|backend| backend.capabilities().replacement_guarantee)
        };
        self.config
            .engine
            .set_form_replacement_guarantee(guarantee.unwrap_or_default());
    }

    /// Cancel the current serialized actor and retain its liveness state until
    /// the worker has completed backend shutdown. This prevents recovery from
    /// overlapping an uncertain operation with a newly connected backend.
    pub(crate) fn retire_output_injector(&mut self) {
        if self.injector.take().is_some() {
            if let Some(worker) = self.output_worker.take() {
                self.retiring_output_workers.push(worker.retire());
                self.publish_retiring_output_workers();
            }
        }
    }

    fn publish_retiring_output_workers(&self) {
        crate::output_health::set_retiring(
            self.retiring_output_workers.len(),
            self.retiring_output_workers
                .iter()
                .filter_map(OutputWorkerState::retired_at)
                .min(),
        );
    }

    /// Count an expansion the route refused because it could not type it.
    pub(crate) fn note_unsupported_output(&self, error: &EventError) {
        if let Some(source) = error.source.source() {
            if let Some(reason) = backend_refusal_category(&source.message) {
                crate::counters::note_output_unsupported_reason(reason);
                return;
            }
        }
        if let Some(limit) = self
            .output_capabilities()
            .and_then(|capabilities| error.route_limit(&capabilities))
        {
            crate::counters::note_output_unsupported(limit);
        }
    }

    /// What the connected output route can type, as status reports it.
    fn output_capabilities(&self) -> Option<wayexpand_core::InjectorCapabilities> {
        if self.input_method_mode {
            let input_method = self.input_method.as_ref()?;
            Some(input_method_injection_capabilities(
                TextInjector::capabilities(input_method),
                input_method.key_pass_through_capabilities(),
            ))
        } else {
            self.injector.as_ref().map(|backend| backend.capabilities())
        }
    }

    /// Publish the small state diagnostics need. Matching and rendering are
    /// performed only by the isolated diagnostics worker.
    pub(crate) fn publish_explain_snapshot(&mut self) {
        let backend = wayexpand_core::policy_backend_name(self.active_source, self.active_backend);
        let output_connected = if self.input_method_mode {
            self.input_method.is_some()
        } else {
            self.backend_name == "none" || self.injector.is_some()
        };
        let capabilities = self.output_capabilities();
        let current_window = self.config.engine.current_window();
        let unchanged = self
            .explain_snapshot_cache
            .as_ref()
            .is_some_and(|previous| {
                Arc::ptr_eq(&previous.validated, self.config.engine.validated_config())
                    && previous.backend == backend
                    && previous.capture == self.active_source
                    && previous.output == self.active_backend
                    && previous.connection_detail == self.connection_state
                    && previous.output_connected == output_connected
                    && previous.capabilities == capabilities
                    && previous.paused == self.config.engine.is_user_paused()
                    && previous.requested_pause == self.paused
                    && previous.profile_switch_pending == self.profile_switch_pending
                    && previous.sensitive == self.config.engine.is_sensitive_focus()
                    && previous.composition == self.config.engine.is_composition_active()
                    && previous.form_open == self.config.engine.is_form_open()
                    && previous.window.as_deref() == current_window
                    && previous.form_guarantee == self.config.engine.form_replacement_guarantee()
                    && previous.commands_disabled == self.config.engine.commands_disabled()
                    && previous.direct_commands_disabled
                        == self.config.engine.direct_commands_disabled()
                    && previous.title_matching_disabled
                        == self.config.engine.title_matching_disabled()
            });
        if unchanged {
            return;
        }
        let snapshot = crate::control::ExplainSnapshot {
            validated: Arc::clone(self.config.engine.validated_config()),
            backend,
            capture: self.active_source,
            output: self.active_backend,
            connection_detail: self.connection_state,
            output_connected,
            capabilities,
            paused: self.config.engine.is_user_paused(),
            requested_pause: self.paused,
            profile_switch_pending: self.profile_switch_pending,
            sensitive: self.config.engine.is_sensitive_focus(),
            composition: self.config.engine.is_composition_active(),
            form_open: self.config.engine.is_form_open(),
            window: current_window.cloned().map(Arc::new),
            form_guarantee: self.config.engine.form_replacement_guarantee(),
            commands_disabled: self.config.engine.commands_disabled(),
            direct_commands_disabled: self.config.engine.direct_commands_disabled(),
            title_matching_disabled: self.config.engine.title_matching_disabled(),
        };
        self.control.set_explain_snapshot(snapshot.clone());
        self.explain_snapshot_cache = Some(snapshot);
    }
}

#[cfg(test)]
mod profile_pause_tests {
    use super::compute_effective_pause;

    #[test]
    fn user_resume_cannot_unpause_matching_during_profile_load() {
        assert!(compute_effective_pause(false, true));
        assert!(!compute_effective_pause(false, false));
        assert!(compute_effective_pause(true, false));
    }
}

/// Policy violation for an explicit insert that names no focus target.
fn unverified_insert_violation(
    policy: &wayexpand_core::OrganizationPolicy,
    request: &InsertRequest,
) -> Option<&'static str> {
    (policy.require_verified_insert_target && request.focus_token.is_none())
        .then_some("explicit insert refused: no verified focus target was supplied")
}

fn insert_target_matches_current_focus(
    request: &InsertRequest,
    current_token: Option<&str>,
    snapshot: &FocusSnapshot,
) -> bool {
    let Some(expected_token) = request.focus_token.as_deref() else {
        // The legacy explicit `wayexpand insert` command has no picker target.
        return true;
    };
    current_token == Some(expected_token)
        && snapshot.exact_window_identity
        && snapshot.token.as_deref() == Some(expected_token)
        && request.focus_generation == Some(snapshot.generation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct LogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn focus_change_refusal_log_does_not_include_window_tokens() {
        let original_window = WindowContext {
            app_id: Some("org.kde.konsole".into()),
            title: Some("bash".into()),
            instance_id: Some("expected-window-instance".into()),
        };
        let focused_window = WindowContext {
            instance_id: Some("actual-window-instance".into()),
            ..original_window.clone()
        };
        let expected_token = focus_token(&original_window).unwrap();
        let actual_token = focus_token(&focused_window).unwrap();
        let request = InsertRequest {
            trigger: ";sig".into(),
            focus_token: Some(expected_token.clone()),
            focus_generation: Some(4),
        };
        let snapshot = FocusSnapshot {
            generation: 5,
            token: Some(actual_token.clone()),
            exact_window_identity: true,
            diagnostic_app_id: None,
        };
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = std::sync::Arc::clone(&bytes);
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || LogBuffer(std::sync::Arc::clone(&writer)))
            .finish();
        let dispatcher = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatcher, || {
            assert!(insert_focus_validation_failed(
                &request,
                Some(&actual_token),
                &snapshot,
            ));
        });

        let record = String::from_utf8(
            bytes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        )
        .unwrap();
        assert!(record.contains("reason=\"focus_changed\""), "{record}");
        assert!(
            record.contains("requested snippet insert refused"),
            "{record}"
        );
        assert!(!record.contains("expected"), "{record}");
        assert!(!record.contains("actual"), "{record}");
        assert!(!record.contains(&expected_token), "{record}");
        assert!(!record.contains(&actual_token), "{record}");
    }

    #[test]
    fn unverified_insert_is_a_violation_only_under_the_policy() {
        let unguarded = InsertRequest {
            trigger: ";sig".into(),
            focus_token: None,
            focus_generation: None,
        };
        let guarded = InsertRequest {
            focus_token: Some("abcd".into()),
            focus_generation: Some(1),
            ..unguarded.clone()
        };
        let mut policy = wayexpand_core::OrganizationPolicy::default();
        assert_eq!(unverified_insert_violation(&policy, &unguarded), None);
        policy.require_verified_insert_target = true;
        assert!(unverified_insert_violation(&policy, &unguarded).is_some());
        assert_eq!(unverified_insert_violation(&policy, &guarded), None);
    }

    #[test]
    fn insert_target_rejects_same_app_title_window_after_focus_changes() {
        let first = WindowContext {
            app_id: Some("org.kde.konsole".into()),
            title: Some("bash".into()),
            instance_id: Some("window-a".into()),
        };
        let second = WindowContext {
            instance_id: Some("window-b".into()),
            ..first.clone()
        };
        let expected_token = focus_token(&first).unwrap();
        let request = InsertRequest {
            trigger: ";sig".into(),
            focus_token: Some(expected_token.clone()),
            focus_generation: Some(4),
        };
        let current_token = focus_token(&second).unwrap();
        let changed_snapshot = FocusSnapshot {
            generation: 5,
            token: Some(current_token.clone()),
            exact_window_identity: true,
            diagnostic_app_id: None,
        };

        assert!(!insert_target_matches_current_focus(
            &request,
            Some(&current_token),
            &changed_snapshot,
        ));
    }

    #[test]
    fn insert_target_requires_exact_identity_and_allows_unguarded_cli_insert() {
        let token = "0123456789abcdef";
        let guarded = InsertRequest {
            trigger: ";sig".into(),
            focus_token: Some(token.into()),
            focus_generation: Some(8),
        };
        let snapshot = FocusSnapshot {
            generation: 8,
            token: Some(token.into()),
            exact_window_identity: false,
            diagnostic_app_id: None,
        };
        assert!(!insert_target_matches_current_focus(
            &guarded,
            Some(token),
            &snapshot,
        ));

        let unguarded = InsertRequest {
            trigger: ";sig".into(),
            focus_token: None,
            focus_generation: None,
        };
        assert!(insert_target_matches_current_focus(
            &unguarded, None, &snapshot
        ));
    }

    #[test]
    fn input_method_status_combines_text_and_key_injector_capabilities() {
        let text = InjectorCapabilities {
            atomic_replace: true,
            replacement_guarantee: wayexpand_core::ReplacementGuarantee::Atomic,
            full_unicode: true,
            key_passthrough: false,
            ..InjectorCapabilities::default()
        };
        let key = InjectorCapabilities {
            key_passthrough: true,
            ..InjectorCapabilities::default()
        };

        let combined = input_method_injection_capabilities(text, Some(key));
        assert!(combined.atomic_replace);
        assert!(combined.full_unicode);
        assert!(combined.key_passthrough);
    }

    #[test]
    fn input_method_status_does_not_claim_key_injection_without_key_support() {
        let text = InjectorCapabilities {
            atomic_replace: true,
            replacement_guarantee: wayexpand_core::ReplacementGuarantee::Atomic,
            full_unicode: true,
            ..InjectorCapabilities::default()
        };
        let key = InjectorCapabilities::default();

        assert!(!input_method_injection_capabilities(text, Some(key)).key_passthrough);
    }
}
