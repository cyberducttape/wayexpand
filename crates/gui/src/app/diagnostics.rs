//! Diagnostics window: runtime health, route capabilities, backends, and protocol probes.

use crate::*;

fn desktop_test_options(expansions: &[ExpansionConfig]) -> Vec<(String, String)> {
    expansions
        .iter()
        .filter(|expansion| {
            expansion.enabled
                && expansion.app_filter.is_empty()
                && expansion.command.is_none()
                && !expansion.replacement.contains("{{")
        })
        .map(|expansion| (expansion.trigger.clone(), expansion.replacement.clone()))
        .collect()
}

fn desktop_test_review_instruction(review: &DesktopTestReview) -> String {
    format!(
        "In {}, inspect the field where you attempted to type {}. Do not type the trigger again.",
        review.application_class, review.trigger
    )
}

impl GuiApp {
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

    pub(crate) fn render_diagnostics_body(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        if let Some(report) = self.setup_failure_report.as_deref() {
            ui.group(|ui| {
                ui.colored_label(palette.danger, "Setup failed");
                if let Some(summary) = report
                    .lines()
                    .find_map(|line| line.strip_prefix("Summary: "))
                {
                    ui.label(summary);
                }
                if ui.button("View diagnostic report").clicked() {
                    self.setup_failure_report_open = true;
                }
            });
            ui.add_space(8.0);
        }
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
                                "{} capture · {} injection · {} · focus tracking {} · {}",
                                route.contract.capture,
                                route.contract.injection,
                                if active_route_id.as_deref() == Some(route.contract.id.as_str()) {
                                    active_replacement
                                        .as_deref()
                                        .unwrap_or(route.contract.replacement_assurance.as_str())
                                } else {
                                    route.contract.replacement_assurance.as_str()
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
                            ui.label(RichText::new("Compatibility mode: Maximum").strong());
                            ui.label("Advantage: reaches more applications through global keyboard capture.");
                            ui.label("Recommended for operators who understand and accept these limitations:");
                        }
                        if !route.assessment.limitations.is_empty() {
                            ui.label(RichText::new("Limitations").strong());
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
                let color = if trace.completion == "applied" {
                    palette.success
                } else if trace.completion.contains("unknown")
                    || trace.completion.contains("failed")
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
                    runtime::ReplacementAssurance::Verified => palette.success,
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
        if ui.button("Export Support Report").clicked() {
            self.support_export_open = true;
            self.support_preview = None;
            self.support_preview_error = None;
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

        theme::section_header(ui, "", "Quick checks and actions");
        theme::card(ui, palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.strong("Installation");
                ui.colored_label(palette.muted, "Use guided setup checks");
                if ui.button("Open setup").clicked() {
                    self.diagnostics_close_requested = true;
                    self.onboarding_visible = true;
                    self.refresh_diagnostics(false);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong("Configuration");
                ui.colored_label(
                    if config_ok {
                        palette.success
                    } else {
                        palette.danger
                    },
                    if config_ok {
                        "Valid"
                    } else {
                        "Needs attention"
                    },
                );
                if ui.button("Edit").clicked() {
                    // The main window is the configuration editor.
                    self.diagnostics_close_requested = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong("Recovery");
                ui.label("Reset integration, services, or GUI preferences with a preview and backup where applicable.");
                if ui.button("Reset…").clicked() {
                    self.reset_dialog_open = true;
                }
                if ui.button("Backup / restore…").clicked() {
                    self.config_backup_dialog_open = true;
                    self.restore_preview = None;
                    self.restore_confirmed = false;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong("Input and output");
                ui.colored_label(
                    if matching && output_ok {
                        palette.success
                    } else {
                        palette.warning
                    },
                    if matching && output_ok {
                        "Available"
                    } else {
                        "Degraded / unavailable"
                    },
                );
                if ui.button("Test").clicked() {
                    self.backend_test_help_open = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong("User service");
                ui.colored_label(
                    if self.daemon_reachable == Some(true) {
                        palette.success
                    } else {
                        palette.warning
                    },
                    if self.daemon_reachable == Some(true) {
                        "Daemon responding"
                    } else {
                        "Not responding"
                    },
                );
                if let Some(unit) = active_service {
                    if ui
                        .add_enabled(self.setup_task.is_none(), egui::Button::new("Restart…"))
                        .clicked()
                    {
                        self.service_restart_unit = Some(unit.to_owned());
                        self.service_restart_confirm_open = true;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong("Software versions");
                let version_state = if self.daemon_reachable != Some(true) {
                    "Daemon unavailable"
                } else {
                    match runtime::daemon_build_state(&self.daemon_status) {
                        runtime::DaemonBuildState::Current => "CLI and daemon match",
                        runtime::DaemonBuildState::RestartRequired => "Daemon restart required",
                        runtime::DaemonBuildState::Unverified => "Version not verified",
                    }
                };
                ui.colored_label(
                    if version_state == "CLI and daemon match" {
                        palette.success
                    } else {
                        palette.warning
                    },
                    version_state,
                );
                if ui
                    .add_enabled(!self.diagnostics_running, egui::Button::new("Check again"))
                    .clicked()
                {
                    self.refresh_diagnostics(true);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong("Test Expansion");
                ui.colored_label(
                    palette.accent,
                    "Choose a local matcher check or a real desktop check",
                );
                if ui.button("Test Expansion…").clicked() {
                    self.test_expansion_dialog_open = true;
                }
            });
            if let Some(route) = self.recommended_route {
                ui.separator();
                ui.label(RichText::new("Capability-based recommendation").strong());
                ui.label(format!(
                    "{} — {}. Based on live capability probes and organization policy; desktop name alone is not treated as proof.",
                    route.contract().label,
                    route.contract().setup_detail,
                ));
            } else {
                ui.separator();
                ui.colored_label(
                    palette.warning,
                    "No policy-allowed route is currently recommended by the capability probes.",
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
                    RichText::new(format!("Integration profile · {}", profile.desktop)).strong(),
                );
                ui.label(format!(
                    "Recommended capture route: {} · injection: {}",
                    profile.recommended_route_id,
                    profile.supported_injection_backends.join(", "),
                ));
                ui.label(
                    RichText::new(format!(
                        "Certification: {}",
                        profile.last_certification_result
                    ))
                    .color(palette.warning),
                );
                ui.collapsing("Profile details", |ui| {
                    ui.label(RichText::new("Required permissions").strong());
                    for item in &profile.required_permissions {
                        ui.label(format!("• {item}"));
                    }
                    ui.label(RichText::new("Setup").strong());
                    for item in &profile.setup_instructions {
                        ui.label(format!("• {item}"));
                    }
                    ui.label(RichText::new("Known limitations").strong());
                    for item in &profile.known_limitations {
                        ui.label(format!("• {item}"));
                    }
                    let verified_applications = if profile.verified_applications.is_empty() {
                        "none".to_owned()
                    } else {
                        profile.verified_applications.join(", ")
                    };
                    ui.label(format!("Verified applications: {verified_applications}"));
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
        egui::Window::new("Desktop backend test")
            .id(egui::Id::new("diagnostics_backend_test_help"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(500.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Matcher Test").strong());
                ui.label("Use the editor’s Test matcher action to check a trigger against the loaded configuration. This does not exercise desktop capture or insertion.");
                ui.label(RichText::new("Desktop Test").strong());
                ui.label(RichText::new("These one-shot checks test evdev capture and libei injection separately. Capture requires explicit consent and never records key values, though your typing still reaches the focused field. Injection inserts only a harmless marker after confirmation; no deletion or Enter key is sent. Neither test certifies the combined route or a specific application.").color(palette.warning));
                ui.horizontal_wrapped(|ui| {
                    for command in ["wayexpand backend test --capture", "wayexpand backend test --injection"] {
                        if ui.button(format!("Copy: {command}")).clicked() {
                            ui.ctx().copy_text(command.to_owned());
                        }
                    }
                });
                if ui.button("Copy portal help: WX-PORTAL-001").clicked() {
                    ui.ctx().copy_text("wayexpand help finding WX-PORTAL-001".into());
                }
                ui.label("Run the copied command in a terminal to complete its consent and target-focus prompts. For the active route, use the harmless tutorial trigger in a normal application and inspect the exact result.");
                if ui.button("Close").clicked() {
                    close_clicked = true;
                }
            });
        if close_clicked {
            open = false;
        }
        self.backend_test_help_open = open;
    }

    fn render_test_expansion_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.test_expansion_dialog_open {
            return;
        }
        let mut open = self.test_expansion_dialog_open;
        let mut close = false;
        let first_run_verification = self.desktop_test_dialog_required;
        let desktop_tests = if first_run_verification {
            super::first_run::onboarding_test_snippet(&self.config)
                .map(|expansion| vec![(expansion.trigger.clone(), expansion.replacement.clone())])
                .unwrap_or_default()
        } else {
            desktop_test_options(&self.config.expansion)
        };
        if !desktop_tests
            .iter()
            .any(|(trigger, _)| trigger == &self.desktop_test_trigger)
        {
            self.desktop_test_trigger = self
                .selected_index()
                .and_then(|index| self.config.expansion.get(index))
                .filter(|expansion| {
                    expansion.enabled
                        && expansion.app_filter.is_empty()
                        && expansion.command.is_none()
                        && !expansion.replacement.contains("{{")
                })
                .map(|expansion| expansion.trigger.clone())
                .or_else(|| desktop_tests.first().map(|(trigger, _)| trigger.clone()))
                .unwrap_or_default();
        }
        let desktop_test = desktop_tests
            .iter()
            .find(|(trigger, _)| trigger == &self.desktop_test_trigger)
            .cloned();
        let desktop_test_command = desktop_test.as_ref().map(|(trigger, _)| {
            let trigger = trigger.replace('\'', "'\\''");
            format!("wayexpand debug --test '{trigger}'")
        });
        let application_class_valid = !self.desktop_test_application_class.trim().is_empty()
            && self.desktop_test_application_class.chars().count() <= 64
            && !self
                .desktop_test_application_class
                .chars()
                .any(char::is_control);
        egui::Window::new("Test Expansion")
            .id(egui::Id::new("test_expansion_mode_picker"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(if first_run_verification {
                    "Complete this real-app test to finish first-run setup. Until you confirm the exact result, WayExpand remains configured but unverified. You can defer and return to the guide later."
                } else {
                    "A real insertion test verifies text in a target application. It never presses Enter or retries an uncertain insertion."
                }).color(palette.muted));
                ui.label(RichText::new("Matcher Test").strong());
                ui.label("Checks the loaded snippets and template rendering locally. It does not read keyboard input or modify another application.");
                if ui.button("Open Matcher Test").clicked() {
                    self.diagnostics_close_requested = true;
                    self.try_live_open = true;
                    close = true;
                }
                ui.separator();
                ui.label(RichText::new("Desktop Test").strong());
                ui.label("Tests the active route in a disposable text field. It never presses Enter or retries insertion. Activity counters are not tied to this test trigger; inspect the resulting text and confirm an exact match yourself. This verifies only the selected application in this session.");
                if desktop_tests.len() > 1 {
                    egui::ComboBox::from_id_salt("desktop_test_trigger_choice")
                        .selected_text(if self.desktop_test_trigger.is_empty() {
                            "Choose a saved static snippet"
                        } else {
                            &self.desktop_test_trigger
                        })
                        .show_ui(ui, |ui| {
                            for (trigger, _) in &desktop_tests {
                                ui.selectable_value(
                                    &mut self.desktop_test_trigger,
                                    trigger.clone(),
                                    trigger,
                                );
                            }
                        });
                }
                if let Some((trigger, replacement)) = &desktop_test {
                    ui.label(
                        RichText::new(format!("Type this once: {trigger}"))
                            .heading()
                            .strong(),
                    );
                    ui.label(RichText::new("Expected replacement").strong());
                    ui.add(
                        egui::TextEdit::multiline(&mut replacement.as_str())
                            .code_editor()
                            .interactive(false)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label("Application type (for the test report)");
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
                    "I am focused on a disposable, non-sensitive text field and will type the trigger once.",
                );
                let can_run_test = desktop_test.is_some()
                    && application_class_valid
                    && self.desktop_test_target_acknowledged
                    && self.draft.is_none()
                    && !self.new_draft
                    && self.setup_task.is_none();
                if ui
                    .add_enabled(can_run_test, egui::Button::new("Start Desktop Test"))
                    .clicked()
                {
                    if let Some((trigger, replacement)) = desktop_test.clone() {
                        self.start_desktop_test(
                            trigger,
                            replacement,
                            self.desktop_test_application_class.trim().to_owned(),
                        );
                        self.desktop_test_target_acknowledged = false;
                        close = true;
                    }
                }
                if self.draft.is_some() || self.new_draft {
                    ui.colored_label(
                        palette.warning,
                        "Save or discard the current snippet draft before testing; the desktop test runs the saved configuration.",
                    );
                }
                if let Some(command) = &desktop_test_command {
                    if ui.button("Copy desktop test command").clicked() {
                        ui.ctx().copy_text(command.clone());
                    }
                } else if first_run_verification {
                    ui.label("A real-app test needs a saved, enabled static snippet. Add the harmless WayExpand test snippet or enable a suitable static snippet; command-backed and templated snippets are excluded.");
                    if ui.button("Add harmless test snippet").clicked() {
                        self.create_test_snippet();
                    }
                } else {
                    ui.label("No enabled static snippet is available for a desktop test. Save or enable a harmless static snippet, then try again. Templated and command-backed snippets are excluded.");
                }
                ui.colored_label(palette.warning, "If the matcher succeeds but desktop insertion fails, inspect the Input and Output route assessment and run the guided backend checks.");
                if ui.button(if first_run_verification {
                    "Continue later — setup stays unverified"
                } else {
                    "Close"
                }).clicked() {
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
                    "WayExpand observed input/output activity during the test window, but could not correlate it to this trigger. No insertion was retried.",
                );
                ui.label(desktop_test_review_instruction(review));
                ui.label(format!(
                    "Capture: {} · Output: {} · Trigger correlation: not verified",
                    if review.capture_activity_observed {
                        "activity observed"
                    } else {
                        "not observed"
                    },
                    if review.output_activity_observed {
                        "activity observed"
                    } else {
                        "not observed"
                    }
                ));
                ui.label("Expected replacement:");
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
                ui.label("Did this exact text appear once, with no lost, duplicated, interleaved, or wrong-field text?");
                ui.horizontal(|ui| {
                    if ui.button("Yes, exact result").clicked() {
                        confirmed = true;
                    }
                    if ui.button("No or unsure").clicked() {
                        rejected = true;
                    }
                    if ui.button("Leave unverified").clicked() {
                        dismissed = true;
                    }
                });
            });
        if confirmed {
            self.desktop_test_verified = true;
            self.desktop_test_verified_application = Some(application_class.clone());
            self.desktop_test_review = None;
            if self.desktop_test_dialog_required || self.onboarding_visible {
                self.onboarding_visible = true;
                self.finish_onboarding();
            } else {
                self.status = Status::success(format!(
                    "User-confirmed exact insertion in {}. This does not certify other applications or sessions.",
                    application_class
                ));
            }
        } else if rejected {
            self.desktop_test_verified = false;
            self.desktop_test_verified_application = None;
            self.status = Status::error(
                "Desktop insertion was not confirmed. Inspect the target field and Diagnostics; do not retry automatically.",
            );
            self.desktop_test_review = None;
            self.diagnostics_open = true;
        } else if dismissed {
            self.desktop_test_verified = false;
            self.desktop_test_verified_application = None;
            self.status = Status::warning("Desktop test result remains unverified.");
            self.desktop_test_review = None;
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
        egui::Window::new("Restart WayExpand service?")
            .id(egui::Id::new("diagnostics_restart_confirmation"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let unit = self.service_restart_unit.as_deref().unwrap_or("unknown service");
                ui.label(format!("Restart {unit} using your user systemd manager?"));
                ui.colored_label(palette.warning, "The input route may be interrupted briefly. No administrator privileges or keyboard-device permissions will be requested.");
                ui.horizontal(|ui| {
                    if ui.button("Restart service").clicked() {
                        if let Some(unit) = self.service_restart_unit.take() {
                            self.start_service_restart(unit);
                        }
                        close_clicked = true;
                    }
                    if ui.button("Cancel").clicked() {
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
            egui::Window::new("Preview recovery reset")
                .id(egui::Id::new("diagnostics_reset_preview"))
                .order(egui::Order::Foreground)
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(500.0)
                .show(ctx, |ui| {
                    ui.label("Choose one reset scope. Your expansion library is never removed.");
                    ui.radio_value(&mut self.reset_scope, "integration".into(), "Integration: stop services and clear remembered route consent");
                    ui.radio_value(&mut self.reset_scope, "services".into(), "Services: stop and disable WayExpand user services only");
                    ui.radio_value(&mut self.reset_scope, "preferences".into(), "Preferences: back up and reset GUI-only preferences");
                    ui.separator();
                    match self.reset_scope.as_str() {
                        "integration" => {
                            ui.label("Preview: disable and stop installed WayExpand services; remove the evdev consent marker and saved portal restoration token. Keep configuration and snippets unchanged.");
                            ui.colored_label(palette.warning, "The next evdev or libei use may require renewed permission. No administrator permission is requested.");
                        }
                        "services" => { ui.label("Preview: disable and stop installed WayExpand user services. Configuration, snippets, permissions, and portal token remain unchanged."); }
                        _ => { ui.label("Preview: create a private backup of gui-prefs.toml, then remove that GUI preference file. Your expansion library is unchanged."); }
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Review and continue…").clicked() {
                            self.reset_confirm_open = true;
                        }
                        if ui.button("Close").clicked() {
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
            egui::Window::new("Confirm reset")
                .id(egui::Id::new("diagnostics_reset_confirm"))
                .order(egui::Order::Foreground)
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.colored_label(
                        palette.warning,
                        format!(
                            "Apply the {} reset now? This cannot be undone automatically.",
                            self.reset_scope
                        ),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.setup_task.is_none(),
                                egui::Button::new("Apply reset"),
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
                                format!("Applying {} reset…", self.reset_scope),
                                completion,
                            );
                            close_clicked = true;
                            self.reset_dialog_open = false;
                        }
                        if ui.button("Cancel").clicked() {
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
        egui::Window::new("Configuration backup and restore")
            .id(egui::Id::new("diagnostics_config_backup_restore"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label("Export creates a private, no-overwrite backup. Restore validates the selected file and previews trigger-level differences before replacing the active configuration. The current configuration is backed up before restore.");
                ui.separator();
                ui.label(RichText::new("Export current configuration").strong());
                if ui.text_edit_singleline(&mut self.config_backup_path).changed() {
                    self.restore_preview = None;
                    self.last_cli_task_output = None;
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.setup_task.is_none() && !self.config_backup_path.trim().is_empty(), egui::Button::new("Export backup")).clicked() {
                        self.start_cli_task(
                            vec!["backup".into(), self.path.display().to_string(), self.config_backup_path.trim().to_owned()],
                            "Creating configuration backup…".into(),
                            "Configuration backup created.".into(),
                        );
                    }
                });
                ui.separator();
                ui.label(RichText::new("Review a backup to restore").strong());
                if ui.text_edit_singleline(&mut self.config_backup_path).changed() {
                    self.restore_preview = None;
                    self.last_cli_task_output = None;
                }
                if ui.add_enabled(self.setup_task.is_none() && !self.config_backup_path.trim().is_empty(), egui::Button::new("Review differences")).clicked() {
                    self.restore_preview = None;
                    self.restore_confirmed = false;
                    self.start_cli_task(
                        vec!["restore".into(), self.config_backup_path.trim().to_owned(), "--check".into(), "--json".into()],
                        "Validating restore backup…".into(),
                        "Restore preview validated.".into(),
                    );
                    self.cli_task_kind = Some("restore-check".into());
                }
                if let Some(report) = self.restore_preview.as_ref() {
                    if report["valid"] == true && report["changes"].is_object() {
                        ui.colored_label(palette.success, "Backup is valid. Review the differences:");
                        for (label, key) in [
                            ("Added", "added_triggers"),
                            ("Removed", "removed_triggers"),
                            ("Modified", "modified_triggers"),
                        ] {
                            let values = report["changes"][key].as_array().cloned().unwrap_or_default();
                            ui.label(format!("{label} triggers ({}): {}", values.len(), values.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>().join(", ")));
                        }
                        ui.label(format!("Settings changed: {}; hotkeys changed: {}", report["changes"]["settings_changed"], report["changes"]["hotkeys_changed"]));
                        ui.checkbox(&mut self.restore_confirmed, "I reviewed these differences and want to restore this configuration");
                        if ui.add_enabled(self.setup_task.is_none() && self.restore_confirmed, egui::Button::new("Restore and back up current config")).clicked() {
                            let current_fingerprint = report["current_fingerprint"].as_str().unwrap_or_default().to_owned();
                            let backup_fingerprint = report["backup_fingerprint"].as_str().unwrap_or_default().to_owned();
                            self.start_cli_task(
                                vec!["restore".into(), self.config_backup_path.trim().to_owned(), "--apply".into(), "--yes".into(), "--json".into(), format!("--expected-current={current_fingerprint}"), format!("--expected-backup={backup_fingerprint}")],
                                "Restoring configuration…".into(),
                                "Configuration restored; diagnostics refreshed.".into(),
                            );
                            self.cli_task_kind = Some("restore-apply".into());
                        }
                    } else {
                        ui.colored_label(palette.danger, "Restore validation did not return an applicable backup report.");
                    }
                }
                if let Some(output) = self.last_cli_task_output.as_deref() {
                    ui.label(RichText::new(output).small().color(palette.muted));
                }
                if self.setup_task.is_some() { ui.spinner(); }
            });
        self.config_backup_dialog_open = open;
    }

    fn render_setup_failure_report(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.setup_failure_report_open {
            return;
        }
        let mut open = self.setup_failure_report_open;
        egui::Window::new("Setup diagnostic report")
            .id(egui::Id::new("setup_failure_diagnostic_report"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(680.0, 520.0))
            .show(ctx, |ui| {
                ui.label(RichText::new("Sanitized setup output; home-directory paths are shortened. No environment variables or input contents are included.").small().color(palette.muted));
                ui.horizontal(|ui| {
                    if ui.button("Copy report").clicked() {
                        if let Some(report) = &self.setup_failure_report {
                            ui.ctx().copy_text(report.clone());
                        }
                    }
                    if ui.button("Open Diagnostics").clicked() {
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
        egui::Window::new("Export Support Report")
            .id(egui::Id::new("support_report_export"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(640.0, 520.0))
            .show(ctx, |ui| {
                ui.label(RichText::new("This report includes").strong());
                for item in [
                    "WayExpand version and desktop session",
                    "Backend and route availability",
                    "Service states, exit codes, restart counts, and stable error IDs",
                    "Bounded sanitized event counters and certification summary",
                ] {
                    ui.label(format!("✓ {item}"));
                }
                ui.add_space(6.0);
                ui.label(RichText::new("This report excludes").strong());
                for item in [
                    "Typed text and keystrokes",
                    "Snippet and replacement contents",
                    "Clipboard contents",
                    "Authentication tokens and raw journal messages",
                    "Configuration paths and free-form diagnostic details",
                ] {
                    ui.label(format!("✓ {item}"));
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.support_preview_loading,
                            egui::Button::new("Preview Report"),
                        )
                        .clicked()
                    {
                        self.support_preview_loading = true;
                        self.support_preview_error = None;
                        if let Some(sender) = self.diagnostics_sender.as_ref() {
                            if sender.try_send(runtime::Request::SupportPreview).is_err() {
                                self.support_preview_loading = false;
                                self.support_preview_error =
                                    Some("The background diagnostics queue is busy.".into());
                            }
                        } else {
                            self.support_preview_loading = false;
                            self.support_preview_error =
                                Some("Background diagnostics are unavailable.".into());
                        }
                    }
                    if self.support_preview_loading {
                        ui.spinner();
                        ui.label("Collecting sanitized report…");
                    }
                    if ui
                        .add_enabled(
                            self.support_preview.is_some(),
                            egui::Button::new("Save Report"),
                        )
                        .clicked()
                    {
                        match self.save_support_report() {
                            Ok(()) => {
                                self.support_preview_error =
                                    Some("Report saved with owner-only permissions.".into())
                            }
                            Err(error) => self.support_preview_error = Some(error),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Save as");
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
                if ui.button("Desktop test…").clicked() {
                    self.backend_test_help_open = true;
                }
            });
        self.support_export_open = open;
    }

    fn save_support_report(&mut self) -> Result<(), String> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let path = std::path::PathBuf::from(self.support_output_path.trim());
        if path.as_os_str().is_empty() {
            return Err("Choose a report path first.".into());
        }
        let report = self
            .support_preview
            .as_deref()
            .ok_or("Preview the report before saving.")?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| format!("Could not create report file: {error}"))?;
        if let Err(error) = file
            .write_all(report.as_bytes())
            .and_then(|()| file.sync_all())
        {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(format!("Could not write report: {error}"));
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
        } else if readiness.expanded {
            "Output observed; application result unverified".to_owned()
        } else {
            "Not verified in an application".to_owned()
        };
        ui.group(|ui| {
            ui.label(RichText::new("GNOME input method: configured, running, verified").strong());
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    if readiness.configuration == wayexpand_backend_ibus::chain::IbusCheckState::Pass { palette.success } else { palette.warning },
                    format!("Configured: {}", readiness.configuration.label()),
                );
                ui.colored_label(
                    if running { palette.success } else if readiness.ibus_running && readiness.engine_running { palette.warning } else { palette.muted },
                    format!("Running: {running_text}"),
                );
                ui.colored_label(
                    if self.desktop_test_verified { palette.success } else { palette.muted },
                    format!("Verified: {verified_text}"),
                );
            });
            if !self.desktop_test_verified {
                ui.label(RichText::new("Only a user-confirmed exact result in a real application is marked verified.").small().color(palette.muted));
                if ui.small_button("Run Desktop Integration Test…").clicked() {
                    self.test_expansion_dialog_open = true;
                }
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
                    ui.label(RichText::new("Finish GNOME input-source selection").strong());
                    if source_registered {
                        ui.label("WayExpand is registered, but GNOME is using another input source. Open the keyboard/layout selector in the GNOME top bar and choose WayExpand. Your existing keyboard layouts remain available.");
                        if ui.button("Try selecting WayExpand").clicked() {
                            self.retry_gnome_source_activation();
                        }
                    } else {
                        ui.label("WayExpand is not yet registered as a GNOME input source. Continue the guided setup to register it and activate it; any required administrator prompt is explained before it appears.");
                        if ui.button("Continue guided setup").clicked() {
                            self.start_guided_setup();
                        }
                    }
                    if ui.small_button("Recheck input-source status").clicked() {
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
                            RichText::new(format!("Problem: {problem_id}"))
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
                    wayexpand_backend_ibus::IbusFieldVerdict::Expanded
                    | wayexpand_backend_ibus::IbusFieldVerdict::Ready => palette.success,
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
                        RichText::new(
                            "Input reached WayExpand, but this field does not provide verifiable surrounding text. Automatic replacement stays disabled here. Use Quick Picker to copy a snippet and paste it yourself.",
                        )
                        .small()
                        .color(palette.warning),
                    );
                    if ui
                        .small_button("Open Quick Picker (copy and paste)")
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
        &self,
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
            ui.label(
                RichText::new(self.strings.session_evidence_note())
                    .small()
                    .color(palette.warning),
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
                let replacement_color = match capabilities
                    .and_then(|caps| caps.replacement_assurance)
                {
                    Some(runtime::ReplacementAssurance::Verified) => palette.success,
                    Some(runtime::ReplacementAssurance::FocusTrackedBestEffort) => palette.warning,
                    Some(
                        runtime::ReplacementAssurance::Unverified
                        | runtime::ReplacementAssurance::Blocked,
                    ) => palette.danger,
                    Some(runtime::ReplacementAssurance::Unavailable) | None => palette.muted,
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
                        palette.success
                    } else {
                        palette.muted
                    },
                );
            });
        });
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
        assert_eq!(
            desktop_test_options(&config.expansion),
            vec![(";custom-check".into(), "read-only check".into())]
        );
    }

    #[test]
    fn review_never_invites_a_second_trigger_after_observing_output() {
        let review = DesktopTestReview {
            trigger: ";;hello".into(),
            replacement: "Hello from Wayland!".into(),
            application_class: "GTK".into(),
            capture_activity_observed: true,
            output_activity_observed: true,
        };
        let instruction = desktop_test_review_instruction(&review);
        assert!(instruction.contains("inspect the field where you attempted to type"));
        assert!(instruction.contains("Do not type the trigger again"));
    }
}
