use crate::app::setup::onboarding_step;
use crate::*;

const ONBOARDING_TRIGGER: &str = ":wayexpand-test";
const ONBOARDING_REPLACEMENT: &str = "WayExpand is working!";

fn onboarding_snippet(config: &Config) -> Option<&ExpansionConfig> {
    config.expansion.iter().find(|expansion| {
        expansion.replacement == ONBOARDING_REPLACEMENT
            && expansion.tags.iter().any(|tag| tag == "tutorial")
    })
}

impl GuiApp {
    pub(crate) fn finish_onboarding(&mut self) {
        self.persist_onboarding(self.strings.onboarding_complete_note());
    }

    pub(crate) fn skip_onboarding(&mut self) {
        self.persist_onboarding(self.strings.onboarding_skipped_note());
    }

    fn persist_onboarding(&mut self, message: &str) {
        self.onboarding_completed = true;
        self.onboarding_visible = false;
        self.onboarding_show_troubleshooting = false;
        match save_gui_prefs(
            self.language,
            self.colorpack,
            self.settings_font_scale,
            self.dark_mode,
            self.onboarding_completed,
        ) {
            Ok(()) => self.status = Status::success(message),
            Err(error) => {
                self.status = Status::warning(format!("{message} ({error})"));
            }
        }
    }

    pub(crate) fn open_getting_started(&mut self) {
        self.onboarding_visible = true;
        self.onboarding_show_troubleshooting = false;
        self.refresh_diagnostics(false);
    }

    /// Keep the first-run guide available after saving the tutorial snippet.
    /// The preview below checks matching locally; the user separately confirms
    /// whether a real application received the replacement.
    pub(crate) fn render_first_run(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let desktop = env::var("XDG_CURRENT_DESKTOP")
            .or_else(|_| env::var("XDG_SESSION_DESKTOP"))
            .unwrap_or_else(|_| self.strings.unknown_desktop().into());
        let is_wayland = env::var_os("WAYLAND_DISPLAY").is_some();
        let app_context_available = self.backend_status.iter().any(|status| {
            status.kind == wayexpand_core::BackendKind::WindowTracker
                && status.state == BackendState::Available
        });
        let recommendation = self.recommended_route.and_then(route_recommendation);
        let (recommended_route, recommended_route_detail) =
            self.strings.onboarding_recommended_route(recommendation);
        let daemon_running = self.daemon_reachable == Some(true);
        let route_connected = daemon_running
            && !self.paused
            && self.route_state == Some(runtime::RouteState::Connected);
        let daemon_build = daemon_running.then(|| runtime::daemon_build_state(&self.daemon_status));
        let route_ready =
            route_connected && daemon_build == Some(runtime::DaemonBuildState::Current);
        let sample_trigger = onboarding_snippet(&self.config)
            .map(|expansion| expansion.trigger.clone())
            .unwrap_or_else(|| ONBOARDING_TRIGGER.to_owned());
        let sample_exists = onboarding_snippet(&self.config).is_some();
        let matcher_passed = sample_exists
            && self.playground.last_trigger.as_deref() == Some(sample_trigger.as_str())
            && self.playground.text.contains(ONBOARDING_REPLACEMENT);
        let status_value = |key: &str| {
            self.daemon_status
                .split(" · ")
                .find_map(|part| part.strip_prefix(&format!("{key}=")))
                .map(str::to_owned)
                .or_else(|| runtime::status_field(&self.daemon_status, key))
        };
        let source = status_value("source").unwrap_or_else(|| "unknown capture".into());
        let restart_command = match source.as_str() {
            "evdev" => Some("systemctl --user restart wayexpand-evdev.service"),
            "input-method" | "input-method-v2" => {
                Some("systemctl --user restart wayexpand-input-method.service")
            }
            _ => None,
        };
        let injection = self
            .daemon_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.injection_mode)
            .map(str::to_owned)
            .or_else(|| status_value("backend"))
            .unwrap_or_else(|| "unknown injection".into());
        let active_route = format!("{source} → {injection}");
        let route_state_detail = match self.route_state {
            Some(runtime::RouteState::PermissionRequired) => "permission required".to_owned(),
            Some(runtime::RouteState::PortalRevoked) => "desktop permission was revoked".to_owned(),
            Some(runtime::RouteState::Unsupported) => "no supported route is active".to_owned(),
            Some(runtime::RouteState::Failed) => "route failed to start".to_owned(),
            Some(runtime::RouteState::Reconnecting) => "route is reconnecting".to_owned(),
            Some(runtime::RouteState::Starting) => "route is starting".to_owned(),
            Some(runtime::RouteState::Degraded) => "route is degraded".to_owned(),
            Some(runtime::RouteState::Stopped) => "route is stopped".to_owned(),
            Some(runtime::RouteState::Connected) if self.paused => "WayExpand is paused".to_owned(),
            Some(runtime::RouteState::Connected)
                if daemon_build == Some(runtime::DaemonBuildState::RestartRequired) =>
            {
                self.strings.onboarding_stale_daemon_state(restart_command)
            }
            Some(runtime::RouteState::Connected)
                if daemon_build == Some(runtime::DaemonBuildState::Unverified) =>
            {
                self.strings.onboarding_unverified_daemon_state().into()
            }
            Some(runtime::RouteState::Connected) => active_route.clone(),
            None if !daemon_running => "user service is not reachable".to_owned(),
            None => "waiting for live route status".to_owned(),
        };
        let route_assurance = self
            .daemon_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.replacement_assurance)
            .map(|assurance| self.strings.replacement_explanation(assurance));

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("first_run_guide")
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    theme::brand_mark(ui, 36.0, palette.accent);
                    ui.label(
                        RichText::new(self.strings.welcome_title())
                            .size(26.0)
                            .strong()
                            .color(ui.visuals().strong_text_color()),
                    );
                    ui.label(RichText::new(self.strings.welcome_intro()).color(palette.muted));
                });
                ui.add_space(14.0);
                theme::card(ui, palette, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(self.strings.onboarding_desktop(&desktop, is_wayland))
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(self.strings.onboarding_skip()).clicked() {
                                self.skip_onboarding();
                            }
                        });
                    });
                    ui.label(
                        RichText::new(
                            self.strings
                                .onboarding_install_summary(env!("CARGO_PKG_VERSION")),
                        )
                        .color(palette.muted),
                    );
                    ui.add_space(12.0);

                    onboarding_step(
                        ui,
                        palette,
                        1,
                        self.strings.onboarding_turn_on_title(),
                        daemon_running,
                    );
                    ui.indent("first_run_service", |ui| {
                        if daemon_running {
                            ui.colored_label(palette.success, self.strings.running_status());
                        } else {
                            ui.label(
                                RichText::new(self.strings.onboarding_turn_on_help())
                                    .color(palette.muted),
                            );
                            self.turn_on_button(ui, palette);
                            if self.status.is_error() {
                                ui.colored_label(palette.danger, self.status.text());
                                ui.label(
                                    RichText::new(self.strings.onboarding_setup_recovery())
                                        .small()
                                        .color(palette.muted),
                                );
                            }
                            if ui.link(self.strings.onboarding_evdev_setup()).clicked() {
                                self.evdev_setup_acknowledged = false;
                                self.evdev_setup_open = true;
                            }
                        }
                    });
                    ui.add_space(10.0);

                    onboarding_step(
                        ui,
                        palette,
                        2,
                        self.strings.onboarding_backend_title(),
                        route_ready,
                    );
                    ui.indent("first_run_backend", |ui| {
                        ui.label(
                            RichText::new(self.strings.onboarding_session_state(
                                route_ready,
                                &route_state_detail,
                                daemon_build,
                                restart_command,
                            ))
                            .color(if route_ready {
                                palette.success
                            } else {
                                palette.warning
                            }),
                        );
                        if let Some(assurance) = route_assurance {
                            ui.label(RichText::new(assurance).small().color(palette.muted));
                        }
                        ui.horizontal_wrapped(|ui| {
                            if theme::secondary_button(
                                ui,
                                palette,
                                self.strings.onboarding_refresh_checks(),
                            )
                            .clicked()
                            {
                                self.refresh_diagnostics(true);
                            }
                            if ui
                                .button(self.strings.onboarding_open_diagnostics())
                                .clicked()
                            {
                                self.diagnostics_open = true;
                                self.refresh_diagnostics(false);
                            }
                        });
                        if !route_ready {
                            ui.label(
                                RichText::new(self.strings.onboarding_probe_caveat())
                                    .small()
                                    .color(palette.muted),
                            );
                        }
                    });
                    ui.add_space(10.0);

                    onboarding_step(
                        ui,
                        palette,
                        3,
                        self.strings.onboarding_snippet_title(),
                        sample_exists,
                    );
                    ui.indent("first_run_snippet", |ui| {
                        if sample_exists {
                            ui.colored_label(
                                palette.success,
                                self.strings.onboarding_sample_saved(),
                            );
                        } else {
                            ui.label(
                                RichText::new(self.strings.onboarding_snippet_help())
                                    .color(palette.muted),
                            );
                            ui.horizontal_wrapped(|ui| {
                                if theme::primary_button(
                                    ui,
                                    palette,
                                    self.strings.create_test_snippet(),
                                )
                                .clicked()
                                {
                                    self.create_test_snippet();
                                }
                                if theme::secondary_button(ui, palette, self.strings.new_button())
                                    .clicked()
                                {
                                    self.onboarding_visible = false;
                                    self.request_action(PendingAction::New);
                                }
                                if theme::secondary_button(
                                    ui,
                                    palette,
                                    self.strings.import_espanso(),
                                )
                                .clicked()
                                {
                                    self.onboarding_visible = false;
                                    self.import_open = true;
                                    self.import_preview = None;
                                }
                            });
                        }
                    });
                    ui.add_space(10.0);

                    onboarding_step(
                        ui,
                        palette,
                        4,
                        self.strings.onboarding_matcher_title(),
                        matcher_passed,
                    );
                    ui.indent("first_run_matcher", |ui| {
                        ui.label(
                            RichText::new(self.strings.onboarding_matcher_help())
                                .color(palette.muted),
                        );
                        ui.label(
                            RichText::new(self.strings.onboarding_trigger(&sample_trigger))
                                .monospace()
                                .strong(),
                        );
                        ui.colored_label(palette.accent, self.strings.onboarding_expected_result());
                        if !sample_exists {
                            ui.label(
                                RichText::new(self.strings.onboarding_create_sample_first())
                                    .small()
                                    .color(palette.warning),
                            );
                        }
                    });
                    ui.add_space(10.0);

                    onboarding_step(
                        ui,
                        palette,
                        5,
                        self.strings.onboarding_real_app_title(),
                        false,
                    );
                    ui.indent("first_run_real_app", |ui| {
                        ui.label(
                            RichText::new(self.strings.onboarding_real_app_help())
                                .color(palette.muted),
                        );
                        if sample_exists && route_ready {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(self.strings.onboarding_trigger(&sample_trigger))
                                        .monospace()
                                        .strong(),
                                );
                                if ui
                                    .small_button(self.strings.onboarding_copy_trigger())
                                    .clicked()
                                {
                                    ui.ctx().copy_text(sample_trigger.to_owned());
                                    self.status =
                                        Status::info(self.strings.status_preview_copied());
                                }
                            });
                            ui.colored_label(
                                palette.accent,
                                self.strings.onboarding_expected_result(),
                            );
                            ui.label(
                                RichText::new(self.strings.onboarding_manual_check_note())
                                    .small()
                                    .color(palette.warning),
                            );
                            ui.horizontal_wrapped(|ui| {
                                if theme::primary_button(
                                    ui,
                                    palette,
                                    self.strings.onboarding_test_succeeded(),
                                )
                                .clicked()
                                {
                                    self.finish_onboarding();
                                }
                                if theme::secondary_button(
                                    ui,
                                    palette,
                                    self.strings.onboarding_test_failed(),
                                )
                                .clicked()
                                {
                                    self.onboarding_show_troubleshooting = true;
                                    self.refresh_diagnostics(false);
                                }
                            });
                        } else if !sample_exists {
                            ui.label(
                                RichText::new(self.strings.onboarding_create_sample_first())
                                    .color(palette.warning),
                            );
                        } else if daemon_build == Some(runtime::DaemonBuildState::RestartRequired) {
                            ui.label(
                                RichText::new(
                                    self.strings.onboarding_stale_daemon_state(restart_command),
                                )
                                .color(palette.warning),
                            );
                        } else if daemon_build == Some(runtime::DaemonBuildState::Unverified) {
                            ui.label(
                                RichText::new(self.strings.onboarding_unverified_daemon_state())
                                    .color(palette.warning),
                            );
                        } else {
                            ui.label(
                                RichText::new(self.strings.onboarding_backend_first())
                                    .color(palette.warning),
                            );
                        }
                        if self.onboarding_show_troubleshooting {
                            ui.add_space(8.0);
                            ui.colored_label(
                                palette.warning,
                                self.strings.onboarding_troubleshooting_title(),
                            );
                            ui.label(
                                RichText::new(self.strings.onboarding_troubleshooting_steps())
                                    .small(),
                            );
                            ui.horizontal_wrapped(|ui| {
                                if theme::secondary_button(
                                    ui,
                                    palette,
                                    self.strings.onboarding_refresh_checks(),
                                )
                                .clicked()
                                {
                                    self.refresh_diagnostics(true);
                                }
                                if ui
                                    .button(self.strings.onboarding_open_diagnostics())
                                    .clicked()
                                {
                                    self.diagnostics_open = true;
                                    self.refresh_diagnostics(false);
                                }
                            });
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(self.strings.onboarding_certification_note())
                            .small()
                            .color(palette.warning),
                    );
                    if let Some(route) = recommendation {
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                recommended_route, recommended_route_detail
                            ))
                            .small()
                            .color(palette.muted),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                route.capture_label, route.injection_label
                            ))
                            .small()
                            .monospace()
                            .color(palette.muted),
                        );
                    } else {
                        ui.label(
                            RichText::new(recommended_route)
                                .small()
                                .color(palette.muted),
                        );
                    }
                    ui.label(
                        RichText::new(self.strings.onboarding_detection(app_context_available))
                            .small()
                            .color(palette.muted),
                    );
                    ui.label(
                        RichText::new(self.strings.onboarding_app_caveat())
                            .small()
                            .color(palette.muted),
                    );
                });
                ui.add_space(16.0);
            });
    }
}
