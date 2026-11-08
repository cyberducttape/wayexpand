use crate::app::setup::onboarding_step;
use crate::*;

const ONBOARDING_TRIGGER: &str = ":wayexpand-test";
const ONBOARDING_REPLACEMENT: &str = "WayExpand is working!";

fn gnome_ibus_is_configured(
    ibus_route_active: bool,
    source_registered: wayexpand_backend_ibus::chain::IbusCheckState,
    source_active: wayexpand_backend_ibus::chain::IbusCheckState,
) -> bool {
    ibus_route_active
        && source_registered == wayexpand_backend_ibus::chain::IbusCheckState::Pass
        && source_active == wayexpand_backend_ibus::chain::IbusCheckState::Pass
}

/// Whether first-run should judge this desktop by its GNOME IBus route. A
/// companion daemon runs beside the engine, so it never decides this.
fn uses_gnome_ibus_route(is_gnome: bool, ibus_active: bool, recommends_ibus: bool) -> bool {
    is_gnome && (ibus_active || recommends_ibus)
}

fn onboarding_snippet(config: &Config) -> Option<&ExpansionConfig> {
    config.expansion.iter().find(|expansion| {
        expansion.enabled
            && expansion.app_filter.is_empty()
            && expansion.command.is_none()
            && !expansion.replacement.contains("{{")
            && expansion.replacement == ONBOARDING_REPLACEMENT
            && expansion.tags.iter().any(|tag| tag == "tutorial")
    })
}

pub(super) fn onboarding_test_snippet(config: &Config) -> Option<&ExpansionConfig> {
    onboarding_snippet(config)
}

impl GuiApp {
    /// Persist the pending-test state so a previously completed guide reopens
    /// if setup was changed and the user postpones verification.
    pub(crate) fn require_desktop_test(&mut self) -> Option<String> {
        self.onboarding_completed = false;
        self.onboarding_visible = true;
        self.desktop_test_dialog_required = true;
        save_gui_prefs(
            self.language,
            self.colorpack,
            self.settings_font_scale,
            self.dark_mode,
            false,
        )
        .err()
        .map(|error| error.to_string())
    }

    pub(crate) fn finish_onboarding(&mut self) {
        if !self.desktop_test_verified {
            self.onboarding_visible = true;
            self.status = Status::warning(
                "First run is not complete: verify an exact insertion in a real application first.",
            );
            return;
        }
        self.persist_onboarding(self.strings.onboarding_complete_note());
    }

    pub(crate) fn defer_onboarding(&mut self) {
        // Let users continue working while preserving the incomplete state.
        // The guide returns on the next launch and cannot be recorded as
        // completed without a real-application verification.
        self.onboarding_visible = false;
        self.status = Status::warning(
            "Setup is not verified yet. The first-run guide will return until a real application test succeeds.",
        );
    }

    fn open_copy_only_picker(&mut self) {
        let executable = match env::current_exe() {
            Ok(executable) => executable,
            Err(error) => {
                self.status = Status::error(format!(
                    "Could not locate the Quick Picker executable: {error}"
                ));
                return;
            }
        };
        match std::process::Command::new(executable)
            .arg("--picker")
            .arg("--copy-only")
            .arg(&self.path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {
                self.status = Status::info(
                    "Quick Picker opened in copy-only mode. Choose a snippet, then paste it yourself; the picker does not request automatic insertion.",
                );
            }
            Err(error) => {
                self.status = Status::error(format!("Could not open Quick Picker: {error}"));
            }
        }
    }

    fn persist_onboarding(&mut self, message: &str) {
        self.onboarding_completed = true;
        self.onboarding_visible = false;
        self.desktop_test_dialog_required = false;
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
    /// Readiness of the GNOME IBus route: a live engine plus a registered and
    /// selected source. `wayexpand-control.service` may run beside the engine
    /// and is deliberately not an input here, in either direction.
    pub(crate) fn gnome_ibus_route_ready(&self) -> bool {
        let check_state = |id: &str| {
            self.ibus_route
                .as_ref()
                .and_then(|report| report.check(id))
                .map_or(
                    wayexpand_backend_ibus::chain::IbusCheckState::Unknown,
                    |check| check.state,
                )
        };
        gnome_ibus_is_configured(
            self.ibus_active,
            check_state("gnome-source-registered"),
            check_state("gnome-source-active"),
        )
    }

    pub(crate) fn render_first_run(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let desktop = env::var("XDG_CURRENT_DESKTOP")
            .or_else(|_| env::var("XDG_SESSION_DESKTOP"))
            .unwrap_or_else(|_| self.strings.unknown_desktop().into());
        let is_wayland = env::var_os("WAYLAND_DISPLAY").is_some();
        let app_context_available = self.backend_status.iter().any(|status| {
            status.kind == wayexpand_core::BackendKind::WindowTracker
                && status.state == BackendState::Available
        });
        let recommendation = self
            .recommended_route
            .and_then(|route| route_recommendation(route, self.route_plan.as_ref()));
        let (recommended_route, recommended_route_detail) =
            self.strings.onboarding_recommended_route(recommendation);
        let daemon_running = self.daemon_reachable == Some(true);
        // The IBus route needs no background service; a live engine means
        // WayExpand is already on.
        let ibus_only = !daemon_running && self.ibus_active;
        let route_connected = daemon_running
            && !self.paused
            && self.route_state == Some(runtime::RouteState::Connected);
        let daemon_build = daemon_running.then(|| runtime::daemon_build_state(&self.daemon_status));
        let gnome_ibus = uses_gnome_ibus_route(
            wayexpand_backend_selection::Compositor::detect()
                == wayexpand_backend_selection::Compositor::Gnome,
            self.ibus_active,
            self.recommended_route
                .is_some_and(|route| route.setup_backend() == "ibus"),
        );
        let source_registered = self.ibus_route.as_ref().is_some_and(|report| {
            report
                .check("gnome-source-registered")
                .is_some_and(|check| {
                    check.state == wayexpand_backend_ibus::chain::IbusCheckState::Pass
                })
        });
        let source_active = self.ibus_route.as_ref().is_some_and(|report| {
            report.check("gnome-source-active").is_some_and(|check| {
                check.state == wayexpand_backend_ibus::chain::IbusCheckState::Pass
            })
        });
        // A running IBus engine is only a configured GNOME route when GNOME
        // has registered and actually selected the source. A connection by
        // itself must never look like a working desktop integration.
        let route_ready = if gnome_ibus {
            self.gnome_ibus_route_ready()
        } else {
            ibus_only || route_connected && daemon_build == Some(runtime::DaemonBuildState::Current)
        };
        let test_snippet = onboarding_test_snippet(&self.config)
            .map(|expansion| (expansion.trigger.clone(), expansion.replacement.clone()));
        let sample_trigger = test_snippet
            .as_ref()
            .map(|(trigger, _)| trigger.clone())
            .unwrap_or_else(|| ONBOARDING_TRIGGER.to_owned());
        let sample_exists = onboarding_snippet(&self.config).is_some();
        let matcher_passed = test_snippet.is_some()
            && self.playground.last_trigger.as_deref() == Some(sample_trigger.as_str())
            && test_snippet
                .as_ref()
                .is_some_and(|(_, replacement)| self.playground.text.contains(replacement));
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
            "stdin" => Some("systemctl --user restart wayexpand-control.service"),
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
        let detail = |key: &str| self.strings.onboarding_route_detail(key).to_owned();
        let route_state_detail = match self.route_state {
            // A companion's own route state says nothing about the IBus route.
            _ if ibus_only || gnome_ibus && route_ready => detail("ibus"),
            Some(runtime::RouteState::PermissionRequired) => detail("permission_required"),
            Some(runtime::RouteState::PortalRevoked) => detail("portal_revoked"),
            Some(runtime::RouteState::Unsupported) => detail("unsupported"),
            Some(runtime::RouteState::Failed) => detail("failed"),
            Some(runtime::RouteState::Reconnecting) => detail("reconnecting"),
            Some(runtime::RouteState::Starting) => detail("starting"),
            Some(runtime::RouteState::Degraded) => detail("degraded"),
            Some(runtime::RouteState::Stopped) => detail("stopped"),
            Some(runtime::RouteState::Connected) if self.paused => detail("paused"),
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
            None if !daemon_running => detail("service_not_running"),
            None => detail("waiting"),
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

                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "Installed: {}",
                            if env::current_exe().is_ok() {
                                "yes"
                            } else {
                                "unknown"
                            }
                        ));
                        ui.label(format!(
                            "Configured: {}",
                            if route_ready { "ready" } else { "incomplete" }
                        ));
                        ui.label(format!(
                            "Verified working: {}",
                            if self.desktop_test_verified {
                                "yes"
                            } else {
                                "not tested"
                            }
                        ));
                    });
                    ui.add_space(8.0);
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
                if self.gnome_setup_resume_pending {
                    theme::card(ui, palette, |ui| {
                        ui.set_width(ui.available_width());
                        ui.colored_label(
                            palette.warning,
                            "GNOME setup is ready to resume",
                        );
                        ui.label(
                            "WayExpand's GNOME extension is prepared. If you have not logged out and back in since setup paused, do that first so GNOME can load it. Then continue IBus setup here; replacement consent and any administrator authorization are still reviewed separately.",
                        );
                        if theme::primary_button(ui, palette, "Resume GNOME setup").clicked() {
                            self.start_guided_setup();
                        }
                    });
                    ui.add_space(10.0);
                }
                theme::card(ui, palette, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(self.strings.onboarding_desktop(&desktop, is_wayland))
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(self.strings.onboarding_defer()).clicked() {
                                self.defer_onboarding();
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
                        daemon_running || ibus_only,
                    );
                    ui.indent("first_run_service", |ui| {
                        if ibus_only {
                            ui.colored_label(palette.success, self.strings.ibus_active_status());
                        } else if daemon_running {
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
                        if gnome_ibus {
                            ui.add_space(4.0);
                            ui.label(RichText::new("GNOME input source").strong());
                            ui.label(format!(
                                "Registration: {} · Active: {}",
                                if source_registered { "ready" } else { "needs setup" },
                                if source_active { "WayExpand selected" } else { "select WayExpand" },
                            ));
                            if !source_registered {
                                ui.label("Continue guided setup to check dependencies, register the engine, and explain any required administrator authorization.");
                                if theme::primary_button(ui, palette, "Continue guided setup").clicked() {
                                    self.start_guided_setup();
                                }
                            } else if !source_active {
                                ui.label("Choose WayExpand from GNOME’s keyboard/input-source menu. Your existing keyboard layouts remain available.");
                                ui.horizontal_wrapped(|ui| {
                                    if theme::primary_button(ui, palette, "Activate WayExpand").clicked() {
                                        self.retry_gnome_source_activation();
                                    }
                                    if theme::secondary_button(ui, palette, "I selected it — check again").clicked() {
                                        self.refresh_diagnostics(true);
                                    }
                                });
                            }
                        }
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
                        sample_exists || test_snippet.is_some(),
                    );
                    ui.indent("first_run_snippet", |ui| {
                        if sample_exists {
                            ui.colored_label(
                                palette.success,
                                self.strings.onboarding_sample_saved(),
                            );
                        } else if test_snippet.is_some() {
                            ui.colored_label(
                                palette.success,
                                "An enabled static expansion is available for the real-app test.",
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
                        ui.colored_label(
                            palette.accent,
                            test_snippet.as_ref().map_or_else(
                                || self.strings.onboarding_expected_result().to_owned(),
                                |(_, replacement)| format!("Expected result: {replacement}"),
                            ),
                        );
                        if test_snippet.is_none() {
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
                        self.desktop_test_verified,
                    );
                    ui.indent("first_run_real_app", |ui| {
                        ui.label(
                            RichText::new(self.strings.onboarding_real_app_help())
                                .color(palette.muted),
                        );
                        let has_static_test = test_snippet.is_some();
                        if has_static_test && route_ready {
                            ui.horizontal_wrapped(|ui| {
                                if theme::primary_button(
                                    ui,
                                    palette,
                                    "Start Desktop Integration Test",
                                )
                                .clicked()
                                {
                                    self.select_default_desktop_test();
                                    if let Some(error) = self.require_desktop_test() {
                                        self.status = Status::warning(format!(
                                            "Desktop testing is still required, but the pending first-run state could not be saved: {error}"
                                        ));
                                    }
                                    self.test_expansion_dialog_open = true;
                                }
                            });
                        } else if test_snippet.is_none() {
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
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Lower-privilege fallback: copy a snippet and paste it yourself. This does not enable raw keyboard capture or verify automatic expansion.")
                                .small()
                                .color(palette.muted),
                        );
                        if theme::secondary_button(
                            ui,
                            palette,
                            "Open Quick Picker (copy and paste)",
                        )
                        .clicked()
                        {
                            self.open_copy_only_picker();
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

#[cfg(test)]
mod guided_setup_tests {
    use super::{gnome_ibus_is_configured, uses_gnome_ibus_route};
    use wayexpand_backend_ibus::chain::IbusCheckState;

    #[test]
    fn a_live_engine_selects_the_gnome_route_without_a_recommendation() {
        assert!(uses_gnome_ibus_route(true, true, false));
        assert!(uses_gnome_ibus_route(true, false, true));
        assert!(!uses_gnome_ibus_route(true, false, false));
        assert!(!uses_gnome_ibus_route(false, true, true));
    }

    #[test]
    fn gnome_ibus_requires_a_live_engine_and_registered_active_source() {
        assert!(!gnome_ibus_is_configured(
            false,
            IbusCheckState::Pass,
            IbusCheckState::Pass
        ));
        assert!(!gnome_ibus_is_configured(
            true,
            IbusCheckState::Unknown,
            IbusCheckState::Pass
        ));
        assert!(!gnome_ibus_is_configured(
            true,
            IbusCheckState::Pass,
            IbusCheckState::Warn
        ));
        assert!(gnome_ibus_is_configured(
            true,
            IbusCheckState::Pass,
            IbusCheckState::Pass
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::onboarding_test_snippet;
    use wayexpand_core::{Config, ExpansionConfig};

    #[test]
    fn first_run_uses_only_the_dedicated_global_tutorial_snippet() {
        let config = Config::parse(
            "[[expansion]]\ntrigger = ';custom-check'\nreplacement = 'Custom check passed'\n",
        )
        .unwrap();
        assert!(onboarding_test_snippet(&config).is_none());

        let mut config = Config::parse("").unwrap();
        let mut tutorial = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":wayexpand-test".into(),
            aliases: Vec::new(),
            replacement: "WayExpand is working!".into(),
            description: String::new(),
            tags: vec!["tutorial".into()],
            category: String::new(),
            app_filter: vec!["app_id_exact:org.example.Editor".into()],
            match_mode: Default::default(),
            command: None,
            enabled: true,
            propagate_case: false,
            output_file: None,
        };
        config.expansion.push(tutorial.clone());
        assert!(onboarding_test_snippet(&config).is_none());

        tutorial.app_filter.clear();
        config.expansion[0] = tutorial;
        let snippet = onboarding_test_snippet(&config).expect("global tutorial is testable");
        assert_eq!(snippet.trigger, ":wayexpand-test");
        assert_eq!(snippet.replacement, "WayExpand is working!");
    }

    #[test]
    fn invalid_tutorial_variants_are_not_selected_for_first_run() {
        let config = Config::parse(
            "[[expansion]]\ntrigger = ':wayexpand-test'\nreplacement = 'WayExpand is working! {{field:review}}'\ntags = ['tutorial']\n",
        )
        .unwrap();
        assert!(onboarding_test_snippet(&config).is_none());
    }
}
