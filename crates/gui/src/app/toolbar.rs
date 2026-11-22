//! Toolbar: brand, route status, pause/run controls, and library search.

use crate::lang::{HeaderState, RouteTrustDetails};
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
                // Keep the brand and controls on one line while they fit.
                // Otherwise separate them into rows: a right-to-left control
                // layout placed in the remainder of a horizontal row overlaps
                // the brand once its labels no longer fit. The width the row
                // needs depends on the status chips, so it is measured each
                // time the row is laid out rather than guessed.
                let compact_width =
                    (920.0 * self.settings_font_scale.multiplier()).max(self.toolbar_row_width);
                let compact = ui.available_width() < compact_width;
                if compact {
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| self.render_toolbar_brand(ui, palette));
                        ui.add_space(6.0);
                        // Right to left, like the wide layout, so the primary
                        // action keeps its place at the end of the row. The
                        // zero height keeps this centered layout from claiming
                        // the whole window and hiding the panels below.
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 0.0),
                            egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
                            |ui| self.render_toolbar_controls(ui, palette, true),
                        );
                    });
                } else {
                    let row_width = ui.available_width();
                    ui.horizontal(|ui| {
                        // One fixed row height, whichever chips and buttons
                        // are visible, so the panels below never jump.
                        ui.set_min_height(34.0);
                        let brand = ui
                            .horizontal(|ui| self.render_toolbar_brand(ui, palette))
                            .response
                            .rect
                            .width();
                        let controls = ui
                            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                self.render_toolbar_controls(ui, palette, false);
                                ui.min_rect().width()
                            })
                            .inner;
                        let spacing = ui.spacing().item_spacing.x * 2.0 + 16.0;
                        let needed = brand + controls + spacing;
                        self.toolbar_row_width = needed;
                        if needed > row_width {
                            // This frame overlapped; redo it in compact rows.
                            ui.ctx()
                                .request_discard("toolbar no longer fits on one row");
                        }
                    });
                }
            });
        egui::Panel::top("experience_switcher")
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(18, 4))
                    .stroke(egui::Stroke::NONE),
            )
            .show(root, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(self.strings.experience_label())
                            .small()
                            .color(palette.muted),
                    );
                    for (experience, label, help) in [
                        (
                            GuiExperience::Everyday,
                            self.strings.experience_everyday(),
                            self.strings.experience_everyday_help(),
                        ),
                        (
                            GuiExperience::Advanced,
                            self.strings.experience_advanced(),
                            self.strings.experience_advanced_help(),
                        ),
                        (
                            GuiExperience::DeveloperAdmin,
                            self.strings.experience_developer_admin(),
                            self.strings.experience_developer_admin_help(),
                        ),
                    ] {
                        let selected = self.experience == experience;
                        if ui
                            .selectable_label(selected, label)
                            .on_hover_text(help)
                            .clicked()
                        {
                            self.experience = experience;
                            self.editor_advanced = match experience {
                                GuiExperience::Everyday => None,
                                GuiExperience::Advanced | GuiExperience::DeveloperAdmin => {
                                    Some(true)
                                }
                            };
                        }
                    }
                });
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
        let (daemon_label, _) = match self.daemon_reachable {
            Some(true) => (self.strings.daemon_running_status(), palette.success),
            Some(false) if self.config.expansion.is_empty() => {
                (self.strings.daemon_not_enabled_status(), palette.muted)
            }
            Some(false) => (self.strings.daemon_unreachable_status(), palette.danger),
            None => (self.strings.daemon_unknown_status(), palette.muted),
        };
        let (route_label, _) = if self.daemon_reachable == Some(true)
            && runtime::daemon_build_state(&self.daemon_status)
                != runtime::DaemonBuildState::Current
        {
            // A stale or schema-incompatible status cannot prove that the
            // reported connection is still the route this GUI is running.
            (self.strings.route_unknown_status(), palette.warning)
        } else {
            match self.route_state {
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
            }
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
                self.strings.route_trust_status(RouteTrustDetails {
                    route: &contract.label,
                    maturity: &contract.status,
                    sensitive_fields: sensitive_field_protection,
                    trigger_precondition: &contract.trigger_precondition,
                    output_transaction: &contract.output_transaction,
                    application_result: &contract.application_result,
                    certified: contract.is_certified(),
                }),
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
        if self.experience.includes_advanced()
            && !compact
            && !self.profile_loaded
            && self.profile_receiver.is_none()
        {
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
        // Profiles are an advanced feature: the quick switcher appears once
        // there is something to switch between. Management stays in the menu.
        let has_profiles =
            !self.profiles.is_empty() || runtime_profile.is_some() || self.active_profile.is_some();
        if self.experience.includes_advanced() && !compact && has_profiles {
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
        let advanced_experience = self.experience.includes_advanced();
        let developer_experience = self.experience.includes_developer_admin();
        let actions_response = ui
            .menu_button(RichText::new("☰").size(18.0), |ui| {
                if advanced_experience {
                    ui.menu_button(self.strings.profiles(), |ui| {
                        if !self.profile_loaded && self.profile_receiver.is_none() {
                            self.start_profile_operation(ProfileOperation::List);
                        }
                        if let Some(active) = self.active_profile.as_deref() {
                            ui.label(format!("{}: {active}", self.strings.profiles()));
                        }
                        if let Some(config_path) =
                            runtime::status_field(&self.daemon_status, "config")
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
                        match runtime::status_field(&self.daemon_status, "config_state").as_deref()
                        {
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
                                self.request_action(PendingAction::SwitchProfile(
                                    profile.name.clone(),
                                ));
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
                                    for profile in
                                        self.profiles.iter().filter(|profile| profile.valid)
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
                                                egui::Button::new(
                                                    self.strings.profile_unassign_app(),
                                                ),
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
                }
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
                if ui.button(self.strings.set_picker_shortcut()).clicked() {
                    self.set_picker_shortcut();
                    ui.close();
                }
                if advanced_experience
                    && self.library_is_synchronized()
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
                if advanced_experience && ui.button(self.strings.import_espanso()).clicked() {
                    self.import_open = true;
                    self.import_preview = None;
                    ui.close();
                }
                if developer_experience && ui.button(self.strings.review_packs()).clicked() {
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
        // One status chip with a one-word state. The facts behind it -- the
        // service, the typing route, sensitive-field protection, replacement
        // assurance -- are in its tooltip and in diagnostics; as separate
        // chips they crowded the header and read as several half-failures.
        // The control-only service (stdin source) captures no typing, so it
        // has no typing route to report.
        let control_only =
            runtime::status_field(&self.daemon_status, "source").as_deref() == Some("stdin");
        let ibus_route_active = self.ibus_route_active();
        let ibus_only = self.daemon_reachable != Some(true) && ibus_route_active;
        let (state, state_color) = self.header_state(ibus_only, control_only, palette);
        let mut details = Vec::new();
        if ibus_only || ibus_route_active {
            details.push(format!(
                "{} — {}",
                self.strings.ibus_active_status(),
                self.strings.ibus_active_help()
            ));
        }
        if !ibus_only {
            details.push(format!(
                "{} — {}",
                daemon_label,
                self.strings.daemon_status_help()
            ));
        }
        if self.daemon_reachable == Some(true) && !control_only {
            details.push(route_label.to_owned());
        }
        if let Some((trust_label, _)) = route_trust.as_ref() {
            details.push(trust_label.clone());
        }
        if let Some(assurance) = self
            .daemon_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.replacement_assurance)
        {
            details.push(format!(
                "{} — {}",
                self.strings.replacement_pill(assurance),
                self.strings.replacement_explanation(assurance)
            ));
        }
        let picker = self.picker_readiness();
        details.push(self.strings.picker_readiness_detail(picker));
        details.push(self.strings.header_status_details_hint().to_owned());
        // The last test result is history, shown apart from the live
        // state: when it no longer covers this session, say why.
        if let Some((record, staleness)) = self.verification_status() {
            if let (Some(application), Some(at)) =
                (record.application.as_deref(), record.verified_at_unix)
            {
                details.push(self.strings.last_test_result(
                    application,
                    &crate::setup_record::date(at),
                    record.verification_method
                        == Some(crate::setup_record::VerificationMethod::MachineObserved),
                ));
            }
            if let Some(change) = staleness {
                details.insert(
                    0,
                    self.strings
                        .setup_changed_since_test(self.strings.setup_change(&change)),
                );
            } else if !self.setup_record_saved {
                details.insert(0, self.strings.verification_not_saved().to_owned());
            }
        }
        // Lead with what to do, the same answer the banner under the header
        // gives, before the facts behind it.
        if let Some((problem, action)) = self.current_next_step() {
            details.insert(
                0,
                format!(
                    "{} → {}",
                    self.strings.problem_text(problem),
                    self.strings.action_label(action)
                ),
            );
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
        // Typed triggers and the Quick Picker are ready in different
        // situations (most routes expand triggers but leave the picker
        // copying), so each has its own chip. A library with no snippets
        // has nothing to pick yet.
        if !compact && !self.config.expansion.is_empty() {
            let color = match picker {
                crate::picker::PickerReadiness::TypesDirectly => palette.success,
                crate::picker::PickerReadiness::Copies(_) => palette.muted,
                crate::picker::PickerReadiness::Unavailable(_) => palette.warning,
            };
            let label = self.strings.picker_readiness_chip(picker);
            let detail = self.strings.picker_readiness_detail(picker);
            let response = theme::status_chip(ui, label, color);
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &detail));
            if response.on_hover_text(&detail).clicked() {
                open_diagnostics = true;
            }
        }
        if let Some((record, stale)) = self.verification_status() {
            if let Some(application) = record.application.as_deref() {
                let machine_observed = record.verification_method
                    == Some(crate::setup_record::VerificationMethod::MachineObserved);
                let label =
                    self.strings
                        .verification_badge(application, machine_observed, stale.is_some());
                let mut detail = record
                    .verified_at_unix
                    .map(crate::setup_record::date)
                    .map(|date| {
                        self.strings
                            .last_test_result(application, &date, machine_observed)
                    })
                    .unwrap_or_default();
                if !detail.is_empty() {
                    detail.push(' ');
                }
                detail.push_str(self.strings.verification_scope());
                if let Some(change) = stale.as_ref() {
                    detail.push(' ');
                    detail.push_str(
                        &self
                            .strings
                            .setup_changed_since_test(self.strings.setup_change(change)),
                    );
                }
                let color = if stale.is_some() {
                    palette.warning
                } else {
                    palette.muted
                };
                theme::status_chip(ui, &label, color).on_hover_text(detail);
            }
        }
        let state_label = self.strings.header_state(state);
        let chip = theme::status_chip(ui, state_label, state_color);
        // Screen readers get the same details sighted users see on hover,
        // after naming what this chip is about.
        let accessible = format!(
            "{}: {state_label}. {}",
            self.strings.typed_triggers_label(),
            details.join(". ")
        );
        details.insert(
            0,
            format!("{}: {state_label}", self.strings.typed_triggers_label()),
        );
        chip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &accessible));
        if chip.on_hover_text(details.join("\n")).clicked() {
            open_diagnostics = true;
        }
        if open_diagnostics {
            self.diagnostics_open = true;
            self.refresh_diagnostics(true);
        }
    }

    /// The header chip's state and color. It follows the same readiness
    /// model as the banner and the first-run guide: a live process alone is
    /// never success. A problem `next_step` names wins; with none, the chip
    /// is green only for a user-confirmed insertion that still covers this
    /// session, build, route and route instance (`expansion_observed`).
    /// Backend acknowledgements never count.
    pub(crate) fn header_state(
        &self,
        ibus_only: bool,
        control_only: bool,
        palette: &Palette,
    ) -> (HeaderState, Color32) {
        use super::Problem as P;
        use HeaderState as S;
        if let Some((problem, _)) = self.current_next_step() {
            return match problem {
                P::ServiceOff if self.config.expansion.is_empty() => (S::NotSetUp, palette.muted),
                P::ServiceOff => (S::Off, palette.danger),
                P::Paused => (S::Paused, palette.warning),
                P::PermissionNeeded | P::PermissionRevoked => (S::NeedsPermission, palette.warning),
                P::Connecting if self.route_state == Some(runtime::RouteState::Reconnecting) => {
                    (S::Reconnecting, palette.warning)
                }
                P::Connecting => (S::Starting, palette.warning),
                P::RouteFailed if self.route_state == Some(runtime::RouteState::Stopped) => {
                    (S::Stopped, palette.muted)
                }
                P::RouteFailed => (S::Disconnected, palette.danger),
                P::Unsupported => (S::CopyOnly, palette.muted),
                P::NoTypingRoute => (S::NeedsSetup, palette.warning),
                P::InstallationIncomplete => (S::NeedsSetup, palette.warning),
                P::NoEnabledExpansions => (S::NotSetUp, palette.warning),
                P::FieldCannotReplace
                | P::SensitiveField
                | P::TerminalRouteNoInput
                | P::ApplicationOutsideScope => (S::Limited, palette.warning),
                P::ApplicationDeliveryUnverified
                | P::IbusInputNotObserved
                | P::FieldTextMismatch
                | P::FieldPolicyRefusal
                | P::IbusFieldTypeUnknown
                | P::TriggerNotMatched
                | P::TriggerMatchUnknown
                | P::IbusDesktopRouteRecentlyCrashed
                | P::OutputUnverified
                | P::PartialOutput
                | P::OutputNotApplied
                | P::OutputBlockedByPolicy
                | P::OutputInterrupted
                | P::SnippetActionFailed
                | P::OutputStatusUnknown => (S::NeedsVerification, palette.accent),
                P::InputStatusPending
                | P::GnomeSourceStatusUnavailable
                | P::ReplacementConsentUnknown => (S::Checking, palette.muted),
                P::RestartNeeded
                | P::BuildUnknown
                | P::InputSourceNotSetUp
                | P::IbusEngineNotRegistered
                | P::IbusManifestScopeInvalid
                | P::IbusDesktopRouteUnavailable
                | P::InputSourceNotSelected
                | P::IbusEngineUnavailable
                | P::TerminalFocusUnknown
                | P::TerminalCompositorIdentity
                | P::InputStatusStale
                | P::ReplacementNotAllowed => (S::NeedsSetup, palette.warning),
                P::StatusUnknown | P::RouteStatusUnknown => (S::Checking, palette.muted),
            };
        }
        let ibus_route = ibus_only || control_only && self.ibus_route_active();
        if !ibus_route {
            match self.daemon_reachable {
                None => return (S::Checking, palette.muted),
                Some(false) => return (S::Off, palette.danger),
                Some(true) => {}
            }
            if !control_only {
                match self.route_state {
                    None => return (S::Checking, palette.muted),
                    Some(runtime::RouteState::Degraded) => return (S::Limited, palette.warning),
                    _ => {}
                }
                let full_protection = self
                    .daemon_capabilities
                    .as_ref()
                    .is_some_and(|capabilities| capabilities.provides_full_protection());
                if !full_protection {
                    return (S::Limited, palette.warning);
                }
            }
        }
        if self.expansion_observed() {
            // Verified, but only in memory: say it lasts this session only.
            if self.setup_record_saved {
                (S::Working, palette.success)
            } else {
                (S::WorkingUnsaved, palette.success)
            }
        } else {
            (S::NeedsVerification, palette.accent)
        }
    }

    /// What the Quick Picker would do if opened now, by the same rule the
    /// picker applies on launch.
    pub(crate) fn picker_readiness(&self) -> crate::picker::PickerReadiness {
        let status = (self.daemon_reachable == Some(true)).then_some(self.daemon_status.as_str());
        let policy = crate::picker::PickerPolicy::merge(
            &self.config.organization,
            self.administrator_policy.as_ref(),
        );
        crate::picker::picker_readiness(status, policy)
    }

    /// The typing route as a setup record names it: "ibus" for the IBus
    /// route, "<source> → <backend>" for a connected daemon, `None` while
    /// unknown.
    pub(crate) fn current_route_label(&self) -> Option<String> {
        // A saved real-app confirmation is scoped to the route actually
        // running now. Do not let old or schema-incompatible daemon status
        // make an earlier route name/instance look current.
        if self.daemon_reachable == Some(true)
            && runtime::daemon_build_state(&self.daemon_status)
                != runtime::DaemonBuildState::Current
        {
            return None;
        }
        let control_only =
            runtime::status_field(&self.daemon_status, "source").as_deref() == Some("stdin");
        if self.ibus_route_active() && (self.daemon_reachable != Some(true) || control_only) {
            return Some("ibus".into());
        }
        if self.daemon_reachable != Some(true)
            || self.route_state != Some(runtime::RouteState::Connected)
        {
            return None;
        }
        let source = runtime::status_field(&self.daemon_status, "source")?;
        let backend = runtime::status_field(&self.daemon_status, "backend")?;
        Some(format!("{source} → {backend}"))
    }

    /// Which run of the typing route is live: the IBus engine instance, or
    /// the daemon's input and output connection times. A restart or
    /// reconnect changes it. `None` while unknown.
    pub(crate) fn current_route_instance(&self) -> Option<String> {
        if self.current_route_label().as_deref() == Some("ibus") {
            return self.ibus_instance.clone();
        }
        let input = runtime::status_field(&self.daemon_status, "input_last_connected_unix")?;
        let output = runtime::status_field(&self.daemon_status, "output_last_connected_unix")?;
        Some(format!("daemon:{input}:{output}"))
    }

    /// The saved verification and why it does not cover what is running
    /// now (`None` when it does).
    pub(crate) fn verification_status(
        &self,
    ) -> Option<(
        &crate::setup_record::SetupRecord,
        Option<crate::setup_record::Staleness>,
    )> {
        let record = self
            .setup_record
            .as_ref()
            .filter(|record| record.outcome == crate::setup_record::SetupOutcome::Verified)?;
        let route = self.current_route_label();
        let instance = self.current_route_instance();
        let desktop = crate::setup_record::current_desktop();
        let session = crate::setup_record::current_session();
        let build = crate::setup_record::current_build();
        let config_revision = self.config_revision.fingerprint();
        let scope = crate::setup_record::Scope {
            route: route.as_deref(),
            route_instance: instance.as_deref(),
            config_revision: Some(&config_revision),
            desktop: &desktop,
            session: session.as_deref(),
            build: &build,
        };
        Some((record, record.staleness(&scope)))
    }

    /// Whether the user confirmed that an expansion reached an application
    /// through what is running now: this compositor session, this build,
    /// this typing route on this desktop. Backend acknowledgements do not
    /// count; they are process-wide and do not show the application's
    /// resulting text.
    pub(crate) fn expansion_observed(&self) -> bool {
        self.verification_status()
            .is_some_and(|(_, staleness)| staleness.is_none())
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
        ui.menu_button("⏷", |ui| {
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
