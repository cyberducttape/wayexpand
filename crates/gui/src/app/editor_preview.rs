use crate::*;

impl GuiApp {
    /// Live preview of the draft, including the command preview runner.
    pub(crate) fn render_preview_section(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.add_space(14.0);
        theme::section_header(ui, "", self.strings.preview());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(self.strings.input());
            ui.add(
                TextEdit::singleline(&mut self.preview_input)
                    .margin(theme::FIELD_MARGIN)
                    .hint_text(self.strings.input_hint())
                    .desired_width(420.0),
            );
            if theme::secondary_button(ui, palette, self.strings.use_trigger()).clicked() {
                self.preview_input = self
                    .draft
                    .as_ref()
                    .map(|draft| draft.trigger.clone())
                    .unwrap_or_default();
            }
        });
        ui.add_space(4.0);
        let command_backed = self
            .draft
            .as_ref()
            .is_some_and(|draft| draft.command_enabled);
        if command_backed {
            // Never auto-run a configured program from a render/repaint
            // path: unlike a template, this has real side effects and
            // this code runs every frame the editor is open. Running is
            // opt-in via the button below, and only ever once per click.
            egui::Frame::new()
                .fill(theme::tint(palette.warning, 20))
                .stroke(egui::Stroke::new(1.0, palette.warning))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    let managed_action = self
                        .draft
                        .as_ref()
                        .is_some_and(|draft| draft.command_action_mode);
                    let sandboxed_preview = wayexpand_core::command_preview_sandboxed();
                    ui.label(if managed_action {
                        self.strings.managed_command_preview_help()
                    } else if sandboxed_preview {
                        self.strings.sandboxed_direct_command_preview_help()
                    } else {
                        self.strings.direct_command_preview_help()
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let preview_is_current =
                            self.command_preview_key == Some(self.preview_revision);
                        if ui
                            .add_enabled(
                                self.command_preview_receiver.is_none(),
                                egui::Button::new(if self.command_preview_receiver.is_some() {
                                    self.strings.running()
                                } else if managed_action {
                                    self.strings.run_through_broker()
                                } else if sandboxed_preview {
                                    self.strings.run_in_service_sandbox()
                                } else {
                                    self.strings.run_once()
                                }),
                            )
                            .clicked()
                        {
                            self.run_command_preview();
                        }
                        match self
                            .command_preview_result
                            .as_ref()
                            .filter(|_| preview_is_current)
                        {
                            Some(Ok(output)) => {
                                let shown: String = if output.chars().count() > 200 {
                                    output.chars().take(199).collect::<String>() + "…"
                                } else {
                                    output.clone()
                                };
                                ui.label(RichText::new(shown).monospace());
                            }
                            Some(Err(message)) => {
                                ui.colored_label(palette.danger, message);
                            }
                            None => {}
                        }
                    });
                });
        } else {
            let preview_result = self.preview();
            egui::Frame::new()
                .fill(palette.extreme_bg)
                .stroke(egui::Stroke::new(1.0, palette.accent))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| match preview_result {
                    Ok(preview_text) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&preview_text).monospace());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                                if ui
                                    .small_button(self.strings.copy())
                                    .on_hover_text(self.strings.copy_tooltip())
                                    .clicked()
                                {
                                    ui.ctx().copy_text(preview_text.clone());
                                    self.status =
                                        Status::success(self.strings.status_preview_copied());
                                }
                            });
                        });
                    }
                    Err(error) => {
                        ui.colored_label(palette.danger, self.strings.template_error(&error));
                    }
                });
        }
        self.render_form_test(ui, palette);
    }

    fn render_form_test(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        let fields = match wayexpand_core::form_fields(&draft.replacement) {
            Ok(fields) if !fields.is_empty() => fields,
            _ => return,
        };
        let template = draft.replacement.clone();
        let snippets = Arc::clone(&self.preview_library_snippets);
        ui.collapsing(self.strings.form_test_values(), |ui| {
            ui.label(
                RichText::new(self.strings.form_preview_privacy())
                    .small()
                    .color(palette.muted),
            );
            for (index, field) in fields.iter().enumerate() {
                let default = match &field.kind {
                    wayexpand_core::FormFieldKind::Text { default } => default.clone(),
                    wayexpand_core::FormFieldKind::Choice { options } => {
                        options.first().cloned().unwrap_or_default()
                    }
                };
                let value = self
                    .form_preview_values
                    .entry(field.key.clone())
                    .or_insert(default);
                ui.horizontal(|ui| {
                    ui.label(&field.label);
                    match &field.kind {
                        wayexpand_core::FormFieldKind::Text { .. } => {
                            ui.add(TextEdit::singleline(value).desired_width(260.0));
                        }
                        wayexpand_core::FormFieldKind::Choice { options } => {
                            egui::ComboBox::from_id_salt(("form_test_choice", index))
                                .selected_text(value.as_str())
                                .show_ui(ui, |ui| {
                                    for option in options {
                                        ui.selectable_value(value, option.clone(), option);
                                    }
                                });
                        }
                    }
                });
            }
            match preview::render_form_template(
                &template,
                &self.form_preview_values,
                Arc::clone(&snippets),
            ) {
                Ok((rendered, cursor_offset)) => {
                    ui.label(self.strings.form_template_rendered());
                    egui::Frame::new()
                        .fill(palette.extreme_bg)
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(rendered.clone()).monospace())
                                    .wrap(),
                            );
                        });
                    if let Some(offset) = cursor_offset {
                        let position = rendered.chars().count().saturating_sub(offset);
                        ui.label(
                            RichText::new(self.strings.cursor_placement(position))
                                .small()
                                .color(palette.muted),
                        );
                    }
                }
                Err(error) => {
                    ui.colored_label(palette.danger, self.strings.template_error(&error));
                }
            }
        });
    }
}
