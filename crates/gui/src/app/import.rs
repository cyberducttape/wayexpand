//! Espanso import: preview, merge or replace, and the import dialog.

use crate::*;

impl GuiApp {
    pub(crate) fn preview_import(&mut self) {
        let source = expand_user_path(self.import_path.trim());
        match import::preview_espanso(&source) {
            Ok((config, report)) => {
                self.import_preview = Some((config, report));
                self.status = Status::success(self.strings.status_import_loaded());
            }
            Err(error) => {
                self.status = Status::error(self.strings.status_import_failed(&error.to_string()))
            }
        }
    }

    pub(crate) fn apply_import(&mut self, replace_library: bool) {
        if self.draft_is_dirty() {
            self.status = Status::warning(self.strings.status_import_needs_clean_draft());
            return;
        }
        let Some((imported, report)) = self.import_preview.take() else {
            return;
        };
        let (candidate, merge_stats) = if replace_library {
            (imported.clone(), None)
        } else {
            let (merged, stats) = import::merge_imported_expansions(&self.config, &imported);
            (merged, Some(stats))
        };
        if let Err(error) = candidate.validate() {
            self.status = Status::error(self.strings.status_import_rejected(&error.safe_summary()));
            self.import_preview = Some((imported, report));
            return;
        }
        let message = if let Some(stats) = merge_stats {
            self.strings.status_import_merged(
                stats.added,
                stats.identical_duplicates,
                stats.conflicts_kept,
                report.fully_migrated,
                report.migrated_with_warnings,
                report.unsupported,
            )
        } else {
            self.strings.status_imported_with_report(
                report.fully_migrated,
                report.migrated_with_warnings,
                report.unsupported,
            )
        };
        if !self.queue_save_config(candidate, SaveIntent::Import(message)) {
            self.import_preview = Some((imported, report));
        }
    }

    pub(crate) fn render_import_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.import_open {
            let mut open = self.import_open;
            egui::Window::new(self.strings.import_dialog_title())
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .min_width(460.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(self.strings.source_yaml());
                    ui.add(
                        TextEdit::singleline(&mut self.import_path)
                            .margin(theme::FIELD_MARGIN)
                            .hint_text("~/.config/espanso/match/base.yml")
                            .desired_width(520.0),
                    );
                    ui.label(
                        RichText::new(self.strings.import_preview_info())
                            .small()
                            .color(palette.muted),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, palette, self.strings.load_preview()).clicked()
                        {
                            self.preview_import();
                        }
                        if theme::secondary_button(ui, palette, self.strings.cancel()).clicked() {
                            self.import_preview = None;
                            self.import_open = false;
                        }
                    });
                    if let Some((_config, report)) = self.import_preview.as_ref() {
                        ui.separator();
                        ui.label(self.strings.import_preview_summary(
                            report.fully_migrated,
                            report.migrated_with_warnings,
                            report.unsupported,
                        ));
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .show(ui, |ui| {
                                for warning in &report.warnings {
                                    ui.label(
                                        RichText::new(format!(
                                            "{}: {}",
                                            warning.trigger,
                                            warning.details.join("; ")
                                        ))
                                        .color(palette.warning),
                                    );
                                }
                                for unsupported in &report.unsupported_matches {
                                    ui.label(
                                        RichText::new(format!(
                                            "{}: {}",
                                            unsupported.trigger, unsupported.reason
                                        ))
                                        .color(palette.danger),
                                    );
                                }
                            });
                        ui.horizontal(|ui| {
                            if theme::primary_button(ui, palette, self.strings.merge_library())
                                .clicked()
                            {
                                self.apply_import(false);
                            }
                            if theme::secondary_button(ui, palette, self.strings.replace_library())
                                .clicked()
                            {
                                self.apply_import(true);
                            }
                        });
                    }
                });
            self.import_open = open && self.import_open;
        }
    }
}
