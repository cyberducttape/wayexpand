//! Diagnostics window: runtime health, route capabilities, backends, and protocol probes.

use crate::*;

/// Indices of the snippets the desktop test can use, with first run's
/// default first when `first_run`. Indices, not copies: the dialog borrows
/// trigger and replacement from the library and copies one only to start a
/// test.
pub(crate) fn desktop_test_indices(config: &Config, first_run: bool) -> Vec<usize> {
    let mut indices: Vec<usize> = config
        .expansion
        .iter()
        .enumerate()
        .filter(|(_, expansion)| super::first_run::desktop_testable(expansion))
        .map(|(index, _)| index)
        .collect();
    if first_run {
        if let Some(default) = super::first_run::onboarding_test_snippet(config) {
            if let Some(position) = indices
                .iter()
                .position(|index| std::ptr::eq(&config.expansion[*index], default))
            {
                let index = indices.remove(position);
                indices.insert(0, index);
            }
        }
    }
    indices
}

impl GuiApp {
    /// [`desktop_test_indices`], rebuilt only when the library changes
    /// (`library_revision`) or first-run ordering is toggled, instead of on
    /// every repaint of the dialog.
    pub(crate) fn cached_desktop_tests(&mut self, first_run: bool) -> Vec<usize> {
        let key = (self.library_revision, first_run);
        let stale = self
            .desktop_test_cache
            .as_ref()
            .is_none_or(|(cached, indices)| {
                *cached != key
                    || indices
                        .iter()
                        .any(|index| *index >= self.config.expansion.len())
            });
        if stale {
            self.desktop_test_cache = Some((key, desktop_test_indices(&self.config, first_run)));
        }
        self.desktop_test_cache
            .as_ref()
            .map(|(_, indices)| indices.clone())
            .unwrap_or_default()
    }
}

fn desktop_test_review_instruction(strings: &Strings, review: &DesktopTestReview) -> String {
    crate::lang::fill(
        strings.dg_in_inspect_the_field_where_you(),
        &[&review.application_class, &review.trigger],
    )
}

fn no_surrounding_text_guidance(strings: &Strings) -> &'static str {
    strings.dg_no_surrounding_text_guidance()
}

impl GuiApp {
    fn window_tracker_lifecycle_detail(
        &self,
        capabilities: &runtime::DaemonCapabilities,
    ) -> Option<String> {
        let state = capabilities.window_tracker_state.as_deref()?;
        let last_connected_age_seconds = capabilities
            .window_tracker_last_connected_unix
            .filter(|timestamp| *timestamp > 0)
            .map(|timestamp| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .saturating_sub(timestamp)
            });
        Some(
            self.strings.window_tracker_lifecycle_detail(
                state,
                capabilities
                    .window_tracker_reconnect_attempts_total
                    .unwrap_or(0),
                capabilities.window_tracker_disconnects_total.unwrap_or(0),
                capabilities
                    .window_tracker_last_failure_reason
                    .as_deref()
                    .unwrap_or("unknown"),
                last_connected_age_seconds,
                capabilities
                    .window_tracker_reconnect_backoff_ms
                    .unwrap_or(0),
            ),
        )
    }

    pub(crate) fn refresh_diagnostics(&mut self, announce: bool) {
        if self.diagnostics_running {
            return;
        }
        let request = runtime::Request::Diagnostics {
            config_path: self.path.clone(),
            announce,
        };
        let Some(sender) = self.diagnostics_sender.as_ref() else {
            return;
        };
        match sender.try_send(request) {
            Ok(()) => {
                self.diagnostics_running = true;
                if announce {
                    self.status = Status::info(self.strings.diagnostics_running());
                }
            }
            Err(_) => {
                if announce {
                    self.status = Status::warning(self.strings.background_queue_full());
                }
            }
        }
    }

    pub(crate) fn render_diagnostics(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.diagnostics_open {
            let mut open = self.diagnostics_open;
            let max_size = theme::dialog_max_size(ctx, 720.0);
            let title = self.strings.diagnostics_title();
            egui::Window::new(title)
                // Keep the area identity stable across egui releases; the
                // default title-derived identity changed between 0.34 and
                // 0.36, which made persisted/inspected dialog geometry
                // disappear after a dependency downgrade.
                .id(egui::Id::new(Some(title)))
                .order(egui::Order::Foreground)
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .constrain(true)
                .min_width(340.0_f32.min(max_size.x))
                // Windows shrink to their content; open at a readable size
                // instead of the 340 px minimum, which wrapped every row.
                .default_size(egui::vec2(720.0_f32.min(max_size.x), max_size.y))
                .max_size(max_size)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    // The report is longer than most windows are tall; scroll
                    // it rather than letting the window grow off screen.
                    egui::ScrollArea::vertical()
                        .id_salt("diagnostics_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.render_diagnostics_body(ui, palette));
                });
            if self.diagnostics_close_requested {
                open = false;
                self.diagnostics_close_requested = false;
            }
            self.diagnostics_open = open;
        }
        self.render_support_export(ctx, palette);
        self.render_setup_failure_report(ctx, palette);
        self.render_test_expansion_dialog(ctx, palette);
        self.render_desktop_test_review(ctx, palette);
        self.render_backend_test_help(ctx, palette);
        self.render_service_restart_confirmation(ctx, palette);
        self.render_reset_dialog(ctx, palette);
        self.render_config_backup_dialog(ctx, palette);
    }

    /// The prioritized answer to "why didn't my trigger expand?". Prefer the
    /// shared route-readiness model used by the header and first-run guide;
    /// use recent content-free route telemetry when that model has no pending
    /// action to offer.
    pub(crate) fn render_why_card(
        &self,
        ui: &mut egui::Ui,
        palette: &Palette,
    ) -> Option<super::NextAction> {
        // Use the same prioritized state model as the header and first-run
        // guide. This keeps the diagnosis and its recommended action from
        // disagreeing about the active route.
        let situation = self.situation();
        if let Some((problem, action)) = super::next_step::next_step(&situation) {
            let answer = self.strings.problem_text(problem);
            let instruction = self
                .strings
                .next_step_instruction(problem, situation.gnome_ibus);
            let action_label = self.strings.action_label(action);
            let mut action_requested = false;
            theme::section_header(ui, "", self.strings.why_title());
            theme::card(ui, palette, |ui| {
                ui.label(
                    RichText::new(self.strings.why_intro())
                        .small()
                        .color(palette.muted),
                );
                ui.add(egui::Label::new(RichText::new(answer).color(palette.warning)).wrap());
                ui.add(egui::Label::new(instruction).wrap());
                ui.add_space(3.0);
                let terminal_scope_blocked = matches!(
                    problem,
                    super::Problem::TerminalFocusUnknown
                        | super::Problem::TerminalCompositorIdentity
                );
                let replacement_gate_blocked = matches!(
                    problem,
                    super::Problem::ReplacementNotAllowed
                        | super::Problem::ReplacementConsentUnknown
                );
                let input_route_blocked = matches!(
                    problem,
                    super::Problem::ServiceOff
                        | super::Problem::NoTypingRoute
                        | super::Problem::Paused
                        | super::Problem::PermissionNeeded
                        | super::Problem::PermissionRevoked
                        | super::Problem::RestartNeeded
                        | super::Problem::BuildUnknown
                        | super::Problem::InputSourceNotSetUp
                        | super::Problem::GnomeSourceStatusUnavailable
                        | super::Problem::IbusManifestScopeInvalid
                        | super::Problem::IbusDesktopRouteUnavailable
                        | super::Problem::IbusDesktopRouteRecentlyCrashed
                        | super::Problem::IbusEngineNotRegistered
                        | super::Problem::InputSourceNotSelected
                        | super::Problem::InputStatusPending
                        | super::Problem::IbusEngineUnavailable
                        | super::Problem::IbusInputNotObserved
                        | super::Problem::InputStatusStale
                        | super::Problem::Connecting
                        | super::Problem::RouteFailed
                        | super::Problem::Unsupported
                        | super::Problem::TerminalRouteNoInput
                        | super::Problem::StatusUnknown
                        | super::Problem::RouteStatusUnknown
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(if terminal_scope_blocked {
                            self.strings.why_focus_scope_pattern()
                        } else if input_route_blocked {
                            self.strings.why_route_blocked_pattern()
                        } else {
                            self.strings.why_pattern_status(
                                situation.ibus_route_expected || situation.gnome_ibus,
                                situation.ibus_latest_match,
                                situation.recent_outcome,
                            )
                        })
                        .small()
                        .color(palette.muted),
                    )
                    .wrap(),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(if terminal_scope_blocked {
                            self.strings.why_focus_scope_delivery()
                        } else if input_route_blocked {
                            self.strings.why_route_blocked_delivery()
                        } else {
                            self.strings.why_delivery_status(
                                situation.ibus_route_expected || situation.gnome_ibus,
                                situation.ibus_latest_match,
                                situation.recent_outcome,
                                // A saved app test cannot imply that the
                                // currently blocked consent/policy gate is
                                // delivering output. Fresh match/refusal
                                // telemetry remains visible above.
                                situation.application_delivery_verified
                                    && !replacement_gate_blocked,
                            )
                        })
                        .small()
                        .color(palette.muted),
                    )
                    .wrap(),
                );
                // Keep the highest-priority repair as the single action, but
                // retain a known route limitation as context. The terminal
                // scope is a route property, not proof that the user's last
                // trigger was typed in the currently focused GUI window.
                if problem != super::Problem::TerminalRouteNoInput
                    && situation.terminal_apps_only
                    && situation.input_events_total == Some(0)
                    && !situation.application_delivery_verified
                {
                    ui.add(
                        egui::Label::new(
                            self.strings
                                .problem_text(super::Problem::TerminalRouteNoInput),
                        )
                        .wrap(),
                    );
                }
                if ui.button(action_label).clicked() {
                    action_requested = true;
                }
            });
            return action_requested.then_some(action);
        }
        // A saved desktop test is scoped to the application named in its
        // record. It cannot make a different, unobserved application appear
        // healthy just because the route, session, and configuration match.
        // Keep Diagnostics useful in that state without invalidating the
        // valid test result or claiming the new application is broken.
        if (situation.gnome_ibus || situation.ibus_route_expected)
            && situation.application_delivery_verified
        {
            let application = self
                .setup_record
                .as_ref()
                .and_then(|record| record.application.as_deref())
                .unwrap_or("the tested application");
            let scope_status = self.strings.why_verified_app_scope(application);
            let mut action_requested = false;
            theme::section_header(ui, "", self.strings.why_title());
            theme::card(ui, palette, |ui| {
                ui.label(
                    RichText::new(self.strings.why_intro())
                        .small()
                        .color(palette.muted),
                );
                ui.label(RichText::new(scope_status).color(palette.muted));
                if ui
                    .button(
                        self.strings
                            .action_label(super::next_step::Action::TestInApplication),
                    )
                    .clicked()
                {
                    action_requested = true;
                }
            });
            return action_requested.then_some(super::NextAction::TestInApplication);
        }
        if self.daemon_reachable != Some(true)
            || runtime::status_field(&self.daemon_status, "source").as_deref() == Some("stdin")
            || runtime::daemon_build_state(&self.daemon_status)
                != runtime::DaemonBuildState::Current
        {
            return None;
        }
        let diagnosis = wayexpand_core::ExpansionDiagnosis::from_status(
            &wayexpand_core::DaemonStatus::parse(&self.daemon_status),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let verdict = diagnosis.verdict(now);
        let (answer, fix) = self
            .strings
            .why_verdict(verdict, diagnosis.terminal_apps_only);
        let action = match verdict {
            // A harmless live test is the next useful step when no recent
            // output transaction identifies the failure. It exercises the
            // desktop path separately from the local matcher preview.
            wayexpand_core::Verdict::NoTypingSeen
            | wayexpand_core::Verdict::InputStatusUnknown
            | wayexpand_core::Verdict::NothingMatched => super::NextAction::TestInApplication,
            // Once output was attempted, inspect the target and transaction
            // details before trying anything again. In particular, a partial
            // or uncertain write must never lead to an automatic replay.
            wayexpand_core::Verdict::LastStopped(_) | wayexpand_core::Verdict::LastExpanded => {
                super::NextAction::InspectText
            }
            wayexpand_core::Verdict::MatchingOff(_) => super::NextAction::CheckAgain,
        };
        let color = match verdict {
            wayexpand_core::Verdict::LastExpanded => palette.muted,
            _ => palette.warning,
        };
        theme::section_header(ui, "", self.strings.why_title());
        let mut action_requested = false;
        theme::card(ui, palette, |ui| {
            ui.label(
                RichText::new(self.strings.why_intro())
                    .small()
                    .color(palette.muted),
            );
            ui.add(egui::Label::new(RichText::new(answer).color(color)).wrap());
            if let Some(fix) = fix {
                ui.add(egui::Label::new(RichText::new(fix).small()).wrap());
            }
            ui.add_space(4.0);
            if ui.button(self.strings.action_label(action)).clicked() {
                action_requested = true;
            }
        });
        action_requested.then_some(action)
    }

    pub(crate) fn render_diagnostics_body(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        if let Some(report) = self.setup_failure_report.as_deref() {
            ui.group(|ui| {
                ui.colored_label(palette.danger, self.strings.dg_setup_failed());
                if let Some(summary) = report
                    .lines()
                    .find_map(|line| line.strip_prefix("Summary: "))
                {
                    ui.label(summary);
                }
                if ui
                    .button(self.strings.dg_view_diagnostic_report())
                    .clicked()
                {
                    self.setup_failure_report_open = true;
                }
            });
            ui.add_space(8.0);
        }
        // Put the direct answer before the detailed route and protocol
        // inventory so users do not have to interpret diagnostics first.
        let why_action = self.render_why_card(ui, palette);
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(self.strings.runtime_dashboard())
                    .size(18.0)
                    .strong(),
            );
            if ui
                .add_enabled(
                    !self.diagnostics_running,
                    egui::Button::new(self.strings.refresh_session_report()),
                )
                .clicked()
            {
                self.refresh_diagnostics(true);
            }
            if self.diagnostics_running {
                ui.spinner();
            }
        });
        let freshness = self.diagnostics_checked_at.map(|checked| checked.elapsed());
        let freshness_text = match freshness {
            Some(age) if age.as_secs() < 60 => self.strings.report_updated_just_now().to_owned(),
            Some(age) => self
                .strings
                .report_updated_minutes_ago((age.as_secs() / 60).max(1)),
            None => self.strings.report_not_run_yet().to_owned(),
        };
        let freshness_color = if freshness.is_some_and(|age| age >= Duration::from_secs(60)) {
            palette.warning
        } else {
            palette.muted
        };
        ui.label(RichText::new(freshness_text).small().color(freshness_color));
        let active_route_id =
            compact_status_value(&self.daemon_status, "route_id").map(str::to_owned);
        let active_replacement = self
            .daemon_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.replacement_assurance)
            .map(|assurance| self.strings.replacement_pill(assurance).to_owned());
        self.render_session_overview(ui, palette, active_route_id.as_deref());
        self.render_quick_actions(ui, palette);
        self.render_ibus_route(ui, palette);
        ui.collapsing(self.strings.ranked_routes_title(), |ui| {
            ui.label(
                RichText::new(self.strings.ranked_routes_subtitle())
                    .small()
                    .color(palette.muted),
            );
            if let Some(plan) = &self.route_plan {
                for route in &plan.routes {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&route.contract.label).strong());
                            if active_route_id.as_deref() == Some(route.contract.id.as_str()) {
                                ui.colored_label(palette.accent, self.strings.active_route());
                            }
                            let standing = route.standing.as_str();
                            let color = match route.standing {
                                wayexpand_backend_selection::RouteStanding::Recommended
                                | wayexpand_backend_selection::RouteStanding::Available => {
                                    palette.success
                                }
                                wayexpand_backend_selection::RouteStanding::RequiresConsent => {
                                    palette.warning
                                }
                                wayexpand_backend_selection::RouteStanding::BlockedByPolicy
                                | wayexpand_backend_selection::RouteStanding::Unavailable => {
                                    palette.muted
                                }
                            };
                            ui.colored_label(color, standing);
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} capture · {} injection · trigger precondition {} · transaction {} · app result {} · live assurance {} · focus tracking {} · {}",
                                route.contract.capture,
                                route.contract.injection,
                                route.contract.trigger_precondition,
                                route.contract.output_transaction,
                                route.contract.application_result,
                                if active_route_id.as_deref() == Some(route.contract.id.as_str()) {
                                    active_replacement
                                        .as_deref()
                                        .unwrap_or("unknown")
                                } else {
                                    "not active"
                                },
                                if route.contract.focus_tracking {
                                    "available"
                                } else {
                                    "unavailable"
                                },
                                route.reason
                            ))
                            .small()
                            .color(palette.muted),
                        );
                        ui.label(RichText::new(format!(
                            "Assessment · installed: {} · supported: {} · authorization: {} · connection: {} · policy: {} · certification: {}",
                            route.assessment.installed.as_str(),
                            route.assessment.supported.as_str(),
                            route.assessment.authorized.as_str(),
                            route.assessment.connected.as_str(),
                            if route.assessment.policy_allowed { "allowed" } else { "blocked" },
                            if route.assessment.certified { "certified" } else { "not certified" },
                        )).small().color(palette.muted));
                        if route.contract.requires_raw_input() {
                            ui.label(RichText::new(self.strings.dg_compatibility_mode_maximum()).strong());
                            ui.label(self.strings.dg_advantage_reaches_more_applications_through_global());
                            ui.label(self.strings.dg_recommended_for_operators_who_understand_and());
                        }
                        if !route.assessment.limitations.is_empty() {
                            ui.label(RichText::new(self.strings.dg_limitations()).strong());
                            for limitation in &route.assessment.limitations {
                                ui.label(format!("• {}", limitation.description()));
                            }
                        }
                        if let Some(portal) = &route.assessment.portal {
                            if portal.provider_candidate || portal.direct_eis_socket_configured {
                                ui.label(RichText::new(format!(
                                    "Portal/EIS progress: provider candidate {}, implementation {}, interface {}, authorization {}, direct socket {}, EIS connection {}, injection negotiation {}, injection verification {}.",
                                    portal.provider_candidate,
                                    portal.implementation_discovered.as_str(),
                                    portal.interface_available.as_str(),
                                    portal.authorization.as_str(),
                                    portal.direct_eis_socket_configured,
                                    portal.eis_connected.as_str(),
                                    portal.injection_negotiated.as_str(),
                                    portal.injection_verified.as_str(),
                                )).small().color(palette.warning));
                            }
                        }
                    });
                }
            } else {
                ui.label(self.strings.not_checked());
            }
        });
        let source = compact_status_value(&self.daemon_status, "source").unwrap_or("unknown");
        let backend = compact_status_value(&self.daemon_status, "backend").unwrap_or("unknown");
        let active_route_certified = active_route_id
            .as_deref()
            .and_then(wayexpand_backend_selection::route_contract_by_id)
            .map(|contract| contract.is_certified());
        let capabilities = self.daemon_capabilities.as_ref();
        let output_p95 = capabilities.and_then(|caps| caps.injection_latency_p95_us);
        let matcher_p95 = capabilities.and_then(|caps| caps.matcher_latency_p95_us);
        let protection_available = capabilities
            .and_then(|caps| caps.capture_sensitive_focus)
            .unwrap_or(false);
        theme::card(ui, palette, |ui| {
            ui.set_width(ui.available_width());
            ui.columns(2, |columns| {
                dashboard_value(
                    &mut columns[0],
                    self.strings.capture_route(),
                    source,
                    palette.accent,
                );
                dashboard_value(
                    &mut columns[1],
                    self.strings.injection_backend(),
                    capabilities
                        .and_then(|caps| caps.injection_mode)
                        .unwrap_or(backend),
                    palette.accent,
                );
                dashboard_value(
                    &mut columns[0],
                    self.strings.route_id(),
                    active_route_id.as_deref().unwrap_or("unknown"),
                    palette.muted,
                );
                let certification_color = match active_route_certified {
                    Some(true) => palette.success,
                    Some(false) => palette.warning,
                    None => palette.muted,
                };
                dashboard_value(
                    &mut columns[1],
                    self.strings.route_certification(),
                    &self
                        .strings
                        .route_certification_value(active_route_certified),
                    certification_color,
                );
                dashboard_value(
                    &mut columns[1],
                    self.strings.compositor_version(),
                    &self.compositor_info,
                    palette.muted,
                );
                let permission_color =
                    if self.permission_health.starts_with("active route connected")
                        || self.permission_health.starts_with("granted")
                    {
                        palette.success
                    } else if self
                        .permission_health
                        .starts_with("OS permission not reported/applicable")
                    {
                        palette.muted
                    } else {
                        palette.warning
                    };
                dashboard_value(
                    &mut columns[1],
                    self.strings.permission_health(),
                    &self.permission_health,
                    permission_color,
                );
                dashboard_value(
                    &mut columns[0],
                    self.strings.matcher_latency_p95(),
                    &matcher_p95.map_or_else(
                        || self.strings.no_samples().to_owned(),
                        |value| format!("{value} µs"),
                    ),
                    palette.accent,
                );
                dashboard_value(
                    &mut columns[0],
                    self.strings.insertion_latency_p95(),
                    &output_p95.map_or_else(
                        || self.strings.no_samples().to_owned(),
                        |value| format!("{value} µs"),
                    ),
                    palette.accent,
                );
                let protection_detail = if protection_available {
                    format!(
                        "runtime signal available · {}",
                        self.strings.protection_uncertified()
                    )
                } else {
                    format!("unavailable · {}", self.strings.protection_uncertified())
                };
                dashboard_value(
                    &mut columns[1],
                    self.strings.sensitive_verification(),
                    &protection_detail,
                    if protection_available {
                        palette.warning
                    } else {
                        palette.danger
                    },
                );
            });
            ui.label(
                RichText::new(self.strings.sensitive_verification_note())
                    .small()
                    .color(palette.muted),
            );
        });

        let assessment = diagnostics::assess_route(
            self.daemon_reachable,
            self.route_state,
            capabilities,
            &self.permission_health,
        );
        if self.daemon_reachable == Some(true) {
            let build_warning = match runtime::daemon_build_state(&self.daemon_status) {
                runtime::DaemonBuildState::Current => None,
                runtime::DaemonBuildState::RestartRequired => {
                    Some(self.strings.daemon_restart_required_help())
                }
                runtime::DaemonBuildState::Unverified => {
                    Some(self.strings.daemon_build_unverified_help())
                }
            };
            if let Some(warning) = build_warning {
                ui.colored_label(palette.warning, RichText::new(warning).strong());
            }
        }
        let mut insertion_detail = match capabilities.and_then(|caps| caps.replacement_assurance) {
            Some(assurance) => self.strings.replacement_explanation(assurance).to_owned(),
            None => self.strings.insertion_assurance_unknown().to_owned(),
        };
        let unicode_detail = match (
            capabilities.and_then(|caps| caps.replacement_assurance),
            capabilities.and_then(|caps| caps.inject_full_unicode),
        ) {
            (Some(crate::runtime::ReplacementAssurance::Verified), None) => {
                Some(self.strings.insertion_unicode_unknown())
            }
            (_, Some(false)) => Some(self.strings.insertion_unicode_limited()),
            _ => None,
        };
        if let Some(detail) = unicode_detail {
            insertion_detail = format!("{insertion_detail} {detail}");
        }
        theme::section_header(ui, "", self.strings.route_assessment_title());
        theme::card(ui, palette, |ui| {
            let source = compact_status_value(&self.daemon_status, "source").unwrap_or("unknown");
            render_assessment_row(
                ui,
                palette,
                self.strings.capture_availability_label(),
                assessment.capture,
                &self.strings.capture_assessment_detail(
                    self.route_state,
                    source,
                    capabilities.and_then(|caps| caps.capture_exclusive),
                ),
                &self.strings,
            );
            render_assessment_row(
                ui,
                palette,
                self.strings.insertion_safety_label(),
                assessment.insertion,
                &insertion_detail,
                &self.strings,
            );
            render_assessment_row(
                ui,
                palette,
                self.strings.compositor_permissions_label(),
                assessment.permissions,
                &self.permission_health,
                &self.strings,
            );
            render_assessment_row(
                ui,
                palette,
                self.strings.focus_visibility_label(),
                assessment.focus,
                &self.strings.focus_assessment_detail(
                    capabilities.and_then(|caps| caps.window_tracker_connected),
                    capabilities.and_then(|caps| caps.window_identity_exact),
                ),
                &self.strings,
            );
            if let Some(detail) = capabilities
                .and_then(|capabilities| self.window_tracker_lifecycle_detail(capabilities))
            {
                ui.label(RichText::new(detail).small().color(palette.muted));
            }
            render_assessment_row(
                ui,
                palette,
                self.strings.keyboard_layout_label(),
                assessment.layout,
                &self.strings.layout_assessment_detail(
                    capabilities.and_then(|caps| caps.capture_layout_aware),
                ),
                &self.strings,
            );
        });

        let mut next_steps = Vec::new();
        if self.daemon_reachable == Some(false) {
            next_steps.push(self.strings.next_step_daemon());
        }
        if matches!(
            self.route_state,
            Some(
                wayexpand_core::DaemonRouteState::PermissionRequired
                    | wayexpand_core::DaemonRouteState::PortalRevoked
            )
        ) {
            next_steps.push(self.strings.next_step_permission());
        }
        if assessment.insertion == diagnostics::AssessmentState::Blocked {
            next_steps.push(self.strings.next_step_insertion_blocked());
        } else if assessment.insertion == diagnostics::AssessmentState::Warning {
            next_steps.push(self.strings.next_step_insertion_limited());
        }
        if assessment.focus == diagnostics::AssessmentState::Warning {
            next_steps.push(self.strings.next_step_focus());
        }
        if assessment.layout == diagnostics::AssessmentState::Warning {
            next_steps.push(self.strings.next_step_layout());
        }
        if capabilities.and_then(|caps| caps.capture_sensitive_focus) != Some(true) {
            next_steps.push(self.strings.next_step_sensitive_fields());
        }
        if terminal_only_route(&self.daemon_status) {
            next_steps.push(self.strings.next_step_terminal_only_route());
        }
        if active_route_certified != Some(true) {
            next_steps.push(self.strings.next_step_certification());
        }
        if !next_steps.is_empty() {
            theme::section_header(ui, "", self.strings.next_steps_title());
            theme::card(ui, palette, |ui| {
                for step in next_steps {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(palette.accent, "•");
                        ui.add(egui::Label::new(step).wrap());
                    });
                }
            });
        }

        ui.collapsing(self.strings.recent_expansion_outcomes(), |ui| {
            let traces = capabilities
                .map(|caps| caps.recent_expansion_outcomes.as_slice())
                .unwrap_or_default();
            if traces.is_empty() {
                ui.label(self.strings.no_recent_outcomes());
            }
            for trace in traces.iter().rev() {
                let color = if trace.application_result == "verified" {
                    palette.success
                } else if trace.completion.contains("unknown")
                    || trace.completion.contains("failed")
                    || trace.application_result == "unknown"
                {
                    palette.danger
                } else {
                    palette.warning
                };
                ui.group(|ui| {
                    ui.colored_label(
                        color,
                        RichText::new(self.strings.trace_completion(&trace.completion)).strong(),
                    );
                    ui.label(
                        RichText::new(
                            self.strings
                                .trace_application_result(&trace.application_result),
                        )
                        .small(),
                    );
                    ui.label(
                        RichText::new(
                            self.strings
                                .trace_timing(trace.duration_us, trace.expected_insert_units),
                        )
                        .small()
                        .color(palette.muted),
                    );
                    if let Some(interrupt_timing) = self
                        .strings
                        .trace_interrupt_timing(trace.interrupt_to_backend_return_us)
                    {
                        ui.label(
                            RichText::new(interrupt_timing)
                                .small()
                                .color(palette.danger),
                        );
                    }
                    if trace.progress_tracked {
                        ui.label(
                            RichText::new(
                                self.strings
                                    .trace_progress(trace.erased_units, trace.inserted_units),
                            )
                            .small(),
                        );
                    }
                    ui.label(
                        RichText::new(format!("{} · {}", trace.backend, trace.at_unix))
                            .small()
                            .color(palette.muted),
                    );
                    for (label, value) in [
                        (
                            self.strings.trace_trigger_matched(),
                            trace.trigger_recognized.as_str(),
                        ),
                        (
                            self.strings.trace_focus_admission(),
                            trace.focus_admission.as_str(),
                        ),
                        (
                            self.strings.trace_backend_selected(),
                            trace.backend_selected.as_str(),
                        ),
                        (
                            self.strings.trace_erase_acknowledgement(),
                            trace.erase_acknowledged.as_str(),
                        ),
                        (
                            self.strings.trace_insertion_attempted(),
                            if trace.insertion_attempted {
                                "yes"
                            } else {
                                "no"
                            },
                        ),
                    ] {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{label}:")).small().strong());
                            ui.label(RichText::new(value).small());
                        });
                    }
                });
            }
            ui.label(
                RichText::new(self.strings.trace_privacy_and_limits())
                    .small()
                    .color(palette.muted),
            );
        });
        ui.add_space(8.0);
        theme::section_header(ui, "", self.strings.compatibility_center());
        ui.label(
            RichText::new(self.strings.onboarding_certification_note())
                .small()
                .color(palette.warning),
        );
        let readiness = match self.daemon_capabilities.as_ref() {
            None => (
                self.strings.production_readiness_unavailable(),
                palette.muted,
            ),
            Some(capabilities) if capabilities.provides_full_protection() => {
                (self.strings.production_readiness_pending(), palette.warning)
            }
            Some(_) => (self.strings.production_readiness_limited(), palette.warning),
        };
        // Stacked rather than label-left/value-right: in a narrow dialog the
        // right-aligned value was painted over the label.
        theme::card(ui, palette, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(self.strings.production_readiness()).strong());
            ui.add(egui::Label::new(RichText::new(readiness.0).color(readiness.1)).wrap());
        });
        ui.label(
            RichText::new(self.strings.live_test_boundary())
                .small()
                .color(palette.muted),
        );
        if let Some(capabilities) = self.daemon_capabilities.as_ref() {
            if let Some(assurance) = capabilities.replacement_assurance {
                let color = match assurance {
                    // This is a backend contract, not evidence that a target
                    // application rendered the replacement.
                    runtime::ReplacementAssurance::Verified => palette.muted,
                    runtime::ReplacementAssurance::FocusTrackedBestEffort => palette.warning,
                    runtime::ReplacementAssurance::Unverified
                    | runtime::ReplacementAssurance::Blocked => palette.danger,
                    runtime::ReplacementAssurance::Unavailable => palette.muted,
                };
                theme::card(ui, palette, |ui| {
                    ui.colored_label(color, self.strings.replacement_pill(assurance));
                    ui.add(
                        egui::Label::new(self.strings.replacement_explanation(assurance)).wrap(),
                    );
                    if let Some(refused) = capabilities.output_refused_total.filter(|n| *n > 0) {
                        ui.colored_label(
                            palette.warning,
                            self.strings
                                .output_refused_notice(refused, capabilities.output_last_refusal),
                        );
                    }
                    if let Some(uncertain) = capabilities.output_uncertain_total.filter(|n| *n > 0)
                    {
                        ui.colored_label(
                            palette.danger,
                            self.strings.output_uncertain_notice(uncertain),
                        );
                    }
                    if let Some(partial) = capabilities.output_partial_total.filter(|n| *n > 0) {
                        ui.colored_label(
                            palette.danger,
                            self.strings.output_partial_notice(partial),
                        );
                    }
                    if let Some(discarded) = capabilities
                        .evdev_quarantined_events_total
                        .filter(|n| *n > 0)
                    {
                        ui.colored_label(
                            palette.warning,
                            self.strings.evdev_quarantine_notice(discarded),
                        );
                    }
                });
            }
            if capabilities.capture_sensitive_focus != Some(true) {
                theme::card(ui, palette, |ui| {
                    ui.colored_label(palette.danger, self.strings.sensitive_field_warning());
                    if capabilities.capture_source.as_deref() == Some("evdev") {
                        ui.colored_label(palette.danger, self.strings.evdev_privacy_limitation());
                        if capabilities.raw_capture_fallback_enabled == Some(true) {
                            if let Some(count) = capabilities
                                .raw_capture_app_count
                                .filter(|count| *count > 0)
                            {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            self.strings.raw_capture_app_scope_notice(count),
                                        )
                                        .color(palette.warning),
                                    )
                                    .wrap(),
                                );
                            }
                            if let Some(scope_state) =
                                capabilities.capture_app_scope_state.as_deref()
                            {
                                ui.colored_label(
                                    if scope_state == "allowed" {
                                        palette.success
                                    } else {
                                        palette.warning
                                    },
                                    self.strings.capture_app_scope_state(scope_state),
                                );
                            }
                            if capabilities.window_process_identity_state.as_deref()
                                == Some("compositor-process-id")
                            {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            self.strings.compositor_surface_focus_diagnostic(),
                                        )
                                        .color(palette.warning),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    }
                });
            }
            if capabilities.inject_atomic_replace != Some(true) {
                theme::card(ui, palette, |ui| {
                    ui.colored_label(palette.warning, self.strings.uncertain_output_warning());
                });
            }
            theme::card(ui, palette, |ui| {
                ui.label(self.strings.active_route());
                let mode = capabilities.injection_mode.unwrap_or("unknown");
                let fallback = mode.contains("keysym fallback");
                ui.horizontal(|ui| {
                    ui.label(self.strings.output_mode());
                    ui.colored_label(
                        if fallback || mode == "unknown" {
                            palette.warning
                        } else {
                            palette.success
                        },
                        mode,
                    );
                });
                if fallback {
                    ui.colored_label(palette.warning, self.strings.keysym_fallback_warning());
                }
                ui.label(
                    RichText::new(&self.daemon_status)
                        .small()
                        .color(palette.muted),
                );
                ui.add_space(4.0);
                for (key, value) in [
                    (
                        "reliable_key_state",
                        capabilities.capture_reliable_key_state,
                    ),
                    ("atomic_replace", capabilities.inject_atomic_replace),
                    ("unicode", capabilities.inject_full_unicode),
                    ("window_tracker", capabilities.window_tracker_connected),
                    ("sensitive_focus", capabilities.capture_sensitive_focus),
                    ("composition", capabilities.capture_composition_aware),
                    ("local_compose", capabilities.capture_local_compose_aware),
                    ("layout", capabilities.capture_layout_aware),
                ] {
                    self.render_capability_row(ui, palette, key, value);
                }
            });
        }
        ui.add_space(8.0);
        theme::section_header(ui, "", self.strings.runtime_health());
        ui.add_space(4.0);
        ui.label(
            RichText::new(self.strings.onboarding_certification_note())
                .color(palette.warning)
                .small(),
        );
        ui.label(
            RichText::new(self.strings.daemon())
                .color(palette.muted)
                .small(),
        );
        theme::card(ui, palette, |ui| {
            ui.add(egui::Label::new(RichText::new(&self.daemon_status).monospace()).wrap());
        });
        if self.daemon_reachable == Some(true) {
            let capabilities = self.daemon_capabilities.clone().unwrap_or_default();
            theme::section_header(ui, "", self.strings.configuration_health());
            let configuration_ok = !self.fleet_status.starts_with("invalid:");
            ui.horizontal(|ui| {
                ui.label(self.strings.configuration_file());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(
                        if configuration_ok {
                            palette.success
                        } else {
                            palette.danger
                        },
                        if configuration_ok {
                            self.strings.configuration_healthy()
                        } else {
                            self.strings.configuration_invalid()
                        },
                    );
                });
            });
            theme::section_header(ui, "", self.strings.safety());
            self.render_capability_row(
                ui,
                palette,
                "sensitive_focus",
                capabilities.capture_sensitive_focus,
            );
            self.render_capability_row(
                ui,
                palette,
                "atomic_replace",
                capabilities.inject_atomic_replace,
            );
            ui.add_space(8.0);
            theme::section_header(ui, "", self.strings.capture_guarantees());
            for (key, value) in [
                ("sensitive_focus", capabilities.capture_sensitive_focus),
                ("exclusive", capabilities.capture_exclusive),
                (
                    "reliable_key_state",
                    capabilities.capture_reliable_key_state,
                ),
                ("capture_passthrough", capabilities.capture_key_passthrough),
                ("composition", capabilities.capture_composition_aware),
                ("local_compose", capabilities.capture_local_compose_aware),
                ("layout", capabilities.capture_layout_aware),
            ] {
                self.render_capability_row(ui, palette, key, value);
            }
            theme::section_header(ui, "", self.strings.application_context());
            self.render_capability_row(
                ui,
                palette,
                "window_tracker",
                capabilities.window_tracker_connected,
            );
            if let Some(detail) = self.window_tracker_lifecycle_detail(&capabilities) {
                ui.label(RichText::new(detail).small().color(palette.muted));
            }
            self.render_capability_row(
                ui,
                palette,
                "exact_window_identity",
                capabilities.window_identity_exact,
            );
            theme::section_header(ui, "", self.strings.injection_guarantees());
            for (key, value) in [
                ("atomic_replace", capabilities.inject_atomic_replace),
                ("unicode", capabilities.inject_full_unicode),
                ("cursor", capabilities.inject_cursor_reposition),
                ("injection_passthrough", capabilities.inject_key_passthrough),
            ] {
                self.render_capability_row(ui, palette, key, value);
            }
            theme::section_header(ui, "", self.strings.latency());
            self.render_latency_row(
                ui,
                palette,
                self.strings.matcher_latency(),
                capabilities.matcher_latency_p50_us,
                capabilities.matcher_latency_p95_us,
                capabilities.matcher_latency_p99_us,
            );
            self.render_latency_row(
                ui,
                palette,
                self.strings.output_latency(),
                capabilities.injection_latency_p50_us,
                capabilities.injection_latency_p95_us,
                capabilities.injection_latency_p99_us,
            );
            if capabilities
                .output_interrupt_sample_count
                .unwrap_or_default()
                > 0
            {
                self.render_latency_row(
                    ui,
                    palette,
                    self.strings.interruption_latency(),
                    capabilities.output_interrupt_p50_us,
                    capabilities.output_interrupt_p95_us,
                    capabilities.output_interrupt_p99_us,
                );
            }
            theme::section_header(ui, "", self.strings.usage_persistence());
            theme::card(ui, palette, |ui| {
                ui.columns(2, |columns| {
                    dashboard_value(
                        &mut columns[0],
                        self.strings.usage_recorded(),
                        &counter_value(capabilities.usage_events_recorded_total),
                        palette.accent,
                    );
                    dashboard_value(
                        &mut columns[1],
                        self.strings.usage_persisted(),
                        &counter_value(capabilities.usage_events_persisted_total),
                        palette.accent,
                    );
                    let dropped = capabilities.usage_events_dropped_total;
                    dashboard_value(
                        &mut columns[0],
                        self.strings.usage_dropped(),
                        &counter_value(dropped),
                        if dropped.is_some_and(|count| count > 0) {
                            palette.danger
                        } else {
                            palette.success
                        },
                    );
                    dashboard_value(
                        &mut columns[1],
                        self.strings.usage_queue_rejected(),
                        &counter_value(capabilities.usage_queue_rejected_total),
                        palette.muted,
                    );
                    dashboard_value(
                        &mut columns[0],
                        self.strings.usage_flush_failures(),
                        &counter_value(capabilities.usage_flush_failures_total),
                        palette.muted,
                    );
                    dashboard_value(
                        &mut columns[1],
                        self.strings.usage_worker_failures(),
                        &counter_value(capabilities.usage_worker_failures_total),
                        palette.muted,
                    );
                    dashboard_value(
                        &mut columns[0],
                        self.strings.usage_max_flush(),
                        &capabilities.usage_flush_max_duration_us.map_or_else(
                            || self.strings.no_samples().to_owned(),
                            |duration| format!("{duration} µs"),
                        ),
                        palette.muted,
                    );
                });
                ui.label(
                    RichText::new(self.strings.usage_persistence_note())
                        .small()
                        .color(palette.muted),
                );
            });
        }
        ui.label(
            RichText::new(format!(
                "{}: {}",
                self.strings.fleet_layers(),
                self.fleet_status
            ))
            .small()
            .color(palette.muted),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            theme::section_header(ui, "", self.strings.backends());
            if ui
                .add_enabled(
                    !self.diagnostics_running,
                    egui::Button::new(self.strings.refresh()),
                )
                .clicked()
            {
                self.refresh_diagnostics(true);
            }
            if self.diagnostics_running {
                ui.spinner();
                ui.label(self.strings.diagnostics_running());
            }
        });
        ui.add_space(4.0);
        for status in &self.backend_status {
            let color = match status.state {
                BackendState::Available => palette.success,
                BackendState::RequiresPermission => palette.warning,
                BackendState::ActionRequired => palette.warning,
                BackendState::Implemented => palette.warning,
                BackendState::Unavailable | BackendState::NotImplemented => palette.muted,
            };
            ui.horizontal(|ui| {
                theme::pill(
                    ui,
                    self.strings.backend_state(status.state),
                    color,
                    theme::tint(color, 32),
                );
                ui.label(RichText::new(self.strings.backend_label(status.kind)).strong());
            });
            ui.label(RichText::new(&status.detail).small().color(palette.muted));
            // The exact triple `wayexpand doctor` reports, kept
            // verbatim so the GUI mirrors the documented
            // diagnostic vocabulary instead of paraphrasing it.
            ui.collapsing(self.strings.technical_details(), |ui| {
                ui.label(
                    RichText::new(format!(
                        "implementation={} · availability={} · permission={}",
                        status.implementation(),
                        status.availability(),
                        status.permission()
                    ))
                    .monospace()
                    .small()
                    .color(palette.muted),
                );
            });
            ui.add_space(6.0);
        }
        ui.separator();
        theme::section_header(ui, "", self.strings.protocol_probes());
        ui.add_space(4.0);
        for (name, detail) in &self.protocol_probes {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(name).strong());
                let warning = detail.contains("enforcement disabled")
                    || detail.contains("audit-only")
                    || detail.contains("raw keyboard");
                ui.label(RichText::new(detail).color(if warning {
                    palette.warning
                } else {
                    palette.muted
                }));
            });
        }
        ui.separator();
        if ui.button(self.strings.dg_export_support_report()).clicked() {
            self.support_export_open = true;
            self.support_preview = None;
            self.support_preview_error = None;
        }
        if let Some(action) = why_action {
            self.perform_next_step(action);
        }
    }

    fn render_quick_actions(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let matching =
            compact_status_value(&self.daemon_status, "matching_available") == Some("true");
        let output_ok = compact_status_value(&self.daemon_status, "output_health") == Some("ok");
        let config_ok = !self.fleet_status.starts_with("invalid:");
        let active_service = match compact_status_value(&self.daemon_status, "source") {
            Some("evdev") => Some("wayexpand-evdev.service"),
            Some("input-method" | "input-method-v2") => Some("wayexpand-input-method.service"),
            Some("ibus" | "stdin") => Some("wayexpand-control.service"),
            _ => None,
        };

        theme::section_header(ui, "", self.strings.dg_quick_checks_and_actions());
        theme::card(ui, palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.strings.dg_installation());
                match self.installation_audit.ready {
                    Some(true) => {
                        ui.colored_label(palette.success, self.strings.dg_valid());
                    }
                    Some(false) => {
                        ui.colored_label(palette.danger, self.strings.dg_needs_attention());
                        if self.installation_audit.path_shadow_detected {
                            ui.label(
                                RichText::new(self.strings.dg_installation_identity_issue("path"))
                                    .small()
                                    .color(palette.muted),
                            );
                        }
                        if self.installation_audit.mixed_installation {
                            ui.label(
                                RichText::new(self.strings.dg_installation_identity_issue("mixed"))
                                    .small()
                                    .color(palette.muted),
                            );
                        }
                        if !self.installation_audit.failed_components.is_empty() {
                            ui.label(
                                RichText::new(self.strings.dg_installation_components(
                                    &self.installation_audit.failed_components.join(", "),
                                ))
                                .small()
                                .color(palette.muted),
                            );
                        }
                        if ui
                            .button(
                                self.strings
                                    .action_label(crate::app::NextAction::CheckInstallation),
                            )
                            .clicked()
                        {
                            self.perform_next_step(crate::app::NextAction::CheckInstallation);
                        }
                    }
                    None => {
                        ui.colored_label(palette.muted, self.strings.dg_use_guided_setup_checks());
                    }
                }
                if ui.button(self.strings.dg_open_setup()).clicked() {
                    self.diagnostics_close_requested = true;
                    self.onboarding_visible = true;
                    self.refresh_diagnostics(false);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.strings.dg_configuration());
                ui.colored_label(
                    if config_ok {
                        palette.success
                    } else {
                        palette.danger
                    },
                    if config_ok {
                        self.strings.dg_valid()
                    } else {
                        self.strings.dg_needs_attention()
                    },
                );
                if ui.button(self.strings.dg_edit()).clicked() {
                    // The main window is the configuration editor.
                    self.diagnostics_close_requested = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.strings.dg_recovery());
                ui.label(
                    self.strings
                        .dg_reset_integration_services_or_gui_preferences(),
                );
                if ui.button(self.strings.dg_reset()).clicked() {
                    self.reset_dialog_open = true;
                }
                if ui.button(self.strings.dg_backup_restore()).clicked() {
                    self.config_backup_dialog_open = true;
                    self.restore_preview = None;
                    self.restore_confirmed = false;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.strings.dg_input_and_output());
                ui.colored_label(
                    if matching && output_ok {
                        palette.success
                    } else {
                        palette.warning
                    },
                    if matching && output_ok {
                        self.strings.dg_available()
                    } else {
                        self.strings.dg_degraded_unavailable()
                    },
                );
                if ui.button(self.strings.dg_test()).clicked() {
                    self.backend_test_help_open = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.strings.dg_user_service());
                ui.colored_label(
                    if self.daemon_reachable == Some(true) {
                        palette.success
                    } else {
                        palette.warning
                    },
                    if self.daemon_reachable == Some(true) {
                        self.strings.dg_daemon_responding()
                    } else {
                        self.strings.dg_not_responding()
                    },
                );
                if let Some(unit) = active_service {
                    if ui
                        .add_enabled(
                            self.setup_task.is_none(),
                            egui::Button::new(self.strings.dg_restart()),
                        )
                        .clicked()
                    {
                        self.service_restart_unit = Some(unit.to_owned());
                        self.service_restart_confirm_open = true;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.strings.dg_software_versions());
                let versions_match = self.daemon_reachable == Some(true)
                    && runtime::daemon_build_state(&self.daemon_status)
                        == runtime::DaemonBuildState::Current;
                let version_state = if self.daemon_reachable != Some(true) {
                    self.strings.dg_daemon_unavailable()
                } else {
                    match runtime::daemon_build_state(&self.daemon_status) {
                        runtime::DaemonBuildState::Current => self.strings.dg_versions_match(),
                        runtime::DaemonBuildState::RestartRequired => {
                            self.strings.dg_daemon_restart_required()
                        }
                        runtime::DaemonBuildState::Unverified => {
                            self.strings.dg_version_not_verified()
                        }
                    }
                };
                ui.colored_label(
                    if versions_match {
                        palette.success
                    } else {
                        palette.warning
                    },
                    version_state,
                );
                if ui
                    .add_enabled(
                        !self.diagnostics_running,
                        egui::Button::new(self.strings.dg_check_again()),
                    )
                    .clicked()
                {
                    self.refresh_diagnostics(true);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.strings.dg_test_expansion());
                ui.colored_label(
                    palette.accent,
                    self.strings.dg_choose_a_local_matcher_check_or(),
                );
                if ui.button(self.strings.dg_test_expansion_2()).clicked() {
                    self.test_expansion_dialog_open = true;
                }
            });
            if let Some(route) = self.recommended_route {
                ui.separator();
                ui.label(RichText::new(self.strings.dg_capability_based_recommendation()).strong());
                ui.label(format!(
                    "{} — {}. Based on live capability probes and organization policy; desktop name alone is not treated as proof.",
                    route.contract().label,
                    route.contract().setup_detail,
                ));
                // A running terminal-scoped route may be healthy for its
                // current apps while a broader route is still recommended.
                // Keep that recommendation actionable from Diagnostics; the
                // guided setup performs its normal preflight and consent flow
                // before applying a route change.
                let current_backend = super::setup::current_guided_setup_backend(
                    self.daemon_reachable,
                    &self.daemon_status,
                    self.route_plan.as_ref(),
                );
                if route.setup_backend() == "ibus"
                    && current_backend.is_some_and(|backend| backend != "ibus")
                {
                    ui.label(
                        RichText::new(self.strings.onboarding_ibus_field_limitation())
                            .small()
                            .color(palette.muted),
                    );
                    if ui
                        .button(self.strings.onboarding_set_up_recommended_ibus())
                        .clicked()
                    {
                        self.start_guided_setup();
                    }
                }
            } else {
                ui.separator();
                ui.colored_label(
                    palette.warning,
                    self.strings.dg_no_policy_allowed_route_is_currently(),
                );
            }
            let profile_id = self
                .selection_capabilities
                .as_ref()
                .and_then(|capabilities| match capabilities.compositor {
                    wayexpand_backend_selection::Compositor::KdePlasma => Some("kde-wayland"),
                    wayexpand_backend_selection::Compositor::Gnome => Some("gnome-wayland"),
                    wayexpand_backend_selection::Compositor::Sway => Some("sway"),
                    wayexpand_backend_selection::Compositor::Hyprland => Some("hyprland"),
                    _ => None,
                });
            if let Some(profile) =
                profile_id.and_then(wayexpand_backend_selection::desktop_integration_profile)
            {
                ui.separator();
                ui.label(
                    RichText::new(crate::lang::fill(
                        self.strings.dg_integration_profile(),
                        &[&profile.desktop],
                    ))
                    .strong(),
                );
                ui.label(crate::lang::fill(
                    self.strings.dg_recommended_capture_route_injection(),
                    &[
                        &profile.recommended_route_id,
                        &profile.supported_injection_backends.join(", "),
                    ],
                ));
                ui.label(
                    RichText::new(crate::lang::fill(
                        self.strings.dg_certification(),
                        &[&profile.last_certification_result],
                    ))
                    .color(palette.warning),
                );
                ui.collapsing(self.strings.dg_profile_details(), |ui| {
                    ui.label(RichText::new(self.strings.dg_required_permissions()).strong());
                    for item in &profile.required_permissions {
                        ui.label(format!("• {item}"));
                    }
                    ui.label(RichText::new(self.strings.dg_setup()).strong());
                    for item in &profile.setup_instructions {
                        ui.label(format!("• {item}"));
                    }
                    ui.label(RichText::new(self.strings.dg_known_limitations()).strong());
                    for item in &profile.known_limitations {
                        ui.label(format!("• {item}"));
                    }
                    let verified_applications = if profile.verified_applications.is_empty() {
                        "none".to_owned()
                    } else {
                        profile.verified_applications.join(", ")
                    };
                    ui.label(crate::lang::fill(
                        self.strings
                            .dg_verified_applications_verified_applications(),
                        &[&verified_applications],
                    ));
                });
            }
        });
    }

    fn render_backend_test_help(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.backend_test_help_open {
            return;
        }
        let mut open = self.backend_test_help_open;
        let mut close_clicked = false;
        egui::Window::new(self.strings.dg_desktop_backend_test())
            .id(egui::Id::new("diagnostics_backend_test_help"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(500.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(self.strings.dg_matcher_test()).strong());
                ui.label(self.strings.dg_use_the_editor_s_test_matcher());
                ui.label(RichText::new(self.strings.dg_desktop_test()).strong());
                ui.label(
                    RichText::new(self.strings.dg_these_one_shot_checks_test_evdev())
                        .color(palette.warning),
                );
                ui.horizontal_wrapped(|ui| {
                    for command in [
                        "wayexpand backend test --capture",
                        "wayexpand backend test --injection",
                    ] {
                        if ui
                            .button(crate::lang::fill(
                                self.strings.dg_copy_command(),
                                &[&command],
                            ))
                            .clicked()
                        {
                            ui.ctx().copy_text(command.to_owned());
                        }
                    }
                });
                if ui
                    .button(self.strings.dg_copy_portal_help_wx_portal_001())
                    .clicked()
                {
                    ui.ctx()
                        .copy_text("wayexpand help finding WX-PORTAL-001".into());
                }
                ui.label(self.strings.dg_run_the_copied_command_in_a());
                if ui.button(self.strings.dg_close()).clicked() {
                    close_clicked = true;
                }
            });
        if close_clicked {
            open = false;
        }
        self.backend_test_help_open = open;
    }

    /// Whether unsaved editor changes block the desktop test, which runs the
    /// saved configuration. A clean draft of a saved snippet does not.
    pub(crate) fn desktop_test_blocked_by_draft(&self) -> bool {
        self.new_draft || self.draft_is_dirty()
    }

    fn render_test_expansion_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.test_expansion_dialog_open {
            return;
        }
        let mut open = self.test_expansion_dialog_open;
        let mut close = false;
        let first_run_verification = self.desktop_test_dialog_required;
        // First-run offers every testable snippet, with the tutorial snippet
        // first when it exists; it is a default, not a requirement.
        let desktop_tests = self.cached_desktop_tests(first_run_verification);
        let trigger_at = |index: usize| self.config.expansion[index].trigger.as_str();
        if !desktop_tests
            .iter()
            .any(|index| trigger_at(*index) == self.desktop_test_trigger)
        {
            self.desktop_test_trigger = self
                .selected_index()
                .filter(|index| desktop_tests.contains(index))
                .or_else(|| desktop_tests.first().copied())
                .map(|index| self.config.expansion[index].trigger.clone())
                .unwrap_or_default();
        }
        let desktop_test = desktop_tests
            .iter()
            .copied()
            .find(|index| self.config.expansion[*index].trigger == self.desktop_test_trigger);
        let desktop_test_command = desktop_test.map(|index| {
            let trigger = self.config.expansion[index].trigger.replace('\'', "'\\''");
            format!("wayexpand debug --test '{trigger}'")
        });
        let application_class_valid = !self.desktop_test_application_class.trim().is_empty()
            && self.desktop_test_application_class.chars().count() <= 64
            && !self
                .desktop_test_application_class
                .chars()
                .any(char::is_control);
        egui::Window::new(self.strings.dg_test_expansion())
            .id(egui::Id::new("test_expansion_mode_picker"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(if first_run_verification {
                        self.strings.dg_complete_this_real_app_test_to()
                    } else {
                        self.strings.dg_real_insertion_test_explainer()
                    })
                    .color(palette.muted),
                );
                // First run keeps to the one test it needs; the matcher
                // test, copied command and route advice stay available from
                // Diagnostics.
                if !first_run_verification {
                    ui.label(RichText::new(self.strings.dg_matcher_test()).strong());
                    ui.label(self.strings.dg_checks_the_loaded_snippets_and_template());
                    if ui.button(self.strings.dg_open_matcher_test()).clicked() {
                        self.diagnostics_close_requested = true;
                        self.try_live_open = true;
                        close = true;
                    }
                    ui.separator();
                    ui.label(RichText::new(self.strings.dg_desktop_test()).strong());
                    ui.label(self.strings.dg_tests_the_active_route_in_a());
                }
                if desktop_tests.len() > 1 {
                    egui::ComboBox::from_id_salt("desktop_test_trigger_choice")
                        .selected_text(if self.desktop_test_trigger.is_empty() {
                            self.strings.dg_choose_a_saved_static_snippet()
                        } else {
                            &self.desktop_test_trigger
                        })
                        .show_ui(ui, |ui| {
                            for index in &desktop_tests {
                                let trigger = &self.config.expansion[*index].trigger;
                                ui.selectable_value(
                                    &mut self.desktop_test_trigger,
                                    trigger.clone(),
                                    trigger,
                                );
                            }
                        });
                }
                if let Some(index) = desktop_test {
                    let ExpansionConfig {
                        trigger,
                        replacement,
                        ..
                    } = &self.config.expansion[index];
                    ui.label(
                        RichText::new(crate::lang::fill(
                            self.strings.dg_type_this_once_trigger(),
                            &[&trigger],
                        ))
                        .heading()
                        .strong(),
                    );
                    ui.label(RichText::new(self.strings.dg_expected_replacement()).strong());
                    ui.add(
                        egui::TextEdit::multiline(&mut replacement.as_str())
                            .code_editor()
                            .interactive(false)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label(self.strings.dg_application_type_for_the_test_report());
                    egui::ComboBox::from_id_salt("desktop_test_application_type")
                        .selected_text(&self.desktop_test_application_class)
                        .show_ui(ui, |ui| {
                            for application_type in [
                                "Text editor",
                                "Browser",
                                "Terminal",
                                "Office application",
                                "Other",
                            ] {
                                ui.selectable_value(
                                    &mut self.desktop_test_application_class,
                                    application_type.to_owned(),
                                    application_type,
                                );
                            }
                        });
                });
                ui.checkbox(
                    &mut self.desktop_test_target_acknowledged,
                    self.strings.dg_i_am_focused_on_a_disposable(),
                );
                // The desktop test runs the saved configuration, so only
                // unsaved edits block it. Showing a saved snippet always
                // creates a clean draft; requiring no draft at all disabled
                // the test whenever the editor showed a snippet.
                let can_run_test = desktop_test.is_some()
                    && application_class_valid
                    && self.desktop_test_target_acknowledged
                    && !self.desktop_test_blocked_by_draft()
                    && self.setup_task.is_none();
                if ui
                    .add_enabled(
                        can_run_test,
                        egui::Button::new(self.strings.dg_start_desktop_test()),
                    )
                    .clicked()
                {
                    if let Some(index) = desktop_test {
                        let expansion = &self.config.expansion[index];
                        let (trigger, replacement) =
                            (expansion.trigger.clone(), expansion.replacement.clone());
                        self.start_desktop_test(
                            trigger,
                            replacement,
                            self.desktop_test_application_class.trim().to_owned(),
                        );
                        self.desktop_test_target_acknowledged = false;
                        close = true;
                    }
                }
                if self.desktop_test_blocked_by_draft() {
                    ui.colored_label(
                        palette.warning,
                        self.strings.dg_save_or_discard_the_current_snippet(),
                    );
                }
                if let Some(command) = &desktop_test_command {
                    if !first_run_verification
                        && ui
                            .button(self.strings.dg_copy_desktop_test_command())
                            .clicked()
                    {
                        ui.ctx().copy_text(command.clone());
                    }
                } else if first_run_verification {
                    ui.label(self.strings.dg_real_app_test_needs_snippet());
                    if ui
                        .button(self.strings.dg_add_harmless_test_snippet())
                        .clicked()
                    {
                        self.create_test_snippet();
                    }
                } else {
                    ui.label(self.strings.dg_no_enabled_static_snippet_is_available());
                    if ui
                        .button(self.strings.dg_add_harmless_test_snippet())
                        .clicked()
                    {
                        self.create_diagnostic_test_snippet();
                    }
                }
                if !first_run_verification {
                    ui.colored_label(
                        palette.warning,
                        self.strings.dg_if_the_matcher_succeeds_but_desktop(),
                    );
                }
                if ui
                    .button(if first_run_verification {
                        self.strings.dg_continue_later_setup_stays_unverified()
                    } else {
                        self.strings.dg_close()
                    })
                    .clicked()
                {
                    close = true;
                }
            });
        if close {
            open = false;
        }
        self.test_expansion_dialog_open = open;
    }

    fn render_desktop_test_review(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.desktop_test_review_open {
            return;
        }
        let Some(review) = self.desktop_test_review.as_ref() else {
            self.desktop_test_review_open = false;
            return;
        };
        let application_class = review.application_class.clone();
        let mut confirmed = false;
        let mut rejected = false;
        let mut dismissed = false;
        egui::Modal::new(egui::Id::new("desktop_test_result_review")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.colored_label(
                palette.warning,
                self.strings
                    .dg_wayexpand_observed_input_output_activity_during(),
            );
            ui.label(desktop_test_review_instruction(&self.strings, review));
            ui.label(crate::lang::fill(
                self.strings
                    .dg_capture_output_trigger_correlation_not_verified(),
                &[
                    &if review.capture_activity_observed {
                        "activity observed"
                    } else {
                        "not observed"
                    },
                    &if review.output_activity_observed {
                        "activity observed"
                    } else {
                        "not observed"
                    },
                ],
            ));
            ui.label(self.strings.dg_expected_replacement_2());
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&review.replacement).code())
                            .wrap()
                            .selectable(true),
                    );
                });
            ui.label(self.strings.dg_did_this_exact_text_appear_once());
            ui.horizontal(|ui| {
                if ui.button(self.strings.dg_yes_exact_result()).clicked() {
                    confirmed = true;
                }
                if ui.button(self.strings.dg_no_or_unsure()).clicked() {
                    rejected = true;
                }
                if ui.button(self.strings.dg_leave_unverified()).clicked() {
                    dismissed = true;
                }
            });
        });
        let mut save_error = None;
        let review = self.desktop_test_review.clone();
        let review_scope = review.as_ref().map(|review| crate::setup_record::Scope {
            route: review.route.as_deref(),
            route_instance: review.route_instance.as_deref(),
            config_revision: Some(&review.config_revision),
            desktop: &review.desktop,
            session: review.session.as_deref(),
            build: &review.build,
        });
        let current_route = self.current_route_label();
        let current_route_instance = self.current_route_instance();
        let current_desktop = crate::setup_record::current_desktop();
        let current_session = crate::setup_record::current_session();
        let current_build = crate::setup_record::current_build();
        let current_config_revision = self.config_revision.fingerprint();
        let current_scope = crate::setup_record::Scope {
            route: current_route.as_deref(),
            route_instance: current_route_instance.as_deref(),
            config_revision: Some(&current_config_revision),
            desktop: &current_desktop,
            session: current_session.as_deref(),
            build: &current_build,
        };
        let review_staleness = match (review.as_ref(), review_scope.as_ref()) {
            (Some(review), Some(scope)) => {
                crate::setup_record::SetupRecord::verified(&review.application_class, scope)
                    .staleness(&current_scope)
            }
            _ => Some(crate::setup_record::Staleness::Incomplete),
        };
        if let (true, Some(staleness)) = (confirmed, review_staleness.as_ref()) {
            self.desktop_test_verified = false;
            self.desktop_test_verified_application = None;
            self.status = Status::warning(
                self.strings
                    .setup_changed_since_test(&self.strings.setup_change(staleness)),
            );
            self.desktop_test_review = None;
        } else if confirmed {
            self.desktop_test_verified = true;
            self.desktop_test_verified_application = Some(application_class.clone());
            // Persist exactly the environment used by the test. The earlier
            // staleness check establishes that it still matches current
            // route/config/session state; never retag an old result as a
            // verification of a newer route instance.
            let record = crate::setup_record::SetupRecord::verified(
                &application_class,
                review_scope
                    .as_ref()
                    .expect("confirmed review has a captured scope"),
            );
            save_error = crate::setup_record::save(&record).err();
            self.setup_record_saved = save_error.is_none();
            self.setup_record = Some(record);
            self.desktop_test_review = None;
            if self.desktop_test_dialog_required || self.onboarding_visible {
                self.onboarding_visible = true;
                self.finish_onboarding();
            } else {
                self.status = Status::success(crate::lang::fill(
                    self.strings.dg_user_confirmed_exact_insertion_in_this(),
                    &[&application_class],
                ));
            }
        } else if rejected {
            self.desktop_test_verified = false;
            self.desktop_test_verified_application = None;
            self.status = Status::error(
                self.strings
                    .dg_desktop_insertion_was_not_confirmed_inspect(),
            );
            self.desktop_test_review = None;
            self.diagnostics_open = true;
        } else if dismissed {
            self.desktop_test_verified = false;
            self.desktop_test_verified_application = None;
            self.status = Status::warning(self.strings.dg_desktop_test_result_remains_unverified());
            self.desktop_test_review = None;
        }
        if let Some(error) = save_error {
            // The confirmation still holds for this session.
            self.status = self.status.clone().with_caveat(error);
        }
        if confirmed || rejected || dismissed {
            self.desktop_test_review_open = false;
        }
    }

    fn render_service_restart_confirmation(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.service_restart_confirm_open {
            return;
        }
        let mut open = self.service_restart_confirm_open;
        let mut close_clicked = false;
        egui::Window::new(self.strings.dg_restart_wayexpand_service())
            .id(egui::Id::new("diagnostics_restart_confirmation"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let unit = self
                    .service_restart_unit
                    .as_deref()
                    .unwrap_or("unknown service");
                ui.label(crate::lang::fill(
                    self.strings.dg_restart_unit_using_your_user_systemd(),
                    &[&unit],
                ));
                ui.colored_label(
                    palette.warning,
                    self.strings.dg_the_input_route_may_be_interrupted(),
                );
                ui.horizontal(|ui| {
                    if ui.button(self.strings.dg_restart_service()).clicked() {
                        if let Some(unit) = self.service_restart_unit.take() {
                            self.start_service_restart(unit);
                        }
                        close_clicked = true;
                    }
                    if ui.button(self.strings.cancel()).clicked() {
                        self.service_restart_unit = None;
                        close_clicked = true;
                    }
                });
            });
        if close_clicked {
            open = false;
        }
        self.service_restart_confirm_open = open;
    }

    fn render_reset_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.reset_dialog_open {
            let mut open = self.reset_dialog_open;
            let mut close_clicked = false;
            egui::Window::new(self.strings.dg_preview_recovery_reset())
                .id(egui::Id::new("diagnostics_reset_preview"))
                .order(egui::Order::Foreground)
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(500.0)
                .show(ctx, |ui| {
                    ui.label(self.strings.dg_choose_one_reset_scope_your_expansion());
                    ui.radio_value(
                        &mut self.reset_scope,
                        "integration".into(),
                        self.strings
                            .dg_integration_stop_services_and_clear_remembered(),
                    );
                    ui.radio_value(
                        &mut self.reset_scope,
                        "services".into(),
                        self.strings.dg_services_stop_and_disable_wayexpand_user(),
                    );
                    ui.radio_value(
                        &mut self.reset_scope,
                        "preferences".into(),
                        self.strings.dg_preferences_back_up_and_reset_gui(),
                    );
                    ui.separator();
                    match self.reset_scope.as_str() {
                        "integration" => {
                            ui.label(
                                self.strings
                                    .dg_preview_disable_and_stop_installed_wayexpand(),
                            );
                            ui.colored_label(
                                palette.warning,
                                self.strings.dg_the_next_evdev_or_libei_use(),
                            );
                        }
                        "services" => {
                            ui.label(
                                self.strings
                                    .dg_preview_disable_and_stop_installed_wayexpand_2(),
                            );
                        }
                        _ => {
                            ui.label(self.strings.dg_preview_create_a_private_backup_of());
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button(self.strings.dg_review_and_continue()).clicked() {
                            self.reset_confirm_open = true;
                        }
                        if ui.button(self.strings.dg_close()).clicked() {
                            close_clicked = true;
                        }
                    });
                });
            if close_clicked {
                open = false;
            }
            self.reset_dialog_open = open;
        }
        if self.reset_confirm_open {
            let mut open = self.reset_confirm_open;
            let mut close_clicked = false;
            egui::Window::new(self.strings.dg_confirm_reset())
                .id(egui::Id::new("diagnostics_reset_confirm"))
                .order(egui::Order::Foreground)
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.colored_label(
                        palette.warning,
                        crate::lang::fill(
                            self.strings.dg_apply_the_reset_now_this_cannot(),
                            &[&self.reset_scope],
                        ),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.setup_task.is_none(),
                                egui::Button::new(self.strings.dg_apply_reset()),
                            )
                            .clicked()
                        {
                            let completion = if self.reset_scope == "preferences" {
                                "GUI preferences reset; defaults take effect next launch."
                                    .to_owned()
                            } else {
                                format!(
                                    "{} reset complete; diagnostics refreshed.",
                                    self.reset_scope
                                )
                            };
                            self.start_cli_task(
                                vec![
                                    "reset".into(),
                                    format!("--{}", self.reset_scope),
                                    "--apply".into(),
                                    "--yes".into(),
                                ],
                                crate::lang::fill(
                                    self.strings.dg_applying_reset(),
                                    &[&self.reset_scope],
                                ),
                                completion,
                            );
                            close_clicked = true;
                            self.reset_dialog_open = false;
                        }
                        if ui.button(self.strings.cancel()).clicked() {
                            close_clicked = true;
                        }
                    });
                });
            if close_clicked {
                open = false;
            }
            self.reset_confirm_open = open;
        }
    }

    fn render_config_backup_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.config_backup_dialog_open {
            return;
        }
        let mut open = self.config_backup_dialog_open;
        egui::Window::new(self.strings.dg_configuration_backup_and_restore())
            .id(egui::Id::new("diagnostics_config_backup_restore"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(self.strings.dg_export_creates_a_private_no_overwrite());
                ui.separator();
                ui.label(RichText::new(self.strings.dg_export_current_configuration()).strong());
                if ui
                    .text_edit_singleline(&mut self.config_backup_path)
                    .changed()
                {
                    self.restore_preview = None;
                    self.last_cli_task_output = None;
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.setup_task.is_none() && !self.config_backup_path.trim().is_empty(),
                            egui::Button::new(self.strings.dg_export_backup()),
                        )
                        .clicked()
                    {
                        self.start_cli_task(
                            vec![
                                "backup".into(),
                                self.path.display().to_string(),
                                self.config_backup_path.trim().to_owned(),
                            ],
                            self.strings.dg_creating_configuration_backup().into(),
                            self.strings.dg_configuration_backup_created().into(),
                        );
                    }
                });
                ui.separator();
                ui.label(RichText::new(self.strings.dg_review_a_backup_to_restore()).strong());
                if ui
                    .text_edit_singleline(&mut self.config_backup_path)
                    .changed()
                {
                    self.restore_preview = None;
                    self.last_cli_task_output = None;
                }
                if ui
                    .add_enabled(
                        self.setup_task.is_none() && !self.config_backup_path.trim().is_empty(),
                        egui::Button::new(self.strings.dg_review_differences()),
                    )
                    .clicked()
                {
                    self.restore_preview = None;
                    self.restore_confirmed = false;
                    self.start_cli_task(
                        vec![
                            "restore".into(),
                            self.config_backup_path.trim().to_owned(),
                            "--check".into(),
                            "--json".into(),
                        ],
                        self.strings.dg_validating_restore_backup().into(),
                        self.strings.dg_restore_preview_validated().into(),
                    );
                    self.cli_task_kind = Some("restore-check".into());
                }
                if let Some(report) = self.restore_preview.as_ref() {
                    if report["valid"] == true && report["changes"].is_object() {
                        ui.colored_label(
                            palette.success,
                            self.strings.dg_backup_is_valid_review_the_differences(),
                        );
                        for (label, key) in [
                            (self.strings.dg_added(), "added_triggers"),
                            (self.strings.dg_removed(), "removed_triggers"),
                            (self.strings.dg_modified(), "modified_triggers"),
                        ] {
                            let values = report["changes"][key]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            ui.label(format!(
                                "{label} triggers ({}): {}",
                                values.len(),
                                values
                                    .iter()
                                    .filter_map(serde_json::Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ));
                        }
                        ui.label(crate::lang::fill(
                            self.strings.dg_settings_changed_hotkeys_changed(),
                            &[
                                &report["changes"]["settings_changed"],
                                &report["changes"]["hotkeys_changed"],
                            ],
                        ));
                        ui.checkbox(
                            &mut self.restore_confirmed,
                            self.strings.dg_restore_confirm(),
                        );
                        if ui
                            .add_enabled(
                                self.setup_task.is_none() && self.restore_confirmed,
                                egui::Button::new(
                                    self.strings.dg_restore_and_back_up_current_config(),
                                ),
                            )
                            .clicked()
                        {
                            let current_fingerprint = report["current_fingerprint"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned();
                            let backup_fingerprint = report["backup_fingerprint"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned();
                            self.start_cli_task(
                                vec![
                                    "restore".into(),
                                    self.config_backup_path.trim().to_owned(),
                                    "--apply".into(),
                                    "--yes".into(),
                                    "--json".into(),
                                    format!("--expected-current={current_fingerprint}"),
                                    format!("--expected-backup={backup_fingerprint}"),
                                ],
                                self.strings.dg_restoring_configuration().into(),
                                self.strings
                                    .dg_configuration_restored_diagnostics_refreshed()
                                    .into(),
                            );
                            self.cli_task_kind = Some("restore-apply".into());
                        }
                    } else {
                        ui.colored_label(
                            palette.danger,
                            self.strings.dg_restore_validation_did_not_return_an(),
                        );
                    }
                }
                if let Some(output) = self.last_cli_task_output.as_deref() {
                    ui.label(RichText::new(output).small().color(palette.muted));
                }
                if self.setup_task.is_some() {
                    ui.spinner();
                }
            });
        self.config_backup_dialog_open = open;
    }

    fn render_setup_failure_report(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.setup_failure_report_open {
            return;
        }
        let mut open = self.setup_failure_report_open;
        egui::Window::new(self.strings.dg_setup_diagnostic_report())
            .id(egui::Id::new("setup_failure_diagnostic_report"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(680.0, 520.0))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        self.strings
                            .dg_sanitized_setup_output_home_directory_paths(),
                    )
                    .small()
                    .color(palette.muted),
                );
                ui.horizontal(|ui| {
                    if ui.button(self.strings.dg_copy_report()).clicked() {
                        if let Some(report) = &self.setup_failure_report {
                            ui.ctx().copy_text(report.clone());
                        }
                    }
                    if ui.button(self.strings.dg_open_diagnostics()).clicked() {
                        self.diagnostics_open = true;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(report) = &self.setup_failure_report {
                            ui.add(egui::Label::new(RichText::new(report).monospace()).wrap());
                        }
                    });
            });
        self.setup_failure_report_open = open;
    }

    fn render_support_export(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.support_export_open {
            return;
        }
        let mut open = self.support_export_open;
        egui::Window::new(self.strings.dg_export_support_report())
            .id(egui::Id::new("support_report_export"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(640.0, 520.0))
            .show(ctx, |ui| {
                ui.label(RichText::new(self.strings.dg_this_report_includes()).strong());
                for item in [
                    self.strings.dg_wayexpand_version_and_desktop_session(),
                    self.strings.dg_backend_and_route_availability(),
                    self.strings.dg_service_states_exit_codes_restart_counts(),
                    self.strings
                        .dg_bounded_sanitized_event_counters_and_certification(),
                ] {
                    ui.label(format!("✓ {item}"));
                }
                ui.add_space(6.0);
                ui.label(RichText::new(self.strings.dg_this_report_excludes()).strong());
                for item in [
                    self.strings.dg_typed_text_and_keystrokes(),
                    self.strings.dg_snippet_and_replacement_contents(),
                    self.strings.dg_clipboard_contents(),
                    self.strings
                        .dg_authentication_tokens_and_raw_journal_messages(),
                    self.strings
                        .dg_configuration_paths_and_free_form_diagnostic(),
                ] {
                    ui.label(format!("✓ {item}"));
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.support_preview_loading,
                            egui::Button::new(self.strings.dg_preview_report()),
                        )
                        .clicked()
                    {
                        self.support_preview_loading = true;
                        self.support_preview_error = None;
                        if let Some(sender) = self.diagnostics_sender.as_ref() {
                            if sender.try_send(runtime::Request::SupportPreview).is_err() {
                                self.support_preview_loading = false;
                                self.support_preview_error = Some(
                                    self.strings
                                        .dg_the_background_diagnostics_queue_is_busy()
                                        .into(),
                                );
                            }
                        } else {
                            self.support_preview_loading = false;
                            self.support_preview_error = Some(
                                self.strings
                                    .dg_background_diagnostics_are_unavailable()
                                    .into(),
                            );
                        }
                    }
                    if self.support_preview_loading {
                        ui.spinner();
                        ui.label(self.strings.dg_collecting_sanitized_report());
                    }
                    if ui
                        .add_enabled(
                            self.support_preview.is_some(),
                            egui::Button::new(self.strings.dg_save_report()),
                        )
                        .clicked()
                    {
                        match self.save_support_report() {
                            Ok(()) => {
                                self.support_preview_error = Some(
                                    self.strings
                                        .dg_report_saved_with_owner_only_permissions()
                                        .into(),
                                )
                            }
                            Err(error) => self.support_preview_error = Some(error),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(self.strings.dg_save_as());
                    ui.text_edit_singleline(&mut self.support_output_path);
                });
                if let Some(error) = &self.support_preview_error {
                    ui.colored_label(palette.warning, error);
                }
                if let Some(report) = &mut self.support_preview {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(report)
                                    .code_editor()
                                    .interactive(false)
                                    .desired_rows(16)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                }
                if ui.button(self.strings.dg_desktop_test_2()).clicked() {
                    self.backend_test_help_open = true;
                }
            });
        self.support_export_open = open;
    }

    fn save_support_report(&mut self) -> Result<(), String> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let path = std::path::PathBuf::from(self.support_output_path.trim());
        if path.as_os_str().is_empty() {
            return Err(self.strings.dg_choose_a_report_path_first().into());
        }
        let report = self
            .support_preview
            .as_deref()
            .ok_or(self.strings.dg_preview_the_report_before_saving())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                crate::lang::fill(
                    self.strings.dg_could_not_create_report_file_error(),
                    &[&error],
                )
            })?;
        if let Err(error) = file
            .write_all(report.as_bytes())
            .and_then(|()| file.sync_all())
        {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(crate::lang::fill(
                self.strings.dg_could_not_write_report_error(),
                &[&error],
            ));
        }
        Ok(())
    }

    /// The IBus route, link by link, with the fix for each failure and what
    /// the engine saw in recently focused fields. Hidden when IBus is not
    /// installed at all.
    fn render_ibus_route(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        use wayexpand_backend_ibus::chain::IbusCheckState;
        let Some(report) = self.ibus_route.clone() else {
            return;
        };
        if !report.installed_anywhere() && !self.ibus_active {
            return;
        }
        ui.add_space(8.0);
        theme::section_header(ui, "", self.strings.ibus_route_title());
        let (summary, summary_color) = if let Some(check) = report.first_failure() {
            (
                self.strings.ibus_route_not_ready(check.title),
                palette.danger,
            )
        } else {
            match report.operational_readiness() {
                IbusCheckState::Pass => {
                    (self.strings.ibus_route_ready().to_owned(), palette.success)
                }
                IbusCheckState::Fail => (
                    self.strings
                        .ibus_route_not_ready("operational verification"),
                    palette.danger,
                ),
                IbusCheckState::Warn | IbusCheckState::Unknown | IbusCheckState::Skip => {
                    let incomplete = report
                        .checks
                        .iter()
                        .find(|check| {
                            matches!(check.state, IbusCheckState::Warn | IbusCheckState::Unknown)
                        })
                        .map_or("operational verification", |check| check.title);
                    (
                        self.strings.ibus_route_not_ready(incomplete),
                        palette.warning,
                    )
                }
            }
        };
        ui.label(RichText::new(summary).color(summary_color).strong());
        let readiness = report.readiness();
        let running = readiness.ibus_running && readiness.selected && readiness.engine_running;
        let running_text = if running {
            "running"
        } else if readiness.ibus_running && readiness.engine_running {
            "engine running; WayExpand is not the active source"
        } else if report
            .check("daemon")
            .is_some_and(|check| check.state == wayexpand_backend_ibus::chain::IbusCheckState::Fail)
        {
            "not running"
        } else {
            "unknown or partially running"
        };
        let verified_text = if self.desktop_test_verified {
            self.desktop_test_verified_application
                .as_deref()
                .map_or_else(
                    || "user-confirmed in an application this session".to_owned(),
                    |app| format!("user-confirmed in {app} this session"),
                )
        } else if readiness.output_emitted_unverified {
            self.strings
                .dg_output_emitted_application_result_unknown()
                .to_owned()
        } else {
            self.strings.dg_not_verified_in_an_application().to_owned()
        };
        ui.group(|ui| {
            ui.label(RichText::new(self.strings.input_method_progress_title()).strong());
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    if readiness.configuration == wayexpand_backend_ibus::chain::IbusCheckState::Pass { palette.success } else { palette.warning },
                    crate::lang::fill(self.strings.dg_configured(), &[&readiness.configuration.label()]),
                );
                ui.colored_label(
                    if running { palette.success } else if readiness.ibus_running && readiness.engine_running { palette.warning } else { palette.muted },
                    crate::lang::fill(self.strings.dg_running_running_text(), &[&running_text]),
                );
                ui.colored_label(
                    if self.desktop_test_verified { palette.success } else { palette.muted },
                    crate::lang::fill(self.strings.dg_verified_verified_text(), &[&verified_text]),
                );
            });
            if !self.desktop_test_verified {
                ui.label(RichText::new(self.strings.dg_only_a_user_confirmed_exact_result()).small().color(palette.muted));
                if ui.small_button(self.strings.dg_run_desktop_integration_test()).clicked() {
                    self.test_expansion_dialog_open = true;
                }
            }
            if let Some(output) = report
                .activity
                .as_ref()
                .and_then(|activity| activity.status.recent_outputs.last())
            {
                ui.label(
                    RichText::new(format!(
                        "Latest IBus output {}: {} · {} expected characters · {} · target identity {}",
                        output.transaction_id,
                        output.outcome,
                        output.expected_characters,
                        output.duration_us.map_or_else(
                            || "duration unavailable".to_owned(),
                            |duration| format!("{duration} µs")
                        ),
                        output.target_identity_state,
                    ))
                    .small()
                    .color(palette.warning),
                );
                ui.label(RichText::new("IBus cannot confirm that the target application applied emitted signals.").small().color(palette.muted));
            }
        });
        if report.desktop == wayexpand_backend_ibus::chain::IbusDesktop::Gnome {
            let source_registered = report
                .check("gnome-source-registered")
                .is_some_and(|check| {
                    check.state == wayexpand_backend_ibus::chain::IbusCheckState::Pass
                });
            let source_active = report.check("gnome-source-active").is_some_and(|check| {
                check.state == wayexpand_backend_ibus::chain::IbusCheckState::Pass
            }) && report.check("selected").is_some_and(|check| {
                check.state == wayexpand_backend_ibus::chain::IbusCheckState::Pass
            });
            if !source_active {
                ui.group(|ui| {
                    ui.label(
                        RichText::new(self.strings.dg_finish_gnome_input_source_selection())
                            .strong(),
                    );
                    if source_registered {
                        ui.label(self.strings.dg_wayexpand_is_registered_but_gnome_is());
                        if ui
                            .button(self.strings.dg_try_selecting_wayexpand())
                            .clicked()
                        {
                            self.retry_gnome_source_activation();
                        }
                    } else {
                        ui.label(self.strings.dg_wayexpand_is_not_yet_registered_as());
                        if ui.button(self.strings.first_run_continue_setup()).clicked() {
                            self.start_guided_setup();
                        }
                    }
                    if ui
                        .small_button(self.strings.dg_recheck_input_source_status())
                        .clicked()
                    {
                        self.refresh_diagnostics(true);
                    }
                });
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "{}: {}",
                self.strings.ibus_configuration_readiness(),
                readiness.configuration.label()
            ));
            ui.label(format!(
                "{}: {}",
                self.strings.ibus_operational_readiness(),
                readiness.operational.label()
            ));
        });
        // Installed, registered, running, selected, receiving text and
        // expanding are separate facts; show each instead of one "ready".
        ui.horizontal_wrapped(|ui| {
            for (stage, ok) in report.readiness().stages() {
                let color = if ok { palette.success } else { palette.muted };
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        if ok { "✓" } else { "○" },
                        self.strings.ibus_readiness_stage(stage)
                    ))
                    .small()
                    .color(color),
                );
            }
        });
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for check in &report.checks {
                let color = match check.state {
                    IbusCheckState::Pass => palette.success,
                    IbusCheckState::Warn => palette.warning,
                    IbusCheckState::Fail => palette.danger,
                    IbusCheckState::Skip => palette.muted,
                    IbusCheckState::Unknown => palette.muted,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(check.state.symbol()).color(color).strong());
                    ui.label(RichText::new(check.title).strong());
                });
                ui.indent(check.id, |ui| {
                    ui.label(RichText::new(&check.detail).small().color(palette.muted));
                    if let Some(problem_id) = check.problem_id {
                        ui.label(
                            RichText::new(crate::lang::fill(
                                self.strings.dg_problem_problem_id(),
                                &[&problem_id],
                            ))
                            .small()
                            .color(color),
                        );
                    }
                    if check.state != IbusCheckState::Pass {
                        if let Some(fix) = &check.fix {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!("{} {fix}", self.strings.fix_label()))
                                        .small()
                                        .color(color),
                                );
                                if ui
                                    .small_button(self.strings.copy())
                                    .on_hover_text(self.strings.copy_fix_tooltip())
                                    .clicked()
                                {
                                    ui.ctx().copy_text(fix.clone());
                                }
                            });
                        }
                    }
                });
            }
        });
        let fields = report
            .activity
            .as_ref()
            .filter(|reading| reading.live)
            .map(|reading| reading.status.recent_fields.clone())
            .unwrap_or_default();
        let typed: Vec<_> = fields.iter().filter(|field| field.keys > 0).collect();
        if !typed.is_empty() {
            ui.label(
                RichText::new(self.strings.ibus_recent_fields())
                    .small()
                    .strong(),
            );
            for field in typed.iter().rev().take(5) {
                let verdict = wayexpand_backend_ibus::IbusFieldVerdict::of(field);
                let color = match verdict {
                    wayexpand_backend_ibus::IbusFieldVerdict::OutputEmittedUnverified => {
                        palette.warning
                    }
                    wayexpand_backend_ibus::IbusFieldVerdict::ReplacementContextObserved => {
                        palette.muted
                    }
                    wayexpand_backend_ibus::IbusFieldVerdict::Sensitive
                    | wayexpand_backend_ibus::IbusFieldVerdict::Idle => palette.muted,
                    _ => palette.warning,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(verdict.label()).color(color).strong());
                    ui.label(
                        RichText::new(self.strings.ibus_field_counts(field.keys, field.expansions))
                            .small()
                            .color(palette.muted),
                    );
                });
                ui.label(
                    RichText::new(verdict.explanation())
                        .small()
                        .color(palette.muted),
                );
                if verdict == wayexpand_backend_ibus::IbusFieldVerdict::NoSurroundingText {
                    ui.label(
                        RichText::new(no_surrounding_text_guidance(&self.strings))
                            .small()
                            .color(palette.warning),
                    );
                    if ui
                        .small_button(self.strings.open_quick_picker_copy())
                        .clicked()
                    {
                        self.open_copy_only_picker();
                    }
                }
            }
        }
        ui.label(
            RichText::new(self.strings.ibus_watch_hint())
                .small()
                .color(palette.muted),
        );
    }

    fn render_session_overview(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        route_id: Option<&str>,
    ) {
        let capabilities = self.daemon_capabilities.as_ref();
        let assessment = diagnostics::assess_route(
            self.daemon_reachable,
            self.route_state,
            capabilities,
            &self.permission_health,
        );
        let failures = capabilities
            .filter(|caps| caps.recent_expansion_outcomes_reported)
            .map(|caps| {
                caps.recent_expansion_outcomes
                    .iter()
                    .filter(|trace| trace.completion != "applied")
                    .count()
            });

        let mut toggled = false;
        theme::card(ui, palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&self.compositor_info).strong());
                if let Some(route_id) = route_id {
                    theme::pill(
                        ui,
                        format!("{} · {route_id}", self.strings.active_route()),
                        palette.accent,
                        theme::tint(palette.accent, 34),
                    );
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{}:", self.strings.capture_route())).small());
                ui.label(
                    RichText::new(
                        compact_status_value(&self.daemon_status, "source").unwrap_or("unknown"),
                    )
                    .small()
                    .strong(),
                );
                ui.label(RichText::new(format!("{}:", self.strings.injection_backend())).small());
                ui.label(
                    RichText::new(
                        capabilities
                            .and_then(|caps| caps.injection_mode)
                            .or_else(|| compact_status_value(&self.daemon_status, "backend"))
                            .unwrap_or("unknown"),
                    )
                    .small()
                    .strong(),
                );
            });
            if terminal_only_route(&self.daemon_status) {
                ui.colored_label(
                    palette.warning,
                    RichText::new(self.strings.terminal_only_route_notice()).strong(),
                );
            }
            // The verdict first, in words; the evidence grid behind it is for
            // troubleshooting and starts collapsed.
            let control_only = compact_status_value(&self.daemon_status, "source") == Some("stdin");
            let ibus_only = self.daemon_reachable != Some(true) && self.ibus_route_active();
            let (state, state_color) = self.header_state(ibus_only, control_only, palette);
            let password_fields_detected =
                capabilities.and_then(|caps| caps.capture_sensitive_focus) == Some(true);
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    self.strings
                        .diagnostics_summary(state, password_fields_detected),
                )
                .size(15.0)
                .color(state_color),
            );
            ui.add_space(4.0);
            let details = egui::CollapsingHeader::new(self.strings.technical_details())
                .id_salt("diagnostics_technical_details")
                .open(Some(self.diagnostics_details_open))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(self.strings.session_evidence_note())
                            .small()
                            .color(palette.muted),
                    );
                    ui.add_space(6.0);

                    ui.columns(2, |columns| {
                        overview_value(
                            &mut columns[0],
                            self.strings.capture_availability_label(),
                            self.strings.assessment_state(assessment.capture),
                            assessment_color(assessment.capture, palette),
                        );

                        let unicode = reported_capability(
                            capabilities.and_then(|caps| caps.inject_full_unicode),
                            &self.strings,
                            CapabilityKind::Unicode,
                            palette,
                        );
                        overview_value(
                            &mut columns[1],
                            self.strings.unicode_insertion(),
                            unicode.0,
                            unicode.1,
                        );

                        let replacement = capabilities
                            .and_then(|caps| caps.replacement_assurance)
                            .map(|assurance| self.strings.replacement_pill(assurance))
                            .unwrap_or_else(|| self.strings.not_reported());
                        let replacement_color =
                            match capabilities.and_then(|caps| caps.replacement_assurance) {
                                // Route capability is not an application-level success.
                                Some(runtime::ReplacementAssurance::Verified) => palette.muted,
                                Some(runtime::ReplacementAssurance::FocusTrackedBestEffort) => {
                                    palette.warning
                                }
                                Some(
                                    runtime::ReplacementAssurance::Unverified
                                    | runtime::ReplacementAssurance::Blocked,
                                ) => palette.danger,
                                Some(runtime::ReplacementAssurance::Unavailable) | None => {
                                    palette.muted
                                }
                            };
                        overview_value(
                            &mut columns[0],
                            self.strings.replacement_assurance_label(),
                            replacement,
                            replacement_color,
                        );

                        let composition = reported_capability(
                            capabilities.and_then(|caps| caps.capture_composition_aware),
                            &self.strings,
                            CapabilityKind::Composition,
                            palette,
                        );
                        overview_value(
                            &mut columns[1],
                            self.strings.external_ime_composition(),
                            composition.0,
                            composition.1,
                        );

                        overview_value(
                            &mut columns[0],
                            self.strings.focus_visibility_label(),
                            self.strings.assessment_state(assessment.focus),
                            assessment_color(assessment.focus, palette),
                        );

                        let sensitive = reported_capability(
                            capabilities.and_then(|caps| caps.capture_sensitive_focus),
                            &self.strings,
                            CapabilityKind::Protection,
                            palette,
                        );
                        overview_value(
                            &mut columns[1],
                            self.strings.sensitive_verification(),
                            sensitive.0,
                            sensitive.1,
                        );
                    });

                    if capabilities.and_then(|caps| caps.capture_composition_aware) != Some(true) {
                        ui.label(
                            RichText::new(self.strings.composition_assessment_detail())
                                .small()
                                .color(palette.muted),
                        );
                    }

                    let permission_color = assessment_color(assessment.permissions, palette);
                    overview_value(
                        ui,
                        self.strings.compositor_permissions_label(),
                        &self.permission_health,
                        permission_color,
                    );

                    ui.separator();
                    ui.columns(3, |columns| {
                        overview_value(
                            &mut columns[0],
                            self.strings.matcher_latency_p99(),
                            &capabilities
                                .and_then(|caps| caps.matcher_latency_p99_us)
                                .filter(|value| *value > 0)
                                .map_or_else(
                                    || self.strings.no_samples().to_owned(),
                                    |us| format!("{us} µs"),
                                ),
                            palette.accent,
                        );
                        overview_value(
                            &mut columns[1],
                            self.strings.output_latency_p99(),
                            &capabilities
                                .and_then(|caps| caps.injection_latency_p99_us)
                                .filter(|value| *value > 0)
                                .map_or_else(
                                    || self.strings.no_samples().to_owned(),
                                    |us| format!("{us} µs"),
                                ),
                            palette.accent,
                        );
                        let failed_value = failures.map_or_else(
                            || self.strings.not_reported().to_owned(),
                            |count| count.to_string(),
                        );
                        overview_value(
                            &mut columns[2],
                            self.strings.recent_failures(),
                            &failed_value,
                            if failures.is_some_and(|count| count > 0) {
                                palette.warning
                            } else if failures.is_some() {
                                // No recent failed transaction is not evidence that
                                // an acknowledged output was rendered by the target.
                                palette.muted
                            } else {
                                palette.muted
                            },
                        );
                    });
                });
            toggled = details.header_response.clicked();
        });
        if toggled {
            self.diagnostics_details_open = !self.diagnostics_details_open;
        }
    }

    pub(crate) fn render_capability_row(
        &self,
        ui: &mut egui::Ui,
        palette: &Palette,
        key: &str,
        value: Option<bool>,
    ) {
        let (label, color) = match value {
            Some(true) => (self.strings.capability_available(), palette.success),
            Some(false) => (self.strings.capability_unavailable(), palette.warning),
            None => (self.strings.capability_unknown(), palette.muted),
        };
        ui.horizontal(|ui| {
            ui.label(self.strings.capability_label(key));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(color, label);
            });
        });
    }

    fn render_latency_row(
        &self,
        ui: &mut egui::Ui,
        palette: &Palette,
        label: &str,
        p50: Option<u64>,
        p95: Option<u64>,
        p99: Option<u64>,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label(label);
            // The daemon publishes zeros for an empty window; measured
            // samples always round up to at least 1 µs.
            let text = match (p50, p95, p99) {
                (Some(p50), Some(p95), Some(p99)) if p99 > 0 => {
                    format!("p50 {p50} µs · p95 {p95} µs · p99 {p99} µs")
                }
                _ => self.strings.no_samples().to_owned(),
            };
            ui.label(RichText::new(text).monospace().small().color(palette.muted));
        });
    }
}

fn compact_status_value<'a>(status: &'a str, key: &str) -> Option<&'a str> {
    status.split(" · ").find_map(|part| {
        part.strip_prefix(key)
            .and_then(|part| part.strip_prefix('='))
    })
}

fn terminal_only_route(status: &str) -> bool {
    if compact_status_value(status, "terminal_apps_only") != Some("true") {
        return false;
    }

    // `terminal_apps_only` is a legacy name for the daemon's app-scoped
    // evdev mode. Newer daemons can add explicitly opted-in browser (or other
    // application) IDs through `raw_capture_apps`; calling that route
    // terminal-only would give users incorrect compatibility guidance.
    let raw_fallback_enabled =
        compact_status_value(status, "raw_capture_fallback_enabled") == Some("true");
    let raw_capture_app_count = compact_status_value(status, "raw_capture_app_count")
        .and_then(|value| value.parse::<usize>().ok());
    !(raw_fallback_enabled && raw_capture_app_count.is_none_or(|count| count > 0))
}

fn render_assessment_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    state: diagnostics::AssessmentState,
    detail: &str,
    strings: &Strings,
) {
    let color = match state {
        diagnostics::AssessmentState::Ready => palette.success,
        diagnostics::AssessmentState::Warning => palette.warning,
        diagnostics::AssessmentState::Blocked => palette.danger,
        diagnostics::AssessmentState::Unknown => palette.muted,
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(label).strong());
        theme::pill(
            ui,
            strings.assessment_state(state),
            color,
            theme::tint(color, 34),
        );
    });
    ui.add(egui::Label::new(RichText::new(detail).small()).wrap());
    ui.add_space(4.0);
}

fn dashboard_value(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(label)
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.add(egui::Label::new(RichText::new(value).strong().color(color)).wrap());
    });
}

fn counter_value(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |count| count.to_string())
}

#[derive(Clone, Copy)]
enum CapabilityKind {
    Unicode,
    Composition,
    Protection,
}

fn reported_capability(
    value: Option<bool>,
    strings: &Strings,
    kind: CapabilityKind,
    palette: &Palette,
) -> (&'static str, Color32) {
    match value {
        Some(true) => (strings.reported_supported(), palette.success),
        Some(false)
            if matches!(
                kind,
                CapabilityKind::Composition | CapabilityKind::Protection
            ) =>
        {
            (strings.unsupported_on_route(), palette.warning)
        }
        Some(false) => (strings.reported_limited(), palette.warning),
        None => (strings.not_reported(), palette.muted),
    }
}

fn overview_value(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(label)
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.add(egui::Label::new(RichText::new(value).strong().color(color)).wrap());
        ui.add_space(5.0);
    });
}

fn assessment_color(state: diagnostics::AssessmentState, palette: &Palette) -> Color32 {
    match state {
        diagnostics::AssessmentState::Ready => palette.success,
        diagnostics::AssessmentState::Warning => palette.warning,
        diagnostics::AssessmentState::Blocked => palette.danger,
        diagnostics::AssessmentState::Unknown => palette.muted,
    }
}

#[cfg(test)]
mod desktop_test_review_tests {
    use super::*;

    #[test]
    fn desktop_test_picker_excludes_disabled_dynamic_and_application_scoped_snippets() {
        let config = Config::parse(
            r#"[[expansion]]
trigger = ";custom-check"
replacement = "read-only check"

[[expansion]]
trigger = ";disabled"
replacement = "not available"
enabled = false

[[expansion]]
trigger = ";form"
replacement = "{{field:value}}"

[[expansion]]
trigger = ";action"
replacement = "action"
[expansion.command]
program = "/usr/bin/true"

[[expansion]]
trigger = ";editor-only"
replacement = "editor check"
app_filter = ["app_id_exact:org.example.Editor"]
"#,
        )
        .unwrap();
        let indices = desktop_test_indices(&config, false);
        assert_eq!(indices.len(), 1);
        assert_eq!(config.expansion[indices[0]].trigger, ";custom-check");
        assert_eq!(config.expansion[indices[0]].replacement, "read-only check");
    }

    #[test]
    fn review_never_invites_a_second_trigger_after_observing_output() {
        let review = DesktopTestReview {
            trigger: ";;hello".into(),
            replacement: "Hello from Wayland!".into(),
            application_class: "GTK".into(),
            route: None,
            route_instance: None,
            desktop: "test desktop".into(),
            session: None,
            build: "test build".into(),
            config_revision: "sha256:test-config".into(),
            capture_activity_observed: true,
            output_activity_observed: true,
        };
        let instruction = desktop_test_review_instruction(
            &crate::lang::Strings::new(crate::lang::Language::English),
            &review,
        );
        assert!(instruction.contains("inspect the field where you attempted to type"));
        assert!(instruction.contains("Do not type the trigger again"));
    }

    #[test]
    fn stale_field_limitation_is_not_presented_as_current_focus_diagnosis() {
        let guidance = no_surrounding_text_guidance(&crate::lang::Strings::new(
            crate::lang::Language::English,
        ));
        assert!(guidance.contains("A recent input was refused"));
        assert!(guidance.contains("may not describe the currently focused"));
        assert!(!guidance.contains("this field does not provide"));
    }
}

#[cfg(test)]
mod route_scope_tests {
    use super::terminal_only_route;

    #[test]
    fn terminal_only_scope_excludes_an_enabled_raw_capture_application_fallback() {
        assert!(terminal_only_route(
            "state=connected · source=evdev · terminal_apps_only=true"
        ));
        assert!(terminal_only_route(
            "state=connected · source=evdev · terminal_apps_only=true · raw_capture_fallback_enabled=false · raw_capture_app_count=14"
        ));
        assert!(terminal_only_route(
            "state=connected · source=evdev · terminal_apps_only=true · raw_capture_fallback_enabled=true · raw_capture_app_count=0"
        ));
        assert!(!terminal_only_route(
            "state=connected · source=evdev · terminal_apps_only=true · raw_capture_fallback_enabled=true · raw_capture_app_count=14"
        ));
        // If the fallback is enabled but the daemon lacks the newer count
        // field, do not claim that the route is terminal-only.
        assert!(!terminal_only_route(
            "state=connected · source=evdev · terminal_apps_only=true · raw_capture_fallback_enabled=true"
        ));
        assert!(!terminal_only_route(
            "state=connected · source=ibus · terminal_apps_only=false"
        ));
        assert!(!terminal_only_route("state=connected · source=ibus"));
    }
}
