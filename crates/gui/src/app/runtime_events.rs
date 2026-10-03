//! Background runtime: starting the worker and applying its completions (status, diagnostics, saves, reloads).

use crate::*;

impl GuiApp {
    pub(crate) fn start_runtime(&mut self) -> Result<()> {
        let (control_sender, diagnostics_sender, receiver) =
            runtime::start().context("starting GUI background runtime")?;
        self.runtime_sender = Some(control_sender);
        self.diagnostics_sender = Some(diagnostics_sender);
        self.runtime_receiver = Some(receiver);
        self.refresh_diagnostics(false);
        Ok(())
    }

    pub(crate) fn poll_runtime(&mut self, ctx: &egui::Context) {
        let mut completions = Vec::new();
        let mut disconnected = false;
        if let Some(receiver) = self.runtime_receiver.as_ref() {
            loop {
                match receiver.try_recv() {
                    Ok(completion) => completions.push(completion),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.runtime_receiver = None;
            self.runtime_sender = None;
            self.diagnostics_sender = None;
            self.diagnostics_running = false;
            self.pending_control = 0;
            self.pending_reload_revision = None;
            self.pending_save = None;
            self.daemon_reachable = Some(false);
            self.route_state = None;
            self.status = Status::error(self.strings.background_runtime_stopped());
        }
        for completion in completions {
            match completion {
                runtime::Completion::Diagnostics(snapshot) => {
                    self.backend_status = snapshot.backend_status;
                    self.recommended_route = snapshot.recommended_route;
                    self.selection_capabilities = Some(snapshot.selection_capabilities);
                    self.fleet_status = snapshot.fleet_status;
                    self.protocol_probes = snapshot.protocol_probes;
                    self.daemon_status = snapshot.daemon_status;
                    self.daemon_capabilities = snapshot.daemon_capabilities;
                    self.daemon_reachable = snapshot.daemon_reachable;
                    self.route_state = snapshot.route_state;
                    if let Some(paused) = snapshot.paused {
                        self.paused = paused;
                    }
                    self.diagnostics_running = false;
                    self.next_status_poll = Instant::now() + Duration::from_secs(2);
                    if snapshot.announce {
                        self.status = Status::success(self.strings.status_diagnostics_refreshed());
                    }
                }
                runtime::Completion::Control { operation, result } => {
                    self.pending_control = self.pending_control.saturating_sub(1);
                    match operation {
                        runtime::Operation::Reload(previous_status) => match result {
                            Ok(_) => {
                                self.daemon_reachable = Some(true);
                                self.status = previous_status;
                            }
                            Err(error) => {
                                self.daemon_reachable = Some(false);
                                self.status = previous_status.with_caveat(
                                    self.strings.status_daemon_not_reloaded(&error.to_string()),
                                )
                            }
                        },
                        runtime::Operation::Pause { paused } => match result {
                            Ok(_) => {
                                self.daemon_reachable = Some(true);
                                self.paused = paused;
                                self.status = Status::success(if paused {
                                    self.strings.status_paused()
                                } else {
                                    self.strings.status_resumed()
                                });
                            }
                            Err(error) => {
                                self.daemon_reachable = Some(false);
                                self.status = Status::error(
                                    self.strings.status_control_unavailable(&error.to_string()),
                                )
                            }
                        },
                        runtime::Operation::Status => match result {
                            Ok(response) => {
                                self.daemon_capabilities =
                                    runtime::DaemonCapabilities::parse(&response);
                                self.daemon_status = response.trim().replace('\n', " · ");
                                self.daemon_reachable = Some(true);
                                self.route_state = runtime::parse_route_state(&response);
                                if let Some(paused) = runtime::parse_paused(&response) {
                                    self.paused = paused;
                                }
                            }
                            Err(error) => {
                                self.daemon_capabilities = None;
                                self.daemon_status = format!("Unavailable: {error}");
                                self.daemon_reachable = Some(false);
                                self.route_state = None;
                            }
                        },
                    }
                }
                runtime::Completion::ConfigReloaded(result) => {
                    self.pending_control = self.pending_control.saturating_sub(1);
                    let Some(expected_revision) = self.pending_reload_revision.take() else {
                        continue;
                    };
                    match *result {
                        Ok(snapshot) => {
                            if self.config_revision != expected_revision || self.draft_is_dirty() {
                                self.status = Status::warning(
                                    self.strings.status_reload_discarded_due_edits(),
                                );
                            } else {
                                self.apply_reload_snapshot(snapshot);
                            }
                        }
                        Err(error) => {
                            self.status = Status::error(self.strings.status_reload_failed(&error));
                        }
                    }
                }
                runtime::Completion::ConfigSaved { request_id, result } => {
                    self.finish_save(request_id, result);
                }
            }
        }
        let now = Instant::now();
        if now >= self.next_status_poll {
            self.next_status_poll = now + Duration::from_secs(2);
            if self.pending_control == 0 {
                if let Some(sender) = self.runtime_sender.as_ref() {
                    if sender
                        .try_send(runtime::Request::Control {
                            command: "status".into(),
                            operation: runtime::Operation::Status,
                        })
                        .is_ok()
                    {
                        self.pending_control += 1;
                    }
                }
            }
        }
        if self.diagnostics_running || self.pending_control > 0 {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else if self.runtime_receiver.is_some() {
            ctx.request_repaint_after(self.next_status_poll.saturating_duration_since(now));
        }
    }
}
