use crate::app::setup::onboarding_step;
use crate::*;

const ONBOARDING_TRIGGER: &str = ";hello";
const ONBOARDING_REPLACEMENT: &str = "WayExpand is working!";
/// What `wayexpand shortcut set` assigns without an argument.
const DEFAULT_PICKER_SHORTCUT: &str = "Ctrl+Alt+Space";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FirstRunOutcome {
    WorkingInTestedApplication,
    UserConfirmedInTestedApplication,
    ConfiguredAwaitingTest,
    SafeFallback,
    SetupIncomplete,
}

fn first_run_outcome(
    route_ready: bool,
    replacement_admitted: bool,
    test_snippet_available: bool,
    real_app_test_verified: bool,
    machine_observed: bool,
) -> FirstRunOutcome {
    if !route_ready {
        FirstRunOutcome::SetupIncomplete
    } else if replacement_admitted && test_snippet_available && real_app_test_verified {
        if machine_observed {
            FirstRunOutcome::WorkingInTestedApplication
        } else {
            FirstRunOutcome::UserConfirmedInTestedApplication
        }
    } else if replacement_admitted {
        FirstRunOutcome::ConfiguredAwaitingTest
    } else {
        FirstRunOutcome::SafeFallback
    }
}

fn first_run_outcome_label(strings: &Strings, outcome: FirstRunOutcome) -> &'static str {
    match outcome {
        FirstRunOutcome::WorkingInTestedApplication => strings.outcome_working(),
        FirstRunOutcome::UserConfirmedInTestedApplication => strings.outcome_user_confirmed(),
        FirstRunOutcome::ConfiguredAwaitingTest => strings.outcome_configured_awaiting_test(),
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

fn compatibility_replacement_is_active(
    route_ready: bool,
    replacement_admitted: bool,
    atomic_replace: Option<bool>,
    ibus_route: bool,
) -> bool {
    route_ready && replacement_admitted && (ibus_route || atomic_replace != Some(true))
}

/// Whether first-run should judge this desktop by its GNOME IBus route. A
/// companion daemon runs beside the engine, so it never decides this.
#[cfg(test)]
fn uses_gnome_ibus_route(
    is_gnome: bool,
    gnome_source_active: bool,
    gnome_source_registered: bool,
    recommends_ibus: bool,
    daemon_reachable: Option<bool>,
    active_daemon_source: Option<&str>,
) -> bool {
    uses_gnome_ibus_route_with_scope(
        is_gnome,
        gnome_source_active,
        gnome_source_registered,
        recommends_ibus,
        daemon_reachable,
        active_daemon_source,
        false,
    )
}

pub(super) fn uses_gnome_ibus_route_with_scope(
    is_gnome: bool,
    gnome_source_active: bool,
    gnome_source_registered: bool,
    recommends_ibus: bool,
    daemon_reachable: Option<bool>,
    active_daemon_source: Option<&str>,
    terminal_only_daemon: bool,
) -> bool {
    // A live engine process can have stale contexts without being selected by
    // GNOME. An active source proves the route; a registered source identifies
    // the route that needs repair only when no other typing backend is active,
    // unless that backend is limited to terminal applications. A recommendation
    // likewise must not override an explicit full-coverage typing route.
    let no_active_typing_daemon = daemon_reachable == Some(false)
        || daemon_reachable == Some(true) && active_daemon_source == Some("stdin")
        || terminal_only_daemon;
    is_gnome
        && (gnome_source_active
            || (gnome_source_registered || recommends_ibus) && no_active_typing_daemon)
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
fn is_onboarding_snippet(expansion: &ExpansionConfig) -> bool {
    desktop_testable(expansion)
        && expansion.replacement == ONBOARDING_REPLACEMENT
        && expansion.match_mode == MatchMode::Immediate
        && expansion.tags.iter().any(|tag| tag == "tutorial")
}

fn onboarding_snippet(config: &Config) -> Option<&ExpansionConfig> {
    config
        .expansion
        .iter()
        .find(|expansion| is_onboarding_snippet(expansion))
}

/// Only the dedicated harmless tutorial is used for the automatic first-run
/// check. User snippets may contain private or unexpectedly large text, so
/// they are not repurposed as onboarding test output.
pub(super) fn onboarding_test_snippet(config: &Config) -> Option<&ExpansionConfig> {
    onboarding_snippet(config)
}

impl GuiApp {
    /// Add one harmless, globally applicable tutorial snippet the first time
    /// onboarding needs a real insertion test. Existing testable snippets are
    /// always reused, and the preference prevents an intentionally removed
    /// tutorial from reappearing on every launch.
    pub(crate) fn ensure_onboarding_test_snippet(&mut self) {
        let config_identity = self.path.to_string_lossy().into_owned();
        if !self.onboarding_visible
            || self.onboarding_test_snippet_created_for.as_deref() == Some(config_identity.as_str())
            || onboarding_test_snippet(&self.config).is_some()
            || self
                .pending_save
                .as_ref()
                .is_some_and(|pending| onboarding_test_snippet(&pending.candidate).is_some())
            || self
                .queued_save
                .as_ref()
                .is_some_and(|pending| onboarding_test_snippet(&pending.candidate).is_some())
            || self
                .conflicted_save
                .as_ref()
                .is_some_and(|pending| onboarding_test_snippet(&pending.candidate).is_some())
            || self
                .failed_save
                .as_ref()
                .is_some_and(|pending| onboarding_test_snippet(&pending.candidate).is_some())
        {
            return;
        }
        if !self.create_test_snippet() || self.failed_save.is_some() {
            return;
        }
    }

    /// Record the tutorial only after its config save has committed. The
    /// preference prevents a deliberately removed example from returning, so
    /// writing it before an asynchronous config save succeeds could strand a
    /// first-run library with no example at all.
    pub(crate) fn record_onboarding_test_snippet_saved(&mut self) -> Result<(), String> {
        let config_identity = self.path.to_string_lossy().into_owned();
        if !self.onboarding_visible
            || self.onboarding_test_snippet_created_for.as_deref() == Some(config_identity.as_str())
            || onboarding_test_snippet(&self.config).is_none()
        {
            return Ok(());
        }
        self.onboarding_test_snippet_created_for = Some(config_identity);
        save_gui_prefs(
            self.language,
            self.colorpack,
            self.settings_font_scale,
            self.dark_mode,
            self.onboarding_completed,
            self.onboarding_test_snippet_created_for.clone(),
        )
        .map_err(|error| error.to_string())
    }

    /// Persist the pending-test state so a previously completed guide reopens
    /// if setup was changed and the user postpones verification.
    pub(crate) fn require_desktop_test(&mut self) -> Option<String> {
        self.onboarding_completed = false;
        self.onboarding_visible = true;
        self.desktop_test_dialog_required = true;
        self.desktop_test_verified = false;
        self.desktop_test_verified_application = None;
        self.ensure_onboarding_test_snippet();
        save_gui_prefs(
            self.language,
            self.colorpack,
            self.settings_font_scale,
            self.dark_mode,
            false,
            self.onboarding_test_snippet_created_for.clone(),
        )
        .err()
        .map(|error| error.to_string())
    }

    pub(super) fn start_onboarding_desktop_test(&mut self) {
        if self.active_route_is_terminal_scoped() {
            // This route cannot capture the controlled GTK field. Default the
            // test dialog to a terminal, where the active route can actually
            // receive the test trigger.
            self.desktop_test_application_class = "Terminal".into();
        }
        self.open_desktop_test(true);
    }

    /// Open a real insertion test from troubleshooting without reopening the
    /// mandatory first-run guide or clearing an existing setup confirmation.
    pub(crate) fn start_desktop_integration_test(&mut self) {
        self.open_desktop_test(false);
    }

    fn open_desktop_test(&mut self, require_onboarding: bool) {
        if !self.select_default_desktop_test() {
            self.status = Status::warning(self.strings.status_create_test_expansion_first());
            if require_onboarding {
                return;
            }
        }
        if require_onboarding {
            if let Some(error) = self.require_desktop_test() {
                self.status =
                    Status::warning(self.strings.status_real_app_test_state_unsaved(&error));
            }
        }
        self.test_expansion_dialog_open = true;
    }

    pub(crate) fn finish_onboarding(&mut self) {
        if !self.expansion_observed() {
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
            let mut record = crate::setup_record::SetupRecord::skipped();
            record.inherit_application_checks(self.setup_record.as_ref());
            if crate::setup_record::save(&record).is_ok() {
                self.setup_record = Some(record);
            }
        }
        self.persist_onboarding(self.strings.onboarding_skipped_note());
    }

    /// Finish first run by choosing the safe copy-and-paste workflow. This
    /// closes the guide without claiming that automatic insertion works.
    pub(crate) fn finish_onboarding_with_copy_only(&mut self) {
        let already_verified = self
            .setup_record
            .as_ref()
            .is_some_and(|record| record.outcome == crate::setup_record::SetupOutcome::Verified);
        if !already_verified {
            let mut record = crate::setup_record::SetupRecord::copy_only();
            record.inherit_application_checks(self.setup_record.as_ref());
            if crate::setup_record::save(&record).is_err() {
                self.setup_record_saved = false;
                self.status = Status::warning(self.strings.onboarding_copy_only_not_saved());
                return;
            }
            self.setup_record = Some(record);
        }
        self.setup_record_saved = true;
        self.persist_onboarding(self.strings.onboarding_copy_only_note());
    }

    fn use_quick_picker_for_first_run(&mut self) {
        self.open_copy_only_picker();
        if !self.status.is_error() {
            self.finish_onboarding_with_copy_only();
        }
    }

    pub(crate) fn defer_onboarding(&mut self) {
        // Let users continue working for this session while preserving the
        // incomplete state. They can still finish with the copy-only workflow
        // without claiming a real-application verification.
        self.onboarding_visible = false;
        self.status = Status::warning(self.strings.status_setup_deferred());
    }

    pub(crate) fn open_copy_only_picker(&mut self) {
        match crate::picker::focus_existing_picker() {
            Ok(true) => {
                self.status = Status::info(self.strings.status_picker_copy_only());
                return;
            }
            Ok(false) => {}
            Err(error) => {
                self.status =
                    Status::error(self.strings.status_picker_open_failed(error.to_string()));
                return;
            }
        }
        let executable = match env::current_exe() {
            Ok(executable) => executable,
            Err(error) => {
                self.status = Status::error(
                    self.strings
                        .status_picker_executable_missing(error.to_string()),
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
                    Status::error(self.strings.status_picker_open_failed(error.to_string()));
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
            self.onboarding_test_snippet_created_for.clone(),
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
        self.ensure_onboarding_test_snippet();
        self.refresh_diagnostics(false);
    }

    fn cancel_gnome_setup_resume(&mut self) {
        self.start_cli_task(
            vec!["setup".into(), "--cancel-resume".into()],
            self.strings.gnome_resume_title().into(),
            self.strings.gnome_resume_cancelled().into(),
        );
    }

    /// Keep the first-run guide available after saving the tutorial snippet.
    /// The preview below checks matching locally; the user separately confirms
    /// whether a real application received the replacement.
    /// Readiness uses the same evidence model as the next-step adviser.
    /// `wayexpand-control.service` is deliberately not a substitute for the
    /// GNOME IBus engine.
    pub(crate) fn gnome_ibus_route_ready(&self) -> bool {
        super::next_step::gnome_ibus_route_configured(&self.situation())
    }

    pub(crate) fn render_first_run(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let gnome_resume_marker_present =
            wayexpand_core::gnome_ibus_setup_resume_marker_present(&self.path);
        let desktop = env::var("XDG_CURRENT_DESKTOP")
            .or_else(|_| env::var("XDG_SESSION_DESKTOP"))
            .unwrap_or_else(|_| self.strings.unknown_desktop().into());
        let is_wayland = env::var_os("WAYLAND_DISPLAY").is_some();
        let app_context_available = self.backend_status.iter().any(|status| {
            status.kind == wayexpand_core::BackendKind::WindowTracker
                && status.state == BackendState::Available
        });
        let route_contract = self
            .recommended_route
            .map(|route| route.contract())
            .or_else(|| {
                self.route_plan
                    .as_ref()
                    .and_then(|plan| plan.routes.first().map(|route| route.contract))
            });
        let recommendation = route_contract.and_then(|route| {
            route_recommendation(
                route,
                self.route_plan.as_ref(),
                self.selection_capabilities.as_ref(),
                &self.backend_status,
                self.ibus_route.as_ref(),
            )
        });
        let (recommended_route, recommended_route_detail) =
            self.strings.onboarding_recommended_route(recommendation);
        let daemon_running = self.daemon_reachable == Some(true);
        // The control-only service (stdin source) runs the Quick Picker and
        // hotkeys and captures nothing; it is a typing route only through
        // the IBus engine beside it.
        let control_only = daemon_running
            && runtime::status_field(&self.daemon_status, "source").as_deref() == Some("stdin");
        // The IBus route needs no typing service; a live engine means
        // WayExpand is already on (the same rule as `next_step`).
        // Until the first status answer arrives, "not running" would be a
        // guess, and offering Turn on could start setup for a running service.
        let route_connected = daemon_running
            && !control_only
            && !self.paused
            && self.route_state == Some(runtime::RouteState::Connected);
        // A live input/output connection is not usable when the daemon has
        // explicitly disabled matching (for example, terminal-only capture
        // without a verified focused-app identity). Keep connection health
        // visible in details, but do not mark the first onboarding step ready
        // or offer a desktop insertion test that the current route will refuse.
        let daemon_matching_unavailable =
            runtime::status_field(&self.daemon_status, "matching_available").as_deref()
                == Some("false");
        let daemon_build = daemon_running.then(|| runtime::daemon_build_state(&self.daemon_status));
        let terminal_only_daemon =
            daemon_running && super::next_step::terminal_route_is_scoped(&self.daemon_status);
        let gnome_ibus = uses_gnome_ibus_route_with_scope(
            wayexpand_backend_selection::Compositor::detect()
                == wayexpand_backend_selection::Compositor::Gnome
                || self.ibus_route.as_ref().is_some_and(|report| {
                    report.desktop == wayexpand_backend_ibus::chain::IbusDesktop::Gnome
                }),
            self.ibus_route
                .as_ref()
                .and_then(|report| report.check("gnome-source-active"))
                .is_some_and(|check| {
                    check.state == wayexpand_backend_ibus::chain::IbusCheckState::Pass
                }),
            self.ibus_route
                .as_ref()
                .and_then(|report| report.check("gnome-source-registered"))
                .is_some_and(|check| {
                    check.state == wayexpand_backend_ibus::chain::IbusCheckState::Pass
                }),
            self.recommended_route
                .is_some_and(|route| route.setup_backend() == "ibus"),
            self.daemon_reachable,
            runtime::status_field(&self.daemon_status, "source").as_deref(),
            terminal_only_daemon,
        );
        let kde_ibus = wayexpand_backend_selection::Compositor::detect()
            == wayexpand_backend_selection::Compositor::KdePlasma
            && terminal_only_daemon
            && (self.ibus_active
                || self.ibus_route.as_ref().is_some_and(|report| {
                    report.desktop == wayexpand_backend_ibus::chain::IbusDesktop::KdeWayland
                })
                || self
                    .recommended_route
                    .is_some_and(|route| route.setup_backend() == "ibus"));
        let ibus_only = self.ibus_route_active() && (!daemon_running || control_only);
        let terminal_scoped_active_route =
            terminal_only_daemon && !ibus_only && !gnome_ibus && !kde_ibus;
        let checking = self.daemon_reachable.is_none() && !ibus_only && !gnome_ibus && !kde_ibus;
        // A running IBus engine is only a configured GNOME route when GNOME
        // has registered and actually selected the source. A connection by
        // itself must never look like a working desktop integration.
        let route_ready = if self.installation_audit.ready == Some(false) {
            false
        } else if gnome_ibus {
            self.gnome_ibus_route_ready()
        } else if kde_ibus {
            self.ibus_route_active()
                && self.ibus_check_state("manifest-scope")
                    == wayexpand_backend_ibus::chain::IbusCheckState::Pass
                && !(self.ibus_route.as_ref().is_some_and(|report| {
                    report.check("desktop").is_some_and(|check| {
                        check.problem_id == Some("WX-KWIN-IBUS-001")
                            && check.state == wayexpand_backend_ibus::chain::IbusCheckState::Fail
                    })
                }))
                && !self.paused
        } else {
            ibus_only && !self.paused
                || route_connected
                    && !daemon_matching_unavailable
                    && daemon_build == Some(runtime::DaemonBuildState::Current)
        };
        let test_snippet = onboarding_test_snippet(&self.config)
            .map(|expansion| (expansion.trigger.clone(), expansion.replacement.clone()));
        let picker_policy = crate::picker::PickerPolicy::merge(
            &self.config.organization,
            self.administrator_policy.as_ref(),
        );
        let picker_copy_allowed = picker_policy.clipboard_allowed;
        let picker_readiness = self.picker_readiness();
        let picker_policy_notice = (!self.config.expansion.is_empty() && !picker_copy_allowed)
            .then(|| match picker_readiness {
                crate::picker::PickerReadiness::TypesDirectly => {
                    self.strings.first_run_picker_direct_shortcut().to_owned()
                }
                readiness => self.strings.picker_readiness_detail(readiness),
            });
        let sample_trigger = test_snippet
            .as_ref()
            .map(|(trigger, _)| trigger.clone())
            .unwrap_or_else(|| ONBOARDING_TRIGGER.to_owned());
        let matcher_passed = test_snippet.is_some()
            && self.playground.last_trigger.as_deref() == Some(sample_trigger.as_str())
            && self.playground.last_match_app.as_deref() == Some(self.preview_app.trim())
            && test_snippet
                .as_ref()
                .is_some_and(|(_, replacement)| self.playground.text.contains(replacement));
        // A pending or stale diagnostic is not yet a setup failure. Keep the
        // headline aligned with the connection step while it refreshes or
        // reconnects, and reserve the attention state for an observed blocker.
        let setup_check_pending = checking
            || matches!(
                self.current_next_step().map(|(problem, _)| problem),
                Some(
                    super::Problem::InputStatusPending
                        | super::Problem::InputStatusStale
                        | super::Problem::GnomeSourceStatusUnavailable
                        | super::Problem::ReplacementConsentUnknown
                        | super::Problem::Connecting
                        | super::Problem::StatusUnknown
                        | super::Problem::RouteStatusUnknown
                )
            );
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
        let detail = |key: &str| self.strings.onboarding_route_detail(key).to_owned();
        let route_state_detail = match self.route_state {
            _ if self.paused => detail("paused"),
            // A companion's own route state says nothing about the IBus route.
            _ if route_ready && terminal_scoped_active_route => detail("terminal_only_ready"),
            _ if ibus_only || (gnome_ibus || kde_ibus) && route_ready => detail("typing_ready"),
            Some(runtime::RouteState::PermissionRequired) => detail("permission_required"),
            Some(runtime::RouteState::PortalRevoked) => detail("portal_revoked"),
            Some(runtime::RouteState::Unsupported) => detail("unsupported"),
            Some(runtime::RouteState::Failed) => detail("failed"),
            Some(runtime::RouteState::Reconnecting) => detail("reconnecting"),
            Some(runtime::RouteState::Starting) => detail("starting"),
            Some(runtime::RouteState::Degraded) => detail("degraded"),
            Some(runtime::RouteState::Stopped) => detail("stopped"),
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
            Some(runtime::RouteState::Connected) => detail("typing_ready"),
            None if !daemon_running => detail("service_not_running"),
            None => detail("waiting"),
        };
        let route_assurance = self
            .daemon_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.replacement_assurance)
            .map(|assurance| self.strings.replacement_explanation(assurance));
        // IBus owns replacement consent on both GNOME and KDE. The
        // companion control daemon's injection capabilities are unrelated
        // and may be empty while the selected IBus engine is working.
        let replacement_test_admitted = if gnome_ibus || kde_ibus || ibus_only {
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
            self.expansion_observed(),
            self.verification_status().is_some_and(|(record, stale)| {
                stale.is_none()
                    && record.verification_method
                        == Some(crate::setup_record::VerificationMethod::MachineObserved)
            }),
        );
        let compatibility_replacement_active = compatibility_replacement_is_active(
            route_ready,
            replacement_test_admitted,
            self.daemon_capabilities
                .as_ref()
                .and_then(|capabilities| capabilities.inject_atomic_replace),
            gnome_ibus || kde_ibus || ibus_only,
        );
        let detected_gnome_version = self
            .compositor_info
            .strip_prefix("GNOME Shell · ")
            .filter(|version| *version != "version unavailable")
            .map(str::to_owned);
        let detected_gnome_major = detected_gnome_version
            .as_deref()
            .and_then(|version| version.split('.').next())
            .filter(|major| !major.is_empty() && major.chars().all(|ch| ch.is_ascii_digit()))
            .map(str::to_owned);

        let compact_first_run = ui.available_width() < 400.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("first_run_guide")
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(if compact_first_run { 4.0 } else { 20.0 });
                    theme::brand_mark(
                        ui,
                        if compact_first_run { 30.0 } else { 36.0 },
                        palette.accent,
                    );
                    ui.label(
                        RichText::new(self.strings.welcome_title())
                            .size(if compact_first_run { 24.0 } else { 26.0 })
                            .strong()
                            .color(ui.visuals().strong_text_color()),
                    );
                    ui.label(
                        RichText::new(if !picker_copy_allowed {
                            self.strings.welcome_intro_insert_only()
                        } else if test_snippet.is_some() {
                            self.strings.welcome_intro()
                        } else {
                            self.strings.welcome_intro_needs_example()
                        })
                        .color(palette.muted),
                    );
                });
                ui.add_space(if compact_first_run { 8.0 } else { 14.0 });
                if self.gnome_setup_resume_pending {
                    theme::card(ui, palette, |ui| {
                        ui.set_width(ui.available_width());
                        ui.colored_label(palette.warning, self.strings.gnome_resume_title());
                        ui.label(self.strings.gnome_resume_body());
                        ui.horizontal_wrapped(|ui| {
                            if theme::primary_button(
                                ui,
                                palette,
                                self.strings.gnome_resume_button(),
                            )
                            .clicked()
                            {
                                self.start_guided_setup();
                            }
                            if ui
                                .button(self.strings.gnome_resume_cancel_button())
                                .clicked()
                            {
                                self.cancel_gnome_setup_resume();
                            }
                        });
                    });
                    ui.add_space(10.0);
                } else if gnome_resume_marker_present {
                    theme::card(ui, palette, |ui| {
                        ui.set_width(ui.available_width());
                        ui.colored_label(palette.warning, self.strings.gnome_resume_stale_title());
                        ui.label(self.strings.gnome_resume_stale_body());
                        if ui
                            .button(self.strings.gnome_resume_cancel_button())
                            .clicked()
                        {
                            self.cancel_gnome_setup_resume();
                        }
                    });
                    ui.add_space(10.0);
                }
                theme::card(ui, palette, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(if setup_check_pending {
                                self.strings.first_run_checking()
                            } else {
                                first_run_outcome_label(&self.strings, outcome)
                            })
                            .strong()
                            .color(if setup_check_pending {
                                palette.muted
                            } else {
                                match outcome {
                                    FirstRunOutcome::WorkingInTestedApplication => palette.success,
                                    _ => palette.warning,
                                }
                            }),
                        );
                        // Shortcut status arrives with the first asynchronous
                        // diagnostics snapshot. Do not invite a user to set a
                        // shortcut until the desktop explicitly reports that
                        // setting one is supported and none is configured.
                        // The Quick Picker copy path already works without it.
                        if self.diagnostics_checked_at.is_some()
                            && (picker_copy_allowed
                                || picker_readiness
                                    == crate::picker::PickerReadiness::TypesDirectly)
                            && self.picker_shortcut.as_ref().is_some_and(|shortcut| {
                                shortcut.supported && shortcut.keys.is_none()
                            })
                            && ui
                                .add_enabled(
                                    self.setup_task.is_none(),
                                    egui::Button::new(
                                        self.strings.shortcut_use_keys(DEFAULT_PICKER_SHORTCUT),
                                    ),
                                )
                                .clicked()
                        {
                            self.set_picker_shortcut();
                        }
                        if !self.config.expansion.is_empty() && picker_copy_allowed {
                            // When automatic expansion is unavailable, make
                            // the working copy-and-paste path the clear next
                            // action. Keep it secondary once a route is ready.
                            let quick_picker_button = if route_ready {
                                ui.button(self.strings.first_run_use_quick_picker())
                            } else {
                                theme::primary_button(
                                    ui,
                                    palette,
                                    self.strings.first_run_use_quick_picker(),
                                )
                            };
                            if quick_picker_button.clicked() {
                                self.use_quick_picker_for_first_run();
                            }
                        }
                        if ui.small_button(self.strings.onboarding_defer()).clicked() {
                            self.defer_onboarding();
                        }
                        // Skipping is always available, including when the
                        // only snippet is the tutorial. It records no
                        // verification and can be reversed from Getting
                        // started in the menu.
                        if ui.small_button(self.strings.onboarding_skip()).clicked() {
                            self.skip_onboarding();
                        }
                    });
                    if let Some(notice) = picker_policy_notice {
                        ui.label(RichText::new(notice).color(palette.warning));
                    }
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
                        ui.label(
                            RichText::new(self.strings.onboarding_desktop(&desktop, is_wayland))
                                .small()
                                .color(palette.muted),
                        );
                        ui.label(
                            RichText::new(self.strings.onboarding_integration_state(
                                checking,
                                route_ready,
                                recommendation,
                            ))
                            .strong()
                            .color(if route_ready {
                                palette.success
                            } else {
                                palette.muted
                            }),
                        );
                        if checking {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    RichText::new(self.strings.first_run_checking())
                                        .color(palette.muted),
                                );
                            });
                        } else if !(daemon_running
                            || ibus_only
                            || self.current_next_step().is_some_and(|(problem, _)| {
                                problem == super::Problem::InstallationIncomplete
                            }))
                        {
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
                                self.strings.first_run_connected(&route_state_detail),
                            );
                            if compatibility_replacement_active {
                                ui.colored_label(
                                    palette.warning,
                                    self.strings.compatibility_replacement_active(),
                                );
                            }
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
                                            Some(self.strings.gnome_register_help().to_owned())
                                        }
                                        super::Problem::InputSourceNotSelected => {
                                            Some(self.strings.gnome_select_help().to_owned())
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
                                    if self.can_offer_recommended_ibus_setup(problem)
                                        && theme::secondary_button(
                                            ui,
                                            palette,
                                            self.strings.onboarding_set_up_recommended_ibus(),
                                        )
                                        .clicked()
                                    {
                                        self.start_guided_setup();
                                    }
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
                        if !route_ready
                            && theme::secondary_button(
                                ui,
                                palette,
                                self.strings.onboarding_open_diagnostics(),
                            )
                            .clicked()
                        {
                            self.diagnostics_open = true;
                            self.refresh_diagnostics(false);
                        }
                        // A healthy terminal-scoped route can still be too
                        // narrow for a user's browser/editor workflow. Keep the
                        // capability recommendation actionable during first run
                        // even when the current route itself has no failure.
                        let current_backend = super::setup::current_guided_setup_backend(
                            self.daemon_reachable,
                            &self.daemon_status,
                            self.route_plan.as_ref(),
                        );
                        if route_ready
                            && self
                                .recommended_route
                                .is_some_and(|route| route.setup_backend() == "ibus")
                            && current_backend.is_some_and(|backend| backend != "ibus")
                        {
                            ui.label(
                                RichText::new(self.strings.onboarding_ibus_field_limitation())
                                    .small()
                                    .color(palette.muted),
                            );
                            if theme::secondary_button(
                                ui,
                                palette,
                                self.strings.onboarding_set_up_recommended_ibus(),
                            )
                            .clicked()
                            {
                                self.start_guided_setup();
                            }
                        }
                    });
                    ui.add_space(12.0);

                    // 2. Use the harmless example, or add it if needed.
                    onboarding_step(
                        ui,
                        palette,
                        2,
                        if test_snippet.is_some() {
                            self.strings.step_example_ready_title()
                        } else {
                            self.strings.step_create_title()
                        },
                        test_snippet.is_some(),
                    );
                    ui.indent("first_run_snippet", |ui| {
                        if let Some((trigger, replacement)) = &test_snippet {
                            ui.label(self.strings.first_run_test_trigger_label());
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(
                                    palette.success,
                                    RichText::new(trigger).strong().monospace(),
                                );
                                if theme::secondary_button(
                                    ui,
                                    palette,
                                    self.strings.first_run_copy_trigger(),
                                )
                                .clicked()
                                {
                                    ui.ctx().copy_text(trigger.clone());
                                    self.status =
                                        Status::info(self.strings.first_run_trigger_copied());
                                }
                            });
                            ui.label(crate::lang::fill(
                                self.strings.first_run_expected_output(),
                                &[replacement],
                            ));
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
                        matches!(
                            outcome,
                            FirstRunOutcome::WorkingInTestedApplication
                                | FirstRunOutcome::UserConfirmedInTestedApplication
                        ),
                    );
                    ui.indent("first_run_test", |ui| {
                        if matches!(
                            outcome,
                            FirstRunOutcome::WorkingInTestedApplication
                                | FirstRunOutcome::UserConfirmedInTestedApplication
                        ) {
                            let saved_application = self
                                .verification_status()
                                .filter(|(_, staleness)| staleness.is_none())
                                .and_then(|(record, _)| record.application.as_deref());
                            let application = self
                                .desktop_test_verified_application
                                .as_deref()
                                .or(saved_application)
                                .unwrap_or(self.strings.the_tested_application());
                            ui.colored_label(
                                palette.success,
                                if outcome == FirstRunOutcome::WorkingInTestedApplication {
                                    self.strings.machine_verified_in_application(application)
                                } else {
                                    self.strings.confirmed_in_application(application)
                                },
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
                                self.review_replacement_consent();
                            }
                        } else {
                            ui.label(
                                RichText::new(if terminal_scoped_active_route {
                                    self.strings.first_run_terminal_test_help()
                                } else {
                                    self.strings.first_run_test_help()
                                })
                                .color(palette.muted),
                            );
                            if theme::primary_button(
                                ui,
                                palette,
                                self.strings.test_in_real_application(),
                            )
                            .clicked()
                            {
                                self.start_onboarding_desktop_test();
                            }
                        }
                        if test_snippet.is_some()
                            && outcome != FirstRunOutcome::WorkingInTestedApplication
                        {
                            if matcher_passed {
                                ui.colored_label(
                                    palette.muted,
                                    self.strings.first_run_trigger_works_inside(),
                                );
                            } else if ui.link(self.strings.first_run_try_inside()).clicked() {
                                self.open_onboarding_matcher_preview();
                            }
                        }
                        if self.onboarding_show_troubleshooting {
                            ui.add_space(8.0);
                            ui.colored_label(
                                palette.warning,
                                self.strings.onboarding_troubleshooting_title(),
                            );
                            ui.label(
                                RichText::new(crate::lang::fill(
                                    self.strings.onboarding_troubleshooting_steps(),
                                    &[&sample_trigger],
                                ))
                                .small(),
                            );
                        }
                    });
                    ui.add_space(12.0);

                    // The picker shortcut is useful, but it is not required
                    // to install or verify ordinary expansions. Keep it out
                    // of the three-step first-run path and let users opt into
                    // it after they have seen the core workflow.
                    let shortcut_keys = self
                        .picker_shortcut
                        .as_ref()
                        .and_then(|shortcut| shortcut.keys.clone());
                    egui::CollapsingHeader::new(
                        RichText::new(self.strings.step_shortcut_title()).color(palette.muted),
                    )
                    .id_salt("first_run_optional_picker_shortcut")
                    .default_open(false)
                    .show(ui, |ui| {
                        match (&self.picker_shortcut, &shortcut_keys) {
                            (_, Some(keys)) => {
                                ui.colored_label(
                                    palette.success,
                                    self.strings.shortcut_ready(keys),
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
                ui.add_space(10.0);
                // Compatibility details are useful but do not belong ahead
                // of the first working action, especially in a short window.
                theme::card(ui, palette, |ui| {
                    ui.set_width(ui.available_width());
                    if wayexpand_backend_selection::Compositor::detect()
                        == wayexpand_backend_selection::Compositor::Gnome
                    {
                        let version_hint = detected_gnome_version
                            .as_deref()
                            .map(|version| {
                                self.strings.desktop_compatibility_detected_version(version)
                            })
                            .unwrap_or_else(|| {
                                self.strings.desktop_compatibility_version_hint().to_owned()
                            });
                        ui.label(RichText::new(version_hint).small().color(palette.muted));
                    }
                    egui::CollapsingHeader::new(self.strings.desktop_compatibility_heading())
                        .id_salt("first_run_desktop_compatibility_details")
                        .default_open(false)
                        .show(ui, |ui| {
                            egui::Grid::new("first_run_desktop_compatibility")
                                .num_columns(2)
                                .striped(true)
                                .show(ui, |ui| {
                                    let rows = self.strings.desktop_compatibility_rows();
                                    let detected_major_is_listed =
                                        detected_gnome_major.as_deref().is_some_and(|major| {
                                            rows.iter().any(|(desktop, _)| {
                                                desktop.strip_prefix("GNOME ").is_some_and(
                                                    |versions| {
                                                        versions
                                                            .split(',')
                                                            .any(|version| version.trim() == major)
                                                    },
                                                )
                                            })
                                        });
                                    for (desktop, route) in rows {
                                        let is_detected =
                                            detected_gnome_major.as_deref().is_some_and(|major| {
                                                desktop.strip_prefix("GNOME ").is_some_and(
                                                    |versions| {
                                                        versions
                                                            .split(',')
                                                            .any(|version| version.trim() == major)
                                                    },
                                                ) || desktop.starts_with("Other GNOME")
                                                    && !detected_major_is_listed
                                            });
                                        if is_detected {
                                            let color = if detected_gnome_major
                                                .as_deref()
                                                .is_some_and(|major| {
                                                    matches!(major, "46" | "48" | "50" | "51")
                                                }) {
                                                palette.accent
                                            } else {
                                                palette.warning
                                            };
                                            ui.label(RichText::new(desktop).strong().color(color));
                                            ui.label(RichText::new(route).color(color));
                                        } else {
                                            ui.label(RichText::new(desktop).strong());
                                            ui.label(route);
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                });
                ui.add_space(16.0);
            });
    }
}

#[cfg(test)]
mod guided_setup_tests {
    use super::{
        compatibility_replacement_is_active, first_run_outcome, first_run_outcome_label,
        replacement_test_is_admitted, uses_gnome_ibus_route, FirstRunOutcome,
    };
    use crate::runtime::ReplacementAssurance;
    use wayexpand_core::CompatibilityMode;

    #[test]
    fn first_run_outcome_requires_a_ready_route_and_a_real_app_confirmation() {
        assert_eq!(
            first_run_outcome(false, false, false, false, false),
            FirstRunOutcome::SetupIncomplete
        );
        assert_eq!(
            first_run_outcome(false, true, true, true, true),
            FirstRunOutcome::SetupIncomplete,
            "a stale success cannot hide an unavailable route"
        );
        assert_eq!(
            first_run_outcome(true, true, true, false, false),
            FirstRunOutcome::ConfiguredAwaitingTest
        );
        assert_eq!(
            first_run_outcome(true, true, true, true, true),
            FirstRunOutcome::WorkingInTestedApplication
        );
        assert_eq!(
            first_run_outcome(true, false, true, true, true),
            FirstRunOutcome::SafeFallback,
            "a consent change invalidates the previous ready state"
        );
        assert_eq!(
            first_run_outcome(true, true, false, true, true),
            FirstRunOutcome::ConfiguredAwaitingTest,
            "removing the test snippet invalidates the previous ready state"
        );
    }

    #[test]
    fn first_run_states_have_distinct_labels_and_only_one_claims_success() {
        let strings = crate::lang::Strings::new(crate::lang::Language::English);
        assert_eq!(
            first_run_outcome_label(&strings, FirstRunOutcome::WorkingInTestedApplication),
            "Verified in the controlled test field"
        );
        assert_eq!(
            first_run_outcome_label(&strings, FirstRunOutcome::ConfiguredAwaitingTest),
            "Configured, awaiting test"
        );
        assert_eq!(
            first_run_outcome_label(&strings, FirstRunOutcome::SafeFallback),
            "Automatic replacement unavailable · copy and paste"
        );
        assert_eq!(
            first_run_outcome_label(&strings, FirstRunOutcome::SetupIncomplete),
            "Automatic expansion needs attention"
        );
    }

    #[test]
    fn visible_status_evidence_names_the_tested_application_and_method() {
        for language in [
            crate::lang::Language::English,
            crate::lang::Language::German,
            crate::lang::Language::Indonesian,
        ] {
            let strings = crate::lang::Strings::new(language);
            let observed = strings.verification_badge("GTK 3 test field", true, false);
            let confirmed = strings.verification_badge("Firefox", false, false);
            let stale = strings.verification_badge("Terminal", true, true);
            assert!(observed.contains("GTK 3 test field"));
            assert!(confirmed.contains("Firefox"));
            assert!(stale.contains("Terminal"));
            assert!(!strings.verification_scope().is_empty());
            assert!(!strings.picker_action_copy().is_empty());
            assert!(!strings.picker_action_insert().is_empty());
        }
    }

    #[test]
    fn compatibility_replacement_is_distinguished_from_atomic_and_copy_only_routes() {
        assert!(compatibility_replacement_is_active(
            true,
            true,
            Some(false),
            false
        ));
        assert!(compatibility_replacement_is_active(true, true, None, true));
        assert!(!compatibility_replacement_is_active(
            true,
            false,
            Some(false),
            false
        ));
        assert!(!compatibility_replacement_is_active(
            false,
            true,
            Some(false),
            false
        ));
        assert!(!compatibility_replacement_is_active(
            true,
            true,
            Some(true),
            false
        ));

        let strings = crate::lang::Strings::new(crate::lang::Language::English);
        assert!(strings
            .compatibility_replacement_active()
            .contains("deletes the trigger and types the replacement separately"));
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
    fn only_an_observed_gnome_source_or_setup_recommendation_selects_the_route() {
        assert!(uses_gnome_ibus_route(
            true,
            true,
            false,
            false,
            Some(true),
            Some("evdev")
        ));
        assert!(uses_gnome_ibus_route(
            true,
            false,
            false,
            true,
            Some(false),
            None
        ));
        assert!(uses_gnome_ibus_route(
            true,
            false,
            false,
            true,
            Some(true),
            Some("stdin")
        ));
        assert!(!uses_gnome_ibus_route(
            true,
            false,
            false,
            false,
            Some(false),
            None
        ));
        assert!(!uses_gnome_ibus_route(
            false,
            true,
            false,
            true,
            Some(false),
            None
        ));
        assert!(!uses_gnome_ibus_route(
            true,
            false,
            true,
            true,
            Some(true),
            Some("evdev")
        ));
        // A reachable daemon with missing source information is not proof
        // that no other typing route is active.
        assert!(!uses_gnome_ibus_route(
            true,
            false,
            false,
            true,
            Some(true),
            None
        ));
        assert!(!uses_gnome_ibus_route(
            true,
            false,
            false,
            true,
            Some(true),
            Some("input-method-v2")
        ));
        assert!(!uses_gnome_ibus_route(true, false, false, true, None, None));
    }
}

#[cfg(test)]
mod tests {
    use super::onboarding_test_snippet;
    use wayexpand_core::{Config, ExpansionConfig};

    #[test]
    fn first_run_uses_only_the_global_harmless_tutorial_snippet() {
        let config = Config::parse(
            "[[expansion]]\ntrigger = ';custom-check'\nreplacement = 'Custom check passed'\n",
        )
        .unwrap();
        assert!(onboarding_test_snippet(&config).is_none());

        let mut config = Config::parse("").unwrap();
        let mut tutorial = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ";hello".into(),
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
        assert_eq!(snippet.trigger, ";hello");
        assert_eq!(snippet.replacement, "WayExpand is working!");

        // With another testable snippet listed first, the tutorial still wins.
        let mut both = Config::parse(
            "[[expansion]]\ntrigger = ';custom-check'\nreplacement = 'Custom check passed'\n",
        )
        .unwrap();
        both.expansion.push(config.expansion[0].clone());
        assert_eq!(onboarding_test_snippet(&both).unwrap().trigger, ";hello");
    }

    #[test]
    fn invalid_tutorial_variants_are_not_selected_for_first_run() {
        let config = Config::parse(
            "[[expansion]]\ntrigger = ';hello'\nreplacement = 'WayExpand is working! {{field:review}}'\ntags = ['tutorial']\n",
        )
        .unwrap();
        assert!(onboarding_test_snippet(&config).is_none());
    }
}
