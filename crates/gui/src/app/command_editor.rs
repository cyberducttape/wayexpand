use crate::*;

impl GuiApp {
    /// Collapsible editor for a command-backed (dynamic) replacement.
    pub(crate) fn render_command_editor(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.collapsing(self.strings.dynamic_command(), |ui| {
            let Some(draft) = self.draft.as_mut() else {
                ui.label(self.strings.draft_unavailable());
                return;
            };
            ui.checkbox(&mut draft.command_enabled, self.strings.command_checkbox());
            ui.label(
                RichText::new(self.strings.command_help())
                    .small()
                    .color(palette.muted),
            );
            if draft.command_enabled {
                egui::Frame::new()
                    .fill(theme::tint(palette.warning, 30))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.colored_label(palette.warning, self.strings.command_warning());
                    });
            }
            ui.add_enabled_ui(draft.command_enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Execution type");
                    if ui
                        .radio(!draft.command_action_mode, "Direct executable")
                        .clicked()
                    {
                        draft.command_action_mode = false;
                        draft.command_action.clear();
                    }
                    if ui
                        .radio(draft.command_action_mode, "Managed action")
                        .clicked()
                    {
                        draft.command_action_mode = true;
                        draft.command_program.clear();
                        draft.command_args.clear();
                    }
                });
                if draft.command_action_mode {
                    ui.horizontal(|ui| {
                        ui.label("Action");
                        let action_ids = broker_action_ids();
                        egui::ComboBox::from_id_salt("broker_action_id")
                            .selected_text(if draft.command_action.is_empty() {
                                "Select or type an action"
                            } else {
                                &draft.command_action
                            })
                            .show_ui(ui, |ui| {
                                for action_id in action_ids {
                                    if ui
                                        .selectable_label(
                                            draft.command_action == action_id,
                                            &action_id,
                                        )
                                        .clicked()
                                    {
                                        draft.command_action = action_id;
                                    }
                                }
                            });
                        ui.add(
                            TextEdit::singleline(&mut draft.command_action)
                                .margin(theme::FIELD_MARGIN)
                                .hint_text("cluster-status")
                                .desired_width(240.0),
                        );
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label(self.strings.program());
                        ui.add(
                            TextEdit::singleline(&mut draft.command_program)
                                .margin(theme::FIELD_MARGIN)
                                .hint_text(self.strings.program_hint())
                                .desired_width(300.0),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(self.strings.timeout_ms());
                    ui.add(
                        TextEdit::singleline(&mut draft.command_timeout_ms)
                            .margin(theme::FIELD_MARGIN)
                            .desired_width(90.0),
                    );
                    ui.label(self.strings.cache_ms());
                    ui.add(
                        TextEdit::singleline(&mut draft.command_cache_ms)
                            .margin(theme::FIELD_MARGIN)
                            .desired_width(90.0),
                    );
                });
                if !draft.command_action_mode {
                    ui.label(self.strings.arguments());
                    let mut remove_arg = None;
                    let mut move_arg = None;
                    for index in 0..draft.command_args.len() {
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::multiline(&mut draft.command_args[index])
                                    .margin(theme::FIELD_MARGIN)
                                    .desired_rows(1)
                                    .desired_width(
                                        (ui.available_width() - 108.0).max(MIN_FIELD_WIDTH),
                                    ),
                            );
                            if ui
                                .small_button("↑")
                                .on_hover_text(self.strings.move_up())
                                .clicked()
                                && index > 0
                            {
                                move_arg = Some((index, index - 1));
                            }
                            if ui
                                .small_button("↓")
                                .on_hover_text(self.strings.move_down())
                                .clicked()
                                && index + 1 < draft.command_args.len()
                            {
                                move_arg = Some((index, index + 1));
                            }
                            if ui
                                .small_button("×")
                                .on_hover_text(self.strings.remove_argument())
                                .clicked()
                            {
                                remove_arg = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove_arg {
                        draft.command_args.remove(index);
                    }
                    if let Some((from, to)) = move_arg {
                        draft.command_args.swap(from, to);
                    }
                    if ui.small_button(self.strings.add_argument()).clicked() {
                        draft.command_args.push(String::new());
                    }
                }
                ui.horizontal(|ui| {
                    ui.label(self.strings.environment())
                        .on_hover_text(self.strings.environment_tooltip());
                    let minimal = self.strings.environment_minimal();
                    let inherit = self.strings.environment_inherit();
                    egui::ComboBox::from_id_salt("command_environment")
                        .selected_text(match draft.command_environment {
                            wayexpand_core::CommandEnvironment::Minimal => minimal,
                            wayexpand_core::CommandEnvironment::Inherit => inherit,
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.command_environment,
                                wayexpand_core::CommandEnvironment::Minimal,
                                minimal,
                            );
                            ui.selectable_value(
                                &mut draft.command_environment,
                                wayexpand_core::CommandEnvironment::Inherit,
                                inherit,
                            );
                        });
                });
                ui.label(self.strings.pass_environment());
                let mut remove_env = None;
                for index in 0..draft.command_pass_env.len() {
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut draft.command_pass_env[index])
                                .margin(theme::FIELD_MARGIN)
                                .desired_width((ui.available_width() - 38.0).max(MIN_FIELD_WIDTH)),
                        );
                        if ui
                            .small_button("×")
                            .on_hover_text(self.strings.remove_environment_variable())
                            .clicked()
                        {
                            remove_env = Some(index);
                        }
                    });
                }
                if let Some(index) = remove_env {
                    draft.command_pass_env.remove(index);
                }
                if ui
                    .small_button(self.strings.add_environment_variable())
                    .clicked()
                {
                    draft.command_pass_env.push(String::new());
                }
            });
        });
    }
}
