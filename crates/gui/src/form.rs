//! Snippet form window (`wayexpand-gui --form SPEC`).
//!
//! The daemon starts this when a snippet with `{{field:...}}` or
//! `{{choice:...}}` markers is triggered. Tab moves between fields, Enter
//! submits, Escape (or closing the window) cancels. Submitted values are
//! printed to stdout as a JSON object keyed by field key and the process
//! exits 0; a cancel exits 1, and the daemon leaves the typed trigger alone.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use eframe::egui::{self, RichText};
use serde::Deserialize;
use wayexpand_core::{FormField, FormFieldKind};

use crate::theme::{self, Palette};

#[derive(Debug, Deserialize)]
pub(crate) struct FormSpec {
    pub(crate) title: String,
    pub(crate) fields: Vec<FormField>,
}

type Outcome = Arc<Mutex<Option<HashMap<String, String>>>>;

pub(crate) struct FormApp {
    spec: FormSpec,
    values: Vec<String>,
    palette: Palette,
    outcome: Outcome,
    focused_once: bool,
}

impl FormApp {
    pub(crate) fn new(spec: FormSpec, palette: Palette, outcome: Outcome) -> Self {
        let values = spec
            .fields
            .iter()
            .map(|field| match &field.kind {
                FormFieldKind::Text { default } => default.clone(),
                FormFieldKind::Choice { options } => options[0].clone(),
            })
            .collect();
        Self {
            spec,
            values,
            palette,
            outcome,
            focused_once: false,
        }
    }

    fn submit(&self, ctx: &egui::Context) {
        let values = self
            .spec
            .fields
            .iter()
            .zip(&self.values)
            .map(|(field, value)| (field.key.clone(), value.clone()))
            .collect();
        if let Ok(mut outcome) = self.outcome.lock() {
            *outcome = Some(values);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for FormApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.render(root);
    }
}

impl FormApp {
    pub(crate) fn render(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let (enter, escape) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if escape {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if enter {
            self.submit(&ctx);
            return;
        }
        let palette = self.palette;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.background)
                    .inner_margin(egui::Margin::symmetric(18, 14)),
            )
            .show(root, |ui| {
                ui.label(
                    RichText::new(format!("Fill in {}", self.spec.title))
                        .strong()
                        .size(16.0),
                );
                ui.add_space(10.0);
                egui::Grid::new("form_fields")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        for (index, field) in self.spec.fields.iter().enumerate() {
                            ui.label(&field.label);
                            let value = &mut self.values[index];
                            match &field.kind {
                                FormFieldKind::Text { .. } => {
                                    let response = ui.add(
                                        egui::TextEdit::singleline(value)
                                            .margin(theme::FIELD_MARGIN)
                                            .desired_width(320.0),
                                    );
                                    if index == 0 && !self.focused_once {
                                        response.request_focus();
                                        self.focused_once = true;
                                    }
                                }
                                FormFieldKind::Choice { options } => {
                                    egui::ComboBox::from_id_salt(("choice", index))
                                        .selected_text(value.as_str())
                                        .width(320.0)
                                        .show_ui(ui, |ui| {
                                            for option in options {
                                                ui.selectable_value(
                                                    value,
                                                    option.clone(),
                                                    option.as_str(),
                                                );
                                            }
                                        });
                                }
                            }
                            ui.end_row();
                        }
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, &palette, "Insert").clicked() {
                        self.submit(&ctx);
                    }
                    if theme::secondary_button(ui, &palette, "Cancel").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ui.label(
                        RichText::new("Tab moves between fields · Enter inserts · Esc cancels")
                            .small()
                            .color(palette.muted),
                    );
                });
            });
    }
}

/// Show the form; print the values and exit 0, or exit 1 on cancel.
pub(crate) fn run(spec_json: &str) -> anyhow::Result<()> {
    let spec: FormSpec = serde_json::from_str(spec_json)
        .map_err(|error| anyhow::anyhow!("invalid form specification: {error}"))?;
    if spec.fields.is_empty() || spec.fields.len() > 32 {
        anyhow::bail!("a form needs 1 to 32 fields");
    }
    let prefs = crate::settings::load_gui_prefs();
    let outcome: Outcome = Arc::default();
    let app_outcome = Arc::clone(&outcome);
    let height = 120.0 + 40.0 * spec.fields.len() as f32;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WayExpand — Fill in snippet")
            .with_app_id("io.github.cyberducttape.WayExpand.Form")
            .with_inner_size([520.0, height.min(720.0)])
            .with_icon(crate::app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "WayExpand form",
        options,
        Box::new(move |creation_context| {
            crate::fonts::install(&creation_context.egui_ctx);
            theme::install_pack(
                &creation_context.egui_ctx,
                prefs.colorpack,
                prefs.font_scale,
            );
            let dark = prefs
                .dark_mode
                .unwrap_or_else(|| creation_context.egui_ctx.theme() == egui::Theme::Dark);
            creation_context.egui_ctx.set_theme(if dark {
                egui::ThemePreference::Dark
            } else {
                egui::ThemePreference::Light
            });
            Ok(Box::new(FormApp::new(
                spec,
                Palette::for_pack(prefs.colorpack, dark),
                app_outcome,
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!("form failed: {error}"))?;
    let values = outcome.lock().ok().and_then(|mut outcome| outcome.take());
    match values {
        Some(values) => {
            println!("{}", serde_json::to_string(&values)?);
            Ok(())
        }
        None => std::process::exit(1),
    }
}
