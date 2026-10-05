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
            egui::Window::new(self.strings.diagnostics_title())
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
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
        theme::section_header(ui, "", self.strings.compatibility_center());
        ui.label(
            RichText::new(self.strings.onboarding_certification_note())
                .small()
                .color(palette.warning),
        );
        let readiness = match self.daemon_capabilities {
            None => (
                self.strings.production_readiness_unavailable(),
                palette.muted,
            ),
            Some(capabilities)
                if capabilities.capture_sensitive_focus == Some(true)
                    && capabilities.capture_composition_aware == Some(true)
                    && capabilities.capture_key_passthrough == Some(true)
                    && capabilities.capture_layout_aware == Some(true)
                    && capabilities.window_identity_exact == Some(true)
                    && capabilities.inject_atomic_replace == Some(true)
                    && capabilities.inject_full_unicode == Some(true)
                    && capabilities.inject_key_passthrough == Some(true) =>
            {
                (self.strings.production_readiness_pending(), palette.warning)
            }
            Some(_) => (self.strings.production_readiness_limited(), palette.warning),
        };
        theme::card(ui, palette, |ui| {
            ui.horizontal(|ui| {
                ui.label(self.strings.production_readiness());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(readiness.1, readiness.0);
                });
            });
        });
        if let Some(capabilities) = self.daemon_capabilities {
            theme::card(ui, palette, |ui| {
                ui.label(self.strings.active_route());
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
            let capabilities = self.daemon_capabilities.unwrap_or_default();
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
                ui.label(RichText::new(detail).color(palette.muted));
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
}
