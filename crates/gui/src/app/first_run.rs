use crate::app::setup::onboarding_step;
use crate::*;

impl GuiApp {
    /// First-run screen shown while the library is empty and no new
    /// snippet is being drafted: route status, setup, and a test snippet.
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
        let keyboard_probe = recommendation
            .map(|route| route.capture_state)
            .unwrap_or(BackendState::NotImplemented);
        let injection_probe = recommendation
            .map(|route| route.injection_state)
            .unwrap_or(BackendState::NotImplemented);
        let (recommended_route, recommended_route_detail) =
            self.strings.onboarding_recommended_route(recommendation);
        let daemon_running = self.daemon_reachable == Some(true);
        ui.vertical_centered(|ui| {
            ui.add_space(26.0);
            ui.label(RichText::new("⚡").size(40.0).color(palette.accent));
            ui.add_space(6.0);
            ui.label(
                RichText::new(self.strings.welcome_title())
                    .size(28.0)
                    .strong()
                    .color(ui.visuals().strong_text_color()),
            );
            ui.label(RichText::new(self.strings.welcome_intro()).color(palette.muted));
            ui.add_space(22.0);
            theme::card(ui, palette, |ui| {
                ui.set_width(ui.available_width().min(600.0));
                ui.label(RichText::new(self.strings.onboarding_recommended_route_title()).strong());
                ui.label(RichText::new(recommended_route).color(palette.accent));
                ui.label(
                    RichText::new(recommended_route_detail)
                        .small()
                        .color(palette.muted),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(self.strings.onboarding_certification_note())
                        .small()
                        .color(palette.warning),
                );
            });
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width().min(600.0), ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    theme::card(ui, palette, |ui| {
                        ui.set_width(ui.available_width());
                        onboarding_step(
                            ui,
                            palette,
                            1,
                            self.strings.onboarding_turn_on_title(),
                            daemon_running,
                        );
                        ui.indent("step_turn_on", |ui| {
                            if daemon_running {
                                ui.label(
                                    RichText::new(self.strings.running_status())
                                        .color(palette.success),
                                );
                            } else {
                                ui.label(
                                    RichText::new(self.strings.onboarding_turn_on_help())
                                        .color(palette.muted),
                                );
                                ui.add_space(4.0);
                                self.turn_on_button(ui, palette);
                                if ui.link(self.strings.onboarding_evdev_setup()).clicked() {
                                    self.evdev_setup_acknowledged = false;
                                    self.evdev_setup_open = true;
                                }
                            }
                        });
                        ui.add_space(12.0);
                        onboarding_step(
                            ui,
                            palette,
                            2,
                            self.strings.onboarding_snippet_title(),
                            false,
                        );
                        ui.indent("step_snippet", |ui| {
                            ui.label(
                                RichText::new(self.strings.onboarding_snippet_help())
                                    .color(palette.muted),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
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
                        });
                        ui.add_space(12.0);
                        onboarding_step(
                            ui,
                            palette,
                            3,
                            self.strings.onboarding_safety_title(),
                            false,
                        );
                        ui.indent("step_try", |ui| {
                            ui.label(
                                RichText::new(self.strings.onboarding_try_text())
                                    .color(palette.muted),
                            );
                        });
                        ui.add_space(10.0);
                        ui.collapsing(self.strings.onboarding_details_title(), |ui| {
                            ui.label(self.strings.onboarding_desktop(&desktop, is_wayland));
                            ui.label(
                                RichText::new(self.strings.onboarding_probe_caveat())
                                    .small()
                                    .color(palette.muted),
                            );
                            if let Some(route) = recommendation {
                                ui.label(
                                    RichText::new(
                                        self.strings.onboarding_capture_detail(route.capture_label),
                                    )
                                    .small()
                                    .monospace()
                                    .color(palette.muted),
                                );
                                ui.label(
                                    RichText::new(
                                        self.strings
                                            .onboarding_injection_detail(route.injection_label),
                                    )
                                    .small()
                                    .monospace()
                                    .color(palette.muted),
                                );
                            }
                            ui.horizontal(|ui| {
                                ui.label(self.strings.onboarding_keyboard_label());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            self.strings.onboarding_backend_state(keyboard_probe),
                                        );
                                    },
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(self.strings.onboarding_injection_label());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            self.strings.onboarding_backend_state(injection_probe),
                                        );
                                    },
                                );
                            });
                            ui.label(
                                RichText::new(
                                    self.strings.onboarding_detection(app_context_available),
                                )
                                .small()
                                .color(palette.muted),
                            );
                            ui.label(
                                RichText::new(self.strings.onboarding_app_caveat())
                                    .small()
                                    .color(palette.muted),
                            );
                            ui.label(
                                RichText::new(self.strings.onboarding_certification_note())
                                    .small()
                                    .color(palette.warning),
                            );
                        });
                    });
                },
            );
        });
    }
}
