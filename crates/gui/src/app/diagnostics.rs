//! Diagnostics window: runtime health, route capabilities, backends, and protocol probes.

use crate::*;

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
            let max_size = theme::dialog_max_size(ctx, 560.0);
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
                .max_size(max_size)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    // The report is longer than most windows are tall; scroll
                    // it rather than letting the window grow off screen.
                    egui::ScrollArea::vertical()
                        .id_salt("diagnostics_scroll")
                        .auto_shrink([false, true])
                        .show(ui, |ui| self.render_diagnostics_body(ui, palette));
                });
            self.diagnostics_open = open;
        }
    }

    pub(crate) fn render_diagnostics_body(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        theme::section_header(ui, "", self.strings.runtime_dashboard());
        ui.collapsing("Ranked input routes", |ui| {
            ui.label(
                RichText::new(
                    "Based on this session, organization policy, and catalog certification. Experimental routes remain uncertified.",
                )
                .small()
                .color(palette.muted),
            );
            if let Some(plan) = &self.route_plan {
                for route in &plan.routes {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&route.contract.label).strong());
                            let standing = route.standing.as_str();
                            let color = match route.standing {
                                wayexpand_backend_selection::RouteStanding::Recommended
                                | wayexpand_backend_selection::RouteStanding::Available => palette.success,
                                wayexpand_backend_selection::RouteStanding::RequiresConsent => palette.warning,
                                wayexpand_backend_selection::RouteStanding::BlockedByPolicy
                                | wayexpand_backend_selection::RouteStanding::Unavailable => palette.muted,
                            };
                            ui.colored_label(color, standing);
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} capture · {} injection · {}",
                                route.contract.capture, route.contract.injection, route.reason
                            ))
                            .small()
                            .color(palette.muted),
                        );
                    });
                }
            } else {
                ui.label(self.strings.not_checked());
            }
        });
        let source = compact_status_value(&self.daemon_status, "source").unwrap_or("unknown");
        let backend = compact_status_value(&self.daemon_status, "backend").unwrap_or("unknown");
        let capabilities = self.daemon_capabilities.as_ref();
        let mean = capabilities.and_then(|caps| caps.injection_latency_mean_us);
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
                    compact_status_value(&self.daemon_status, "route_id").unwrap_or("unknown"),
                    palette.muted,
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
                    self.strings.average_latency(),
                    &mean.map_or_else(
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
                &self
                    .strings
                    .capture_assessment_detail(self.route_state, source),
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
        });

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
                });
            }
            if capabilities.capture_sensitive_focus != Some(true) {
                theme::card(ui, palette, |ui| {
                    ui.colored_label(palette.danger, self.strings.sensitive_field_warning());
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
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.diagnostics_running,
                    egui::Button::new(self.strings.run_compatibility_test()),
                )
                .clicked()
            {
                self.refresh_diagnostics(true);
            }
            if self.diagnostics_running {
                ui.spinner();
            }
        });
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
