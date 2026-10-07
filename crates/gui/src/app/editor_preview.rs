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
                    ui.label(if managed_action {
                        self.strings.managed_command_preview_help()
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
            let preview_text = self.preview();
            egui::Frame::new()
                .fill(palette.extreme_bg)
                .stroke(egui::Stroke::new(1.0, palette.accent))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&preview_text).monospace());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            if ui
                                .small_button(self.strings.copy())
                                .on_hover_text(self.strings.copy_tooltip())
                                .clicked()
                            {
                                ui.ctx().copy_text(preview_text.clone());
                                self.status = Status::success(self.strings.status_preview_copied());
                            }
                        });
                    });
                });
        }
    }
}
