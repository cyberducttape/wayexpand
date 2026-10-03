//! Toolbar: brand, route status, pause/run controls, and library search.

use crate::*;

impl GuiApp {
    pub(crate) fn toggle_pause(&mut self) {
        if self.pending_control > 0 {
            return;
        }
        let paused = !self.paused;
        let command = if paused { "pause" } else { "resume" };
        let Some(sender) = self.runtime_sender.as_ref() else {
            self.status = Status::error(self.strings.background_runtime_stopped());
            return;
        };
        match sender.try_send(runtime::Request::Control {
            command: command.into(),
            operation: runtime::Operation::Pause { paused },
        }) {
            Ok(()) => {
                self.pending_control += 1;
                self.status = Status::info(self.strings.daemon_control_running());
            }
            Err(_) => {
                self.status = Status::warning(self.strings.background_queue_full());
            }
        }
    }

    pub(crate) fn render_toolbar(&mut self, root: &mut egui::Ui, palette: &Palette) {
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .stroke(egui::Stroke::NONE)
                    .shadow(egui::Shadow {
                        offset: [0, 6],
                        blur: 14,
                        spread: 0,
                        color: Color32::from_black_alpha(if self.dark_mode { 60 } else { 18 }),
                    }),
            )
            .show(root, |ui| {
                // One row when there is room: identity on the left, the
                // controls people reach for on the right. Narrow windows fall
                // back to a second row for search instead of overlapping.
                let single_row = ui.available_width() >= 980.0;
                ui.horizontal(|ui| {
                    self.render_toolbar_brand(ui, palette);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.render_toolbar_controls(ui, palette);
                        if single_row {
                            ui.add_space(4.0);
                            self.render_search(ui);
                        }
                    });
                });
                if !single_row {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_search(ui);
                        });
                    });
                }
            });
    }

    pub(crate) fn render_toolbar_brand(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.label(RichText::new("⚡").size(20.0).color(palette.accent));
        ui.label(RichText::new("WayExpand").heading().strong());
        ui.label(RichText::new(self.strings.title()).color(palette.muted));
        ui.add_space(8.0);
        theme::pill(
            ui,
            self.strings.snippets_count(self.config.expansion.len()),
            palette.muted,
            palette.surface_hover,
        );
        if !self.config.hotkey.is_empty() {
            theme::pill(
                ui,
                self.strings.hotkeys_count(self.config.hotkey.len()),
                palette.muted,
                palette.surface_hover,
            );
        }
    }

    /// Daemon health and the actions menu, laid out right to left.
    pub(crate) fn render_toolbar_controls(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let ctx = ui.ctx().clone();
        let (daemon_label, daemon_color) = match self.daemon_reachable {
            Some(true) => (self.strings.daemon_running_status(), palette.success),
            Some(false) if self.config.expansion.is_empty() => {
                (self.strings.daemon_not_enabled_status(), palette.muted)
            }
            Some(false) => (self.strings.daemon_unreachable_status(), palette.danger),
            None => (self.strings.daemon_unknown_status(), palette.muted),
        };
        let (route_label, route_color) = match self.route_state {
            Some(runtime::RouteState::Connected) if self.paused => {
                (self.strings.route_paused_status(), palette.warning)
            }
            Some(runtime::RouteState::Connected) => {
                (self.strings.route_connected_status(), palette.success)
            }
            Some(runtime::RouteState::Reconnecting) => {
                (self.strings.route_reconnecting_status(), palette.warning)
            }
            Some(runtime::RouteState::Starting) => {
                (self.strings.route_starting_status(), palette.warning)
            }
            Some(runtime::RouteState::PermissionRequired) => (
                self.strings.route_permission_required_status(),
                palette.warning,
            ),
            Some(runtime::RouteState::PortalRevoked) => {
                (self.strings.route_portal_revoked_status(), palette.warning)
            }
            Some(runtime::RouteState::Unsupported) => {
                (self.strings.route_unsupported_status(), palette.muted)
            }
            Some(runtime::RouteState::Degraded) => {
                (self.strings.route_degraded_status(), palette.warning)
            }
            Some(runtime::RouteState::Failed) => {
                (self.strings.route_failed_status(), palette.danger)
            }
            Some(runtime::RouteState::Stopped) => {
                (self.strings.route_stopped_status(), palette.muted)
            }
            None => (self.strings.route_unknown_status(), palette.muted),
        };
        let route_trust = self.active_route_contract().map(|contract| {
            (
                self.strings.route_trust_status(
                    &contract.label,
                    &contract.status,
                    contract.sensitive_fields,
                    contract.atomic_replace,
                ),
                palette.warning,
            )
        });
        let more_actions = self.strings.more_actions();
        let actions_response = ui
            .menu_button(more_actions, |ui| {
                if ui.button(self.strings.reload()).clicked() {
                    self.request_action(PendingAction::Reload);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.pending_control == 0,
                        egui::Button::new(if self.paused {
                            self.strings.resume()
                        } else {
                            self.strings.pause()
                        }),
                    )
                    .clicked()
                {
                    self.toggle_pause();
                    ui.close();
                }
                if ui.button(self.strings.diagnostics()).clicked() {
                    self.diagnostics_open = true;
                    self.refresh_diagnostics(true);
                    ui.close();
                }
                if self.library_is_synchronized()
                    && ui
                        .add_enabled(
                            self.sync_task.is_none(),
                            egui::Button::new(self.strings.sync_library()),
                        )
                        .clicked()
                {
                    self.start_sync();
                    ui.close();
                }
                if ui.button(self.strings.import_espanso()).clicked() {
                    self.import_open = true;
                    self.import_preview = None;
                    ui.close();
                }
                ui.separator();
                if ui.button(self.strings.settings()).clicked() {
                    self.open_settings();
                    ui.close();
                }
                if ui
                    .button(if self.dark_mode {
                        self.strings.theme_light()
                    } else {
                        self.strings.theme_dark()
                    })
                    .clicked()
                {
                    self.set_dark_mode(&ctx, !self.dark_mode);
                    ui.close();
                }
            })
            .response;
        actions_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, more_actions)
        });
        actions_response.on_hover_text(more_actions);
        if ui
            .add(
                egui::Button::new(RichText::new(route_label).color(route_color))
                    .fill(theme::tint(route_color, 38))
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(egui::CornerRadius::same(255))
                    .min_size(egui::vec2(0.0, 30.0)),
            )
            .on_hover_text(self.daemon_status.clone())
            .clicked()
        {
            self.diagnostics_open = true;
            self.refresh_diagnostics(true);
        }
        if let Some((trust_label, trust_color)) = route_trust {
            theme::pill(ui, trust_label, trust_color, theme::tint(trust_color, 34));
        }
        // The first-run screen has its own step for this.
        theme::pill(
            ui,
            daemon_label,
            daemon_color,
            theme::tint(daemon_color, 38),
        );
        if self.daemon_reachable == Some(false) && !self.config.expansion.is_empty() {
            self.turn_on_button(ui, palette);
        }
    }

    pub(crate) fn active_route_contract(
        &self,
    ) -> Option<&'static wayexpand_backend_selection::RouteContract> {
        let source = runtime::status_field(&self.daemon_status, "source")?;
        let backend = runtime::status_field(&self.daemon_status, "backend")?;
        route_contract_for(&source, &backend)
    }

    /// The search field and its field-scope menu, laid out right to left.
    pub(crate) fn render_search(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.strings.search_fields(), |ui| {
            ui.checkbox(
                &mut self.search_fields.triggers,
                self.strings.search_triggers(),
            );
            ui.checkbox(
                &mut self.search_fields.descriptions,
                self.strings.search_descriptions(),
            );
            ui.checkbox(&mut self.search_fields.tags, self.strings.search_tags());
            ui.checkbox(
                &mut self.search_fields.replacements,
                self.strings.search_replacements(),
            );
        });
        if !self.filter.is_empty()
            && ui
                .small_button("×")
                .on_hover_text(self.strings.clear_filters())
                .clicked()
        {
            self.filter.clear();
        }
        ui.add(
            TextEdit::singleline(&mut self.filter)
                .id(egui::Id::new(SEARCH_FIELD_SALT))
                .hint_text(self.strings.search_placeholder())
                .margin(egui::Margin::symmetric(10, 7))
                .desired_width(280.0),
        )
        .on_hover_text(self.strings.search_tooltip());
    }
}
