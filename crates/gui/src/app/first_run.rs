use crate::app::setup::onboarding_step;
use crate::*;

const ONBOARDING_TRIGGER: &str = ":wayexpand-test";
const ONBOARDING_REPLACEMENT: &str = "WayExpand is working!";
/// What `wayexpand shortcut set` assigns without an argument.
const DEFAULT_PICKER_SHORTCUT: &str = "Ctrl+Alt+Space";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FirstRunOutcome {
    WorkingInTestedApplication,
    SafeFallback,
    SetupIncomplete,
}

fn first_run_outcome(
    route_ready: bool,
    replacement_admitted: bool,
    test_snippet_available: bool,
    real_app_test_verified: bool,
) -> FirstRunOutcome {
    if !route_ready {
        FirstRunOutcome::SetupIncomplete
    } else if replacement_admitted && test_snippet_available && real_app_test_verified {
        FirstRunOutcome::WorkingInTestedApplication
    } else {
        FirstRunOutcome::SafeFallback
    }
}

fn first_run_outcome_label(strings: &Strings, outcome: FirstRunOutcome) -> &'static str {
    match outcome {
        FirstRunOutcome::WorkingInTestedApplication => strings.outcome_working(),
        FirstRunOutcome::SafeFallback => strings.outcome_safe_fallback(),
        FirstRunOutcome::SetupIncomplete => strings.outcome_setup_incomplete(),
    }
}

fn replacement_test_is_admitted(
    route_ready: bool,
    atomic_replace: Option<bool>,
    replacement_assurance: Option<runtime::ReplacementAssurance>,
    compatibility_mode: Option<wayexpand_core::CompatibilityMode>,
) -> bool {
    route_ready
        && (atomic_replace == Some(true)
            && replacement_assurance == Some(runtime::ReplacementAssurance::Verified)
            || compatibility_mode
                .is_some_and(|mode| mode != wayexpand_core::CompatibilityMode::Off))
}

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
pub(super) fn uses_gnome_ibus_route(
    is_gnome: bool,
    ibus_active: bool,
    recommends_ibus: bool,
) -> bool {
    is_gnome && (ibus_active || recommends_ibus)
}

/// Whether a saved snippet can drive the real-application test: enabled,
/// global, static text with no command or template to render.
pub(super) fn desktop_testable(expansion: &ExpansionConfig) -> bool {
    expansion.is_keyboard_matchable()
        && expansion.app_filter.is_empty()
        && expansion.command.is_none()
        && !expansion.replacement.contains("{{")
}

/// The harmless tutorial snippet the guide can create for the test.
fn onboarding_snippet(config: &Config) -> Option<&ExpansionConfig> {
    config.expansion.iter().find(|expansion| {
        desktop_testable(expansion)
            && expansion.replacement == ONBOARDING_REPLACEMENT
            && expansion.tags.iter().any(|tag| tag == "tutorial")
    })
}

/// The snippet the first-run test uses by default: the tutorial snippet when
/// it exists, otherwise any testable snippet. Requiring the tutorial left
/// users who wrote their own first snippet unable to finish setup, although
/// the guide tells them a suitable static snippet works.
pub(super) fn onboarding_test_snippet(config: &Config) -> Option<&ExpansionConfig> {
    onboarding_snippet(config).or_else(|| config.expansion.iter().find(|e| desktop_testable(e)))
}

impl GuiApp {
    /// Persist the pending-test state so a previously completed guide reopens
    /// if setup was changed and the user postpones verification.
    pub(crate) fn require_desktop_test(&mut self) -> Option<String> {
        self.onboarding_completed = false;
        self.onboarding_visible = true;
        self.desktop_test_dialog_required = true;
        self.desktop_test_verified = false;
        self.desktop_test_verified_application = None;
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

    fn begin_onboarding_desktop_test(&mut self) {
        if !self.select_default_desktop_test() {
            self.status = Status::warning(self.strings.status_create_test_expansion_first());
            return;
        }
        if let Some(error) = self.require_desktop_test() {
            self.status = Status::warning(self.strings.status_real_app_test_state_unsaved(&error));
        }
        self.test_expansion_dialog_open = true;
    }

    pub(crate) fn finish_onboarding(&mut self) {
        if !self.desktop_test_verified {
            self.onboarding_visible = true;
            self.status = Status::warning(self.strings.status_first_run_not_complete());
            return;
        }
        self.persist_onboarding(self.strings.onboarding_complete_note());
    }

    /// Dismiss the guide for good at the user's explicit request. Unlike
    /// finishing it, this records no real-application verification.
    pub(crate) fn skip_onboarding(&mut self) {
        // Skipping records that nothing was verified, without erasing an
        // earlier confirmed test.
        let verified = self
            .setup_record
            .as_ref()
            .is_some_and(|record| record.outcome == crate::setup_record::SetupOutcome::Verified);
        if !verified {
            let record = crate::setup_record::SetupRecord::skipped();
            if crate::setup_record::save(&record).is_ok() {
                self.setup_record = Some(record);
            }
        }
        self.persist_onboarding(self.strings.onboarding_skipped_note());
    }

    pub(crate) fn defer_onboarding(&mut self) {
        // Let users continue working while preserving the incomplete state.
        // The guide returns on the next launch and cannot be recorded as
        // completed without a real-application verification.
        self.onboarding_visible = false;
        self.status = Status::warning(self.strings.status_setup_deferred());
    }

    pub(crate) fn open_copy_only_picker(&mut self) {
        let executable = match env::current_exe() {
            Ok(executable) => executable,
            Err(error) => {
                self.status = Status::error(
                    self.strings
                        .status_picker_executable_missing(&error.to_string()),
                );
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
                self.status = Status::info(self.strings.status_picker_copy_only());
            }
            Err(error) => {
                self.status =
                    Status::error(self.strings.status_picker_open_failed(&error.to_string()));
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
        // Until the first status answer arrives, "not running" would be a
        // guess, and offering Turn on could start setup for a running service.
        let checking = self.daemon_reachable.is_none() && !self.ibus_active;
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
        let restart_unit = match source.as_str() {
            "evdev" => Some("wayexpand-evdev.service"),
            "input-method" | "input-method-v2" => Some("wayexpand-input-method.service"),
            "stdin" => Some("wayexpand-control.service"),
            _ => None,
        };
        let restart_command = restart_unit.map(|unit| format!("systemctl --user restart {unit}"));
        let restart_command = restart_command.as_deref();
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
        let replacement_test_admitted = if gnome_ibus {
            route_ready
                && self
                    .ibus_route
                    .as_ref()
                    .and_then(|report| report.check("consent"))
                    .is_some_and(|check| {
                        check.state == wayexpand_backend_ibus::chain::IbusCheckState::Pass
                    })
        } else {
            self.daemon_capabilities.as_ref().is_some_and(|caps| {
                replacement_test_is_admitted(
                    route_ready,
                    caps.inject_atomic_replace,
                    caps.replacement_assurance,
                    caps.compatibility_mode,
                )
            })
        };
        let outcome = first_run_outcome(
            route_ready,
            replacement_test_admitted,
            test_snippet.is_some(),
            self.desktop_test_verified,
        );

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("first_run_guide")
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
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
                        ui.colored_label(palette.warning, self.strings.gnome_resume_title());
                        ui.label(self.strings.gnome_resume_body());
                        if theme::primary_button(ui, palette, self.strings.gnome_resume_button())
                            .clicked()
                        {
                            self.start_guided_setup();
                        }
                    });
                    ui.add_space(10.0);
                }
                theme::card(ui, palette, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(first_run_outcome_label(&self.strings, outcome))
                                .strong()
                                .color(match outcome {
                                    FirstRunOutcome::WorkingInTestedApplication => palette.success,
                                    _ => palette.warning,
                                }),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(self.strings.onboarding_defer()).clicked() {
                                self.defer_onboarding();
                            }
                            // Someone who already uses WayExpand (an imported
                            // library, another machine) can dismiss the guide
                            // for good; this never marks anything verified.
                            if !self.config.expansion.is_empty()
                                && ui.small_button(self.strings.onboarding_skip()).clicked()
                            {
                                self.skip_onboarding();
                            }
                        });
                    });
                    ui.add_space(10.0);

                    // 1. Connect your desktop.
                    onboarding_step(
                        ui,
                        palette,
                        1,
                        self.strings.step_connect_title(),
                        route_ready,
                    );
                    ui.indent("first_run_connect", |ui| {
                        if checking {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    RichText::new(self.strings.first_run_checking())
                                        .color(palette.muted),
                                );
                            });
                        } else if !(daemon_running || ibus_only) {
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
                        } else if route_ready {
                            ui.colored_label(
                                palette.success,
                                crate::lang::fill(
                                    self.strings.first_run_connected(),
                                    &[&route_state_detail],
                                ),
                            );
                        } else {
                            // One problem and one action: the same answer the
                            // banner under the header gives after first run.
                            let busy = self.setup_task.is_some();
                            match self.current_next_step() {
                                Some((problem, action)) => {
                                    ui.colored_label(
                                        palette.warning,
                                        self.strings.problem_text(problem),
                                    );
                                    let help = match problem {
                                        super::Problem::InputSourceNotSetUp => {
                                            Some(self.strings.gnome_register_help())
                                        }
                                        super::Problem::InputSourceNotSelected => {
                                            Some(self.strings.gnome_select_help())
                                        }
                                        _ => None,
                                    };
                                    if let Some(help) = help {
                                        ui.label(RichText::new(help).color(palette.muted));
                                    }
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add_enabled_ui(!busy, |ui| {
                                                theme::primary_button(
                                                    ui,
                                                    palette,
                                                    self.strings.action_label(action),
                                                )
                                            })
                                            .inner
                                            .clicked()
                                        {
                                            self.perform_next_step(action);
                                        }
                                        if action != super::NextAction::CheckAgain
                                            && theme::secondary_button(
                                                ui,
                                                palette,
                                                self.strings.first_run_check_again(),
                                            )
                                            .clicked()
                                        {
                                            self.refresh_diagnostics(true);
                                        }
                                    });
                                }
                                None => {
                                    ui.colored_label(
                                        palette.warning,
                                        self.strings.onboarding_session_state(
                                            false,
                                            &route_state_detail,
                                            daemon_build,
                                            restart_command,
                                        ),
                                    );
                                    if theme::secondary_button(
                                        ui,
                                        palette,
                                        self.strings.first_run_check_again(),
                                    )
                                    .clicked()
                                    {
                                        self.refresh_diagnostics(true);
                                    }
                                }
                            }
                        }
                    });
                    ui.add_space(12.0);

                    // 2. Create a snippet.
                    onboarding_step(
                        ui,
                        palette,
                        2,
                        self.strings.step_create_title(),
                        test_snippet.is_some(),
                    );
                    ui.indent("first_run_snippet", |ui| {
                        if let Some((trigger, replacement)) = &test_snippet {
                            ui.colored_label(
                                palette.success,
                                crate::lang::fill(
                                    self.strings.first_run_snippet_ready(),
                                    &[&trigger, &replacement],
                                ),
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
                                // The editor shows a new draft over the guide,
                                // which returns once the snippet is saved or
                                // discarded, with this step checked.
                                if theme::secondary_button(ui, palette, self.strings.new_button())
                                    .clicked()
                                {
                                    self.request_action(PendingAction::New);
                                }
                                if theme::secondary_button(
                                    ui,
                                    palette,
                                    self.strings.import_espanso(),
                                )
                                .clicked()
                                {
                                    self.import_open = true;
                                    self.import_preview = None;
                                }
                            });
                        }
                    });
                    ui.add_space(12.0);

                    // 3. Test it, or fall back to copy and paste.
                    onboarding_step(
                        ui,
                        palette,
                        3,
                        self.strings.step_test_title(),
                        outcome == FirstRunOutcome::WorkingInTestedApplication,
                    );
                    ui.indent("first_run_test", |ui| {
                        let mut offer_picker = true;
                        if outcome == FirstRunOutcome::WorkingInTestedApplication {
                            offer_picker = false;
                            let application = self
                                .desktop_test_verified_application
                                .as_deref()
                                .unwrap_or(self.strings.the_tested_application());
                            ui.colored_label(
                                palette.success,
                                self.strings.confirmed_in_application(application),
                            );
                        } else if !route_ready {
                            ui.label(
                                RichText::new(self.strings.first_run_test_needs_connection())
                                    .color(palette.muted),
                            );
                        } else if test_snippet.is_none() {
                            ui.label(
                                RichText::new(self.strings.first_run_test_needs_snippet())
                                    .color(palette.muted),
                            );
                        } else if !replacement_test_admitted {
                            ui.colored_label(
                                palette.warning,
                                self.strings.authorize_or_use_picker(),
                            );
                            if theme::primary_button(
                                ui,
                                palette,
                                self.strings.review_setup_and_consent(),
                            )
                            .clicked()
                            {
                                self.start_guided_setup();
                            }
                        } else {
                            ui.label(
                                RichText::new(self.strings.first_run_test_help())
                                    .color(palette.muted),
                            );
                            if theme::primary_button(
                                ui,
                                palette,
                                self.strings.test_in_real_application(),
                            )
                            .clicked()
                            {
                                self.begin_onboarding_desktop_test();
                            }
                        }
                        if test_snippet.is_some()
                            && outcome != FirstRunOutcome::WorkingInTestedApplication
                        {
                            if matcher_passed {
                                ui.colored_label(
                                    palette.success,
                                    self.strings.first_run_trigger_works_inside(),
                                );
                            } else if ui.link(self.strings.first_run_try_inside()).clicked() {
                                self.try_live_open = true;
                            }
                        }
                        if offer_picker && ui.link(self.strings.open_quick_picker_copy()).clicked()
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
                        }
                    });
                    ui.add_space(12.0);

                    // 4. Optional: a shortcut for the Quick Picker, the way
                    // to insert where typing a trigger cannot expand.
                    let shortcut_keys = self
                        .picker_shortcut
                        .as_ref()
                        .and_then(|shortcut| shortcut.keys.clone());
                    onboarding_step(
                        ui,
                        palette,
                        4,
                        self.strings.step_shortcut_title(),
                        shortcut_keys.is_some(),
                    );
                    ui.indent("first_run_shortcut", |ui| {
                        match (&self.picker_shortcut, &shortcut_keys) {
                            (_, Some(keys)) => {
                                ui.colored_label(
                                    palette.success,
                                    crate::lang::fill(self.strings.shortcut_ready(), &[keys]),
                                );
                                ui.label(
                                    RichText::new(self.strings.shortcut_change_hint())
                                        .small()
                                        .color(palette.muted),
                                );
                            }
                            (Some(shortcut), None) if !shortcut.supported => {
                                ui.label(
                                    RichText::new(self.strings.shortcut_manual())
                                        .color(palette.muted),
                                );
                            }
                            _ => {
                                ui.label(
                                    RichText::new(self.strings.shortcut_help())
                                        .color(palette.muted),
                                );
                                let running = self.setup_task.is_some();
                                if ui
                                    .add_enabled(
                                        !running,
                                        egui::Button::new(crate::lang::fill(
                                            self.strings.shortcut_use_keys(),
                                            &[&DEFAULT_PICKER_SHORTCUT],
                                        )),
                                    )
                                    .clicked()
                                {
                                    self.set_picker_shortcut();
                                }
                                if let Some(error) = &self.picker_shortcut_error {
                                    ui.colored_label(palette.warning, error);
                                }
                            }
                        }
                    });
                    ui.add_space(12.0);

                    // Everything a new user does not need to act on, kept
                    // for anyone diagnosing a problem.
                    egui::CollapsingHeader::new(
                        RichText::new(self.strings.first_run_troubleshooting_details())
                            .color(palette.muted),
                    )
                    .id_salt("first_run_troubleshooting_details")
                    .default_open(self.onboarding_show_troubleshooting)
                    .show(ui, |ui| {
                        let note = |text: &str| {
                            RichText::new(text.to_owned()).small().color(palette.muted)
                        };
                        ui.label(note(&self.strings.onboarding_desktop(&desktop, is_wayland)));
                        ui.label(note(
                            &self
                                .strings
                                .onboarding_install_summary(env!("CARGO_PKG_VERSION")),
                        ));
                        if daemon_running || ibus_only {
                            ui.label(note(&self.strings.onboarding_session_state(
                                route_ready,
                                &route_state_detail,
                                daemon_build,
                                restart_command,
                            )));
                        }
                        if let Some(assurance) = route_assurance {
                            ui.label(note(assurance));
                        }
                        if let Some(route) = recommendation {
                            ui.label(note(&format!(
                                "{recommended_route} · {recommended_route_detail}"
                            )));
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
                            ui.label(note(&recommended_route));
                        }
                        ui.label(note(
                            self.strings.onboarding_detection(app_context_available),
                        ));
                        ui.label(note(self.strings.onboarding_app_caveat()));
                        ui.label(note(self.strings.onboarding_probe_caveat()));
                        ui.label(note(self.strings.onboarding_certification_note()));
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .button(self.strings.onboarding_refresh_checks())
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
                        if !(daemon_running || ibus_only)
                            && ui.link(self.strings.onboarding_evdev_setup()).clicked()
                        {
                            self.evdev_setup_acknowledged = false;
                            self.evdev_setup_open = true;
                        }
                    });
                });
                ui.add_space(16.0);
            });
    }
}

#[cfg(test)]
mod guided_setup_tests {
    use super::{
        first_run_outcome, first_run_outcome_label, gnome_ibus_is_configured,
        replacement_test_is_admitted, uses_gnome_ibus_route, FirstRunOutcome,
    };
    use crate::runtime::ReplacementAssurance;
    use wayexpand_backend_ibus::chain::IbusCheckState;
    use wayexpand_core::CompatibilityMode;

    #[test]
    fn first_run_outcome_requires_a_ready_route_and_a_real_app_confirmation() {
        assert_eq!(
            first_run_outcome(false, false, false, false),
            FirstRunOutcome::SetupIncomplete
        );
        assert_eq!(
            first_run_outcome(false, true, true, true),
            FirstRunOutcome::SetupIncomplete,
            "a stale success cannot hide an unavailable route"
        );
        assert_eq!(
            first_run_outcome(true, true, true, false),
            FirstRunOutcome::SafeFallback
        );
        assert_eq!(
            first_run_outcome(true, true, true, true),
            FirstRunOutcome::WorkingInTestedApplication
        );
        assert_eq!(
            first_run_outcome(true, false, true, true),
            FirstRunOutcome::SafeFallback,
            "a consent change invalidates the previous ready state"
        );
        assert_eq!(
            first_run_outcome(true, true, false, true),
            FirstRunOutcome::SafeFallback,
            "removing the test snippet invalidates the previous ready state"
        );
    }

    #[test]
    fn first_run_states_have_distinct_labels_and_only_one_claims_success() {
        let strings = crate::lang::Strings::new(crate::lang::Language::English);
        assert_eq!(
            first_run_outcome_label(&strings, FirstRunOutcome::WorkingInTestedApplication),
            "Working in tested application"
        );
        assert_eq!(
            first_run_outcome_label(&strings, FirstRunOutcome::SafeFallback),
            "Connected, not tested yet"
        );
        assert_eq!(
            first_run_outcome_label(&strings, FirstRunOutcome::SetupIncomplete),
            "Setup incomplete"
        );
    }

    #[test]
    fn real_app_test_requires_atomic_admission_or_explicit_compatibility_consent() {
        assert!(replacement_test_is_admitted(
            true,
            Some(true),
            Some(ReplacementAssurance::Verified),
            Some(CompatibilityMode::Off)
        ));
        assert!(!replacement_test_is_admitted(
            true,
            Some(false),
            Some(ReplacementAssurance::Blocked),
            Some(CompatibilityMode::Off)
        ));
        assert!(replacement_test_is_admitted(
            true,
            Some(false),
            Some(ReplacementAssurance::FocusTrackedBestEffort),
            Some(CompatibilityMode::TrackedFocus)
        ));
        assert!(!replacement_test_is_admitted(
            false,
            Some(true),
            Some(ReplacementAssurance::Verified),
            Some(CompatibilityMode::UntrackedFocus)
        ));
    }

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
    fn first_run_prefers_the_global_tutorial_snippet_but_accepts_others() {
        let config = Config::parse(
            "[[expansion]]\ntrigger = ';custom-check'\nreplacement = 'Custom check passed'\n",
        )
        .unwrap();
        let custom = onboarding_test_snippet(&config).expect("a static snippet is testable");
        assert_eq!(custom.trigger, ";custom-check");

        let mut config = Config::parse("").unwrap();
        let mut tutorial = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":wayexpand-test".into(),
            aliases: Vec::new(),
            replacement: "WayExpand is working!".into(),
            delivery: wayexpand_core::ExpansionDelivery::Insert,
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

        // With another testable snippet listed first, the tutorial still wins.
        let mut both = Config::parse(
            "[[expansion]]\ntrigger = ';custom-check'\nreplacement = 'Custom check passed'\n",
        )
        .unwrap();
        both.expansion.push(config.expansion[0].clone());
        assert_eq!(
            onboarding_test_snippet(&both).unwrap().trigger,
            ":wayexpand-test"
        );
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
