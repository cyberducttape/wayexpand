//! Settings dialog and appearance/language/engine preference changes.

use crate::*;

impl GuiApp {
    /// Opens the settings window with the editable fields reset to what is
    /// currently persisted, so a previously abandoned edit never reappears.
    pub(crate) fn open_settings(&mut self) {
        self.settings_buffer = self.config.settings.max_buffer_chars.to_string();
        self.settings_undo_chord = self.config.settings.undo_chord.clone().unwrap_or_default();
        self.settings_error = None;
        self.settings_open = true;
    }

    pub(crate) fn set_dark_mode(&mut self, ctx: &egui::Context, dark: bool) {
        self.dark_mode = dark;
        ctx.set_theme(if dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        if let Err(error) = save_gui_prefs(
            self.language,
            self.colorpack,
            self.settings_font_scale,
            self.dark_mode,
        ) {
            self.status = Status::warning(self.strings.status_appearance_save_failed(&error));
        }
    }

    pub(crate) fn set_language(&mut self, language: Language) {
        self.language = language;
        self.strings.set_language(language);
        if let Err(error) = save_gui_prefs(
            self.language,
            self.colorpack,
            self.settings_font_scale,
            self.dark_mode,
        ) {
            self.status = Status::warning(self.strings.status_appearance_save_failed(&error));
        }
    }

    pub(crate) fn set_colorpack(&mut self, ctx: &egui::Context, pack: ColorPack) {
        self.colorpack = pack;
        theme::install_pack(ctx, pack, self.settings_font_scale);
        if let Err(error) = save_gui_prefs(
            self.language,
            self.colorpack,
            self.settings_font_scale,
            self.dark_mode,
        ) {
            self.status = Status::warning(self.strings.status_appearance_save_failed(&error));
        }
    }

    /// Applies a font scale immediately and persists it, so it behaves like
    /// the rest of the appearance settings rather than waiting on a Save
    /// button sitting under a different tab. Unlike a library edit this does
    /// not push an undo entry: the undo stack exists to recover snippet
    /// content, and filling it with display-preference steps would bury the
    /// change the user actually wants back.
    pub(crate) fn apply_font_scale(&mut self, ctx: &egui::Context, scale: FontScale) {
        self.settings_font_scale = scale;
        // Apply the style first: the preference is visible even in the
        // unlikely case that persisting it fails, and the failure is
        // reported rather than silently producing a scale that resets on
        // the next launch.
        theme::install_pack(ctx, self.colorpack, scale);
        match save_gui_prefs(self.language, self.colorpack, scale, self.dark_mode) {
            Ok(()) => self.status = Status::success(self.strings.status_font_size_saved()),
            Err(error) => {
                self.status = Status::warning(self.strings.status_appearance_save_failed(&error))
            }
        }
    }

    pub(crate) fn save_settings(&mut self, ctx: &egui::Context) {
        let max_buffer_chars = match self.settings_buffer.trim().parse::<usize>() {
            Ok(value) => value,
            Err(error) => {
                let detail = self
                    .strings
                    .status_buffer_limit_not_a_number(&error.to_string());
                self.status = Status::error(self.strings.status_settings_invalid(&detail));
                self.settings_error = Some(detail);
                return;
            }
        };
        let mut candidate = self.config.clone();
        candidate.settings.max_buffer_chars = max_buffer_chars;
        let undo_chord_input = self.settings_undo_chord.trim();
        candidate.settings.undo_chord =
            (!undo_chord_input.is_empty()).then(|| undo_chord_input.to_owned());
        if let Err(error) = candidate.validate() {
            let detail = error.safe_summary();
            self.status = Status::error(self.strings.status_settings_rejected(&detail));
            self.settings_error = Some(detail);
            return;
        }
        if self.queue_save_config(candidate, SaveIntent::Settings) {
            theme::install_pack(ctx, self.colorpack, self.settings_font_scale);
        }
    }

    pub(crate) fn render_settings_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        let max_size = theme::dialog_max_size(ctx, 560.0);
        egui::Window::new(self.strings.settings_title())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(420.0_f32.min(max_size.x))
            .max_size(max_size)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (SettingsTab::Appearance, self.strings.appearance()),
                        (SettingsTab::Engine, self.strings.engine()),
                    ] {
                        if theme::chip_scaled(
                            ui,
                            palette,
                            label,
                            self.settings_tab == tab,
                            self.settings_font_scale.multiplier(),
                        )
                        .clicked()
                        {
                            self.settings_tab = tab;
                        }
                    }
                });
                ui.add_space(10.0);
                egui::ScrollArea::vertical()
                    .id_salt("settings_scroll")
                    .auto_shrink([false, true])
                    .show(ui, |ui| match self.settings_tab {
                        SettingsTab::Appearance => self.render_appearance_settings(ui, palette),
                        SettingsTab::Engine => self.render_engine_settings(ui, palette),
                    });
            });
        self.settings_open = open && self.settings_open;
    }

    /// Display preferences. Everything here applies and persists on click:
    /// the theme, language, and color pack live in the GUI preferences file,
    /// and the font scale is written straight to the configuration.
    pub(crate) fn render_appearance_settings(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.label(
            RichText::new(self.strings.appearance_note())
                .small()
                .color(palette.muted),
        );
        ui.add_space(10.0);

        theme::section_header(ui, "", self.strings.theme());
        ui.horizontal(|ui| {
            for (dark, label) in [
                (false, self.strings.theme_light()),
                (true, self.strings.theme_dark()),
            ] {
                if theme::chip_scaled(
                    ui,
                    palette,
                    label,
                    self.dark_mode == dark,
                    self.settings_font_scale.multiplier(),
                )
                .clicked()
                {
                    self.set_dark_mode(ui.ctx(), dark);
                }
            }
        });

        ui.add_space(12.0);
        theme::section_header(ui, "", self.strings.language());
        ui.horizontal(|ui| {
            for (language, label) in [
                (Language::English, "English"),
                (Language::German, "Deutsch"),
                (Language::Indonesian, "Bahasa Indonesia"),
            ] {
                if theme::chip_scaled(
                    ui,
                    palette,
                    label,
                    self.language == language,
                    self.settings_font_scale.multiplier(),
                )
                .clicked()
                {
                    self.set_language(language);
                }
            }
        });

        ui.add_space(12.0);
        theme::section_header(ui, "", self.strings.font_size());
        ui.horizontal_wrapped(|ui| {
            for scale in FONT_SCALES {
                if theme::chip_scaled(
                    ui,
                    palette,
                    self.strings.font_scale_label(*scale),
                    *scale == self.settings_font_scale,
                    self.settings_font_scale.multiplier(),
                )
                .clicked()
                {
                    self.apply_font_scale(ui.ctx(), *scale);
                }
            }
        });
        ui.label(
            RichText::new(self.strings.font_size_help())
                .small()
                .color(palette.muted),
        );

        ui.add_space(12.0);
        theme::section_header(ui, "", self.strings.color_pack());
        ui.label(
            RichText::new(self.strings.color_pack_help())
                .small()
                .color(palette.muted),
        );
        ui.add_space(6.0);
        for pack in ColorPack::all() {
            let scheme = ColorScheme::for_pack(*pack, self.dark_mode);
            if theme::colorpack_card(
                ui,
                palette,
                pack.name(),
                pack.description(),
                scheme.accent,
                self.colorpack == *pack,
                self.settings_font_scale.multiplier(),
            )
            .clicked()
            {
                self.set_colorpack(ui.ctx(), *pack);
            }
            ui.add_space(6.0);
        }
    }

    /// Configuration values the daemon reads. These are validated as a whole
    /// and written by an explicit Save, so the dialog says so rather than
    /// leaving the user to discover that half the window commits instantly
    /// and half does not.
    pub(crate) fn render_engine_settings(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.label(
            RichText::new(self.strings.engine_note())
                .small()
                .color(palette.muted),
        );
        ui.add_space(10.0);

        ui.label(self.strings.buffer_limit());
        ui.add(
            TextEdit::singleline(&mut self.settings_buffer)
                .margin(theme::FIELD_MARGIN)
                .desired_width(120.0),
        );
        ui.label(
            RichText::new(self.strings.buffer_limit_help())
                .small()
                .color(palette.muted),
        );

        ui.add_space(10.0);
        ui.label(self.strings.undo_chord());
        ui.add(
            TextEdit::singleline(&mut self.settings_undo_chord)
                .margin(theme::FIELD_MARGIN)
                .hint_text("Ctrl+Z")
                .desired_width(120.0),
        );
        ui.label(
            RichText::new(self.strings.undo_chord_help())
                .small()
                .color(palette.muted),
        );

        if let Some(error) = self.settings_error.clone() {
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(theme::tint(palette.danger, 26))
                .stroke(egui::Stroke::new(1.0, palette.danger))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.colored_label(palette.danger, error);
                });
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if theme::primary_button(ui, palette, self.strings.save_settings()).clicked() {
                let ctx = ui.ctx().clone();
                self.save_settings(&ctx);
            }
            if theme::secondary_button(ui, palette, self.strings.close()).clicked() {
                self.settings_open = false;
            }
        });
    }
}
