//! Toolbar: brand, route status, pause/run controls, and library search.

use crate::*;

impl GuiApp {
    pub(crate) fn toggle_pause(&mut self) {
        if self.pending_control > 0 {
            return;
        }
        let paused = !self.paused;
        let daemon_operation = if paused {
            wayexpand_core::DaemonOperation::Pause
        } else {
            wayexpand_core::DaemonOperation::Resume
        };
        let Some(sender) = self.runtime_sender.as_ref() else {
            self.status = Status::error(self.strings.background_runtime_stopped());
            return;
        };
        match sender.try_send(runtime::Request::Control {
            daemon_operation,
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
                // Keep the brand and controls on one line while there is room.
                // At narrow widths, separate them into wrapped rows: placing a
                // right-to-left control layout in the remainder of a horizontal
                // row can make it overlap the brand when the labels no longer fit.
                let compact_width = 920.0 * self.settings_font_scale.multiplier();
                if ui.available_width() < compact_width {
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| self.render_toolbar_brand(ui, palette));
                        ui.add_space(6.0);
                        // Right to left, like the wide layout, so the primary
                        // action keeps its place at the end of the row.
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
                            |ui| self.render_toolbar_controls(ui, palette, true),
                        );
                    });
                } else {
                    ui.horizontal(|ui| {
                        // One fixed row height, whichever chips and buttons
                        // are visible, so the panels below never jump.
                        ui.set_min_height(34.0);
                        self.render_toolbar_brand(ui, palette);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_toolbar_controls(ui, palette, false);
                        });
                    });
                }
            });
    }

    pub(crate) fn render_toolbar_brand(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        theme::brand_mark(ui, 20.0, palette.accent);
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
    pub(crate) fn render_toolbar_controls(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        compact: bool,
    ) {
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
                let full_protection = self
                    .daemon_capabilities
                    .as_ref()
                    .is_some_and(|capabilities| capabilities.provides_full_protection());
                // "Ready" claims verified text expansion on this desktop;
                // without certification evidence the route is only running.
                let certified = self
                    .active_route_contract()
                    .is_some_and(|contract| contract.is_certified());
                if full_protection && certified {
                    (self.strings.route_connected_status(), palette.success)
                } else if full_protection {
                    (self.strings.route_running_status(), palette.success)
                } else {
                    (self.strings.route_limited_status(), palette.warning)
                }
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
            let sensitive_field_protection = if contract.sensitive_fields {
                self.daemon_capabilities
                    .as_ref()
                    .and_then(|capabilities| capabilities.capture_sensitive_focus)
            } else {
                Some(false)
            };
            (
                self.strings.route_trust_status(
                    &contract.label,
                    &contract.status,
                    sensitive_field_protection,
                    contract.atomic_replace,
                    &contract.replacement_assurance,
                    contract.is_certified(),
                ),
                if sensitive_field_protection == Some(false) {
                    palette.danger
                } else if contract.is_certified() && sensitive_field_protection == Some(true) {
                    palette.success
                } else {
                    palette.warning
                },
            )
        });
        // Workspace switching is a frequent action, so keep it directly in
        // the toolbar as well as in the profile-management menu. The list is
        // still sourced from the CLI's validated profile inventory.
        if !compact && !self.profile_loaded && self.profile_receiver.is_none() {
            self.start_profile_operation(ProfileOperation::List);
        }
        let runtime_profile = runtime_selected_profile(&self.daemon_status);
        let active_profile_label = runtime_profile
            .as_deref()
            .map(|name| self.strings.profile_automatic_active(name))
            .or_else(|| {
                self.active_profile
                    .as_deref()
                    .map(|name| format!("{}: {name}", self.strings.profiles()))
            })
            .unwrap_or_else(|| self.strings.profiles().to_owned());
        if !compact {
            ui.menu_button(active_profile_label, |ui| {
                if self.profile_receiver.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(self.strings.profile_loading());
                    });
                }
                if let Some(name) = runtime_profile.as_deref() {
                    ui.label(self.strings.profile_automatic_active(name));
                }
                for profile in self.profiles.clone() {
                    let active = self.active_profile.as_deref() == Some(&profile.name);
                    let label = if profile.valid {
                        profile.name.clone()
                    } else {
                        format!("{} · {}", profile.name, self.strings.profile_invalid())
                    };
                    if ui
                        .add_enabled(
                            profile.valid && !active && self.profile_receiver.is_none(),
                            egui::Button::new(if active {
                                format!("✓ {label}")
                            } else {
                                label
                            }),
                        )
                        .clicked()
                    {
                        self.request_action(PendingAction::SwitchProfile(profile.name));
                        ui.close();
                    }
                }
                if self.profiles.is_empty() && self.profile_loaded {
                    ui.label(self.strings.no_profiles());
                }
                if let Some(error) = self.profile_error.as_deref() {
                    ui.colored_label(palette.danger, error);
                }
            });
        }
        let more_actions = self.strings.more_actions();
        let actions_response = ui
            .menu_button(more_actions, |ui| {
                ui.menu_button(self.strings.profiles(), |ui| {
                    if !self.profile_loaded && self.profile_receiver.is_none() {
                        self.start_profile_operation(ProfileOperation::List);
                    }
                    if let Some(active) = self.active_profile.as_deref() {
                        ui.label(format!("{}: {active}", self.strings.profiles()));
                    }
                    if let Some(config_path) = runtime::status_field(&self.daemon_status, "config")
                    {
                        let config_path = std::path::Path::new(&config_path);
                        let is_profile = config_path
                            .parent()
                            .and_then(std::path::Path::file_name)
                            .is_some_and(|name| name == "profiles");
                        if is_profile {
                            if let Some(name) =
                                config_path.file_stem().and_then(|name| name.to_str())
                            {
                                ui.label(self.strings.profile_automatic_active(name));
                            }
                        }
                    }
                    match runtime::status_field(&self.daemon_status, "config_state").as_deref() {
                        Some("loading") => {
                            ui.label(self.strings.profile_config_loading());
                        }
                        Some("reload-rejected") => {
                            ui.colored_label(
                                palette.danger,
                                self.strings.profile_config_rejected(),
                            );
                        }
                        _ => {}
                    }
                    if self.profile_receiver.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(self.strings.profile_loading());
                        });
                    }
                    let profiles = self.profiles.clone();
                    for profile in &profiles {
                        let active = self.active_profile.as_deref() == Some(&profile.name);
                        let label = if profile.valid {
                            profile.name.clone()
                        } else {
                            format!("{} · {}", profile.name, self.strings.profile_invalid())
                        };
                        if ui
                            .add_enabled(
                                profile.valid && !active && self.profile_receiver.is_none(),
                                egui::Button::new(if active {
                                    format!("✓ {label}")
                                } else {
                                    label
                                }),
                            )
                            .clicked()
                        {
                            self.request_action(PendingAction::SwitchProfile(profile.name.clone()));
                            ui.close();
                        }
                    }
                    if profiles.is_empty() && self.profile_loaded {
                        ui.label(self.strings.no_profiles());
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut self.profile_name_draft)
                                .hint_text(self.strings.profile_name_hint())
                                .desired_width(120.0),
                        );
                        let name = self.profile_name_draft.trim().to_owned();
                        if ui
                            .add_enabled(
                                !name.is_empty() && self.profile_receiver.is_none(),
                                egui::Button::new(self.strings.create_profile()),
                            )
                            .clicked()
                        {
                            self.request_action(PendingAction::CreateProfile(name));
                            ui.close();
                        }
                    });
                    ui.separator();
                    ui.label(self.strings.profile_app_defaults());
                    ui.label(self.strings.profile_app_id_help());
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut self.profile_app_id_draft)
                                .hint_text(self.strings.profile_app_id_hint())
                                .desired_width(170.0),
                        );
                        let selected = self
                            .profile_app_target
                            .as_deref()
                            .unwrap_or(self.strings.profile_select_app_target());
                        egui::ComboBox::from_id_salt("profile-app-target")
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                for profile in self.profiles.iter().filter(|profile| profile.valid)
                                {
                                    ui.selectable_value(
                                        &mut self.profile_app_target,
                                        Some(profile.name.clone()),
                                        &profile.name,
                                    );
                                }
                            });
                    });
                    let app_id = self.profile_app_id_draft.trim().to_owned();
                    let target_profile = self.profile_app_target.clone();
                    if ui
                        .add_enabled(
                            !app_id.is_empty()
                                && target_profile.is_some()
                                && self.profile_receiver.is_none(),
                            egui::Button::new(self.strings.profile_assign_app()),
                        )
                        .clicked()
                    {
                        if let Some(profile) = target_profile {
                            self.start_profile_operation(ProfileOperation::AssignApp {
                                app_id,
                                profile,
                            });
                        }
                    }
                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .show(ui, |ui| {
                            for assignment in self.profile_app_assignments.clone() {
                                ui.horizontal(|ui| {
                                    ui.small(format!(
                                        "{} → {}",
                                        assignment.app_id, assignment.profile
                                    ));
                                    if ui
                                        .add_enabled(
                                            self.profile_receiver.is_none(),
                                            egui::Button::new(self.strings.profile_unassign_app()),
                                        )
                                        .clicked()
                                    {
                                        self.start_profile_operation(
                                            ProfileOperation::UnassignApp(assignment.app_id),
                                        );
                                    }
                                });
                            }
                        });
                    if self.profile_error.is_some() {
                        ui.colored_label(
                            palette.danger,
                            self.profile_error.clone().unwrap_or_default(),
                        );
                    }
                    if self.profile_receiver.is_none()
                        && ui.button(self.strings.refresh_profiles()).clicked()
                    {
                        self.profile_loaded = false;
                        self.start_profile_operation(ProfileOperation::List);
                    }
                });
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
                if ui.button(self.strings.getting_started()).clicked() {
                    self.open_getting_started();
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
                if ui.button(self.strings.review_packs()).clicked() {
                    self.pack_dialog_open = true;
                    self.pack_review = None;
                    self.pack_error = None;
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
        let mut open_diagnostics = false;
        // The typing-route chip only means something once the service
        // answers; "Unknown" next to "Service: Not running" says nothing new.
        if self.daemon_reachable == Some(true)
            && theme::status_chip(ui, route_label, route_color)
                .on_hover_text(self.daemon_status.clone())
                .clicked()
        {
            open_diagnostics = true;
        }
        if let Some((trust_label, trust_color)) = route_trust {
            if theme::status_chip(ui, &trust_label, trust_color).clicked() {
                open_diagnostics = true;
            }
        }
        if let Some(count) = self
            .daemon_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.output_partial_total)
            .filter(|count| *count > 0)
        {
            let notice = self.strings.output_partial_notice(count);
            if theme::status_chip(
                ui,
                &self.strings.output_partial_badge(count),
                palette.danger,
            )
            .on_hover_text(notice)
            .clicked()
            {
                open_diagnostics = true;
            }
        }
        if let Some(count) = self
            .daemon_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.output_uncertain_total)
            .filter(|count| *count > 0)
        {
            let notice = self.strings.output_uncertain_notice(count);
            if theme::status_chip(
                ui,
                &self.strings.output_uncertain_badge(count),
                palette.warning,
            )
            .on_hover_text(notice)
            .clicked()
            {
                open_diagnostics = true;
            }
        }
        // Whether replacing a trigger is verified against the field's text or
        // relies on keystrokes, shown wherever the route is visible.
        if let Some(capabilities) = self.daemon_capabilities.as_ref() {
            if let Some(assurance) = capabilities.replacement_assurance {
                let color = match assurance {
                    runtime::ReplacementAssurance::Verified => palette.success,
                    runtime::ReplacementAssurance::FocusTrackedBestEffort => palette.warning,
                    runtime::ReplacementAssurance::Unverified
                    | runtime::ReplacementAssurance::Blocked => palette.danger,
                    runtime::ReplacementAssurance::Unavailable => palette.muted,
                };
                let explanation = self.strings.replacement_explanation(assurance);
                if theme::status_chip(ui, self.strings.replacement_pill(assurance), color)
                    .on_hover_text(explanation)
                    .clicked()
                {
                    open_diagnostics = true;
                }
            }
        }
        // The IBus route needs no background service: when its engine is
        // live and the service is not running, say so instead of showing a
        // red "not running" and a "Turn on" button for a working setup.
        let ibus_only = self.daemon_reachable != Some(true) && self.ibus_active;
        if ibus_only {
            if theme::status_chip(ui, self.strings.ibus_active_status(), palette.success)
                .on_hover_text(self.strings.ibus_active_help())
                .clicked()
            {
                open_diagnostics = true;
            }
        } else {
            if self.ibus_active
                && theme::status_chip(ui, self.strings.ibus_active_status(), palette.success)
                    .on_hover_text(self.strings.ibus_active_help())
                    .clicked()
            {
                open_diagnostics = true;
            }
            if theme::status_chip(ui, daemon_label, daemon_color)
                .on_hover_text(self.strings.daemon_status_help())
                .clicked()
            {
                open_diagnostics = true;
            }
        }
        if self.daemon_reachable == Some(true)
            && runtime::daemon_build_state(&self.daemon_status)
                == runtime::DaemonBuildState::RestartRequired
            && theme::status_chip(
                ui,
                self.strings.daemon_restart_required_status(),
                palette.warning,
            )
            .on_hover_text(self.strings.daemon_restart_required_help())
            .clicked()
        {
            open_diagnostics = true;
        }
        if open_diagnostics {
            self.diagnostics_open = true;
            self.refresh_diagnostics(true);
        }
        if self.daemon_reachable == Some(false) && !ibus_only && !self.config.expansion.is_empty() {
            self.turn_on_button(ui, palette);
        }
    }

    pub(crate) fn active_route_contract(
        &self,
    ) -> Option<&'static wayexpand_backend_selection::RouteContract> {
        let route_id = runtime::status_field(&self.daemon_status, "route_id")?;
        route_contract_by_id(&route_id)
    }

    /// The search field, a clear button, and its field-scope menu, filling
    /// the width of the snippet list.
    pub(crate) fn render_search(&mut self, ui: &mut egui::Ui) {
        // Right to left: the buttons take their real width first and the
        // field gets exactly what is left. Guessing the buttons' width made
        // the row wider than the resizable panel whenever the clear button
        // appeared, and the panel then grew every frame to fit it.
        let margin = egui::Margin::symmetric(10, 7);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 0.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                self.render_search_fields_menu(ui);
                if !self.filter.is_empty()
                    && ui
                        .small_button("×")
                        .on_hover_text(self.strings.clear_filters())
                        .clicked()
                {
                    self.filter.clear();
                }
                // egui's desired width is the outer width, margin included.
                let width = ui.available_width().max(40.0);
                ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .id(egui::Id::new(SEARCH_FIELD_SALT))
                        .hint_text(self.strings.search_placeholder())
                        .margin(margin)
                        .desired_width(width),
                )
                .on_hover_text(self.strings.search_tooltip());
            },
        );
    }

    fn render_search_fields_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("⚙", |ui| {
            ui.label(RichText::new(self.strings.search_fields()).small().strong());
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
        })
        .response
        .on_hover_text(self.strings.search_fields());
    }
}

fn runtime_selected_profile(status: &str) -> Option<String> {
    let config = runtime::status_field(status, "config")?;
    let config_path = std::path::Path::new(&config);
    if config_path
        .parent()
        .and_then(std::path::Path::file_name)
        .is_none_or(|name| name != "profiles")
    {
        return None;
    }
    config_path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}
