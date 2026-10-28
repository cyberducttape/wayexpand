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
    error: Option<String>,
    date_months: Vec<(i32, u32)>,
}

impl FormApp {
    pub(crate) fn new(spec: FormSpec, palette: Palette, outcome: Outcome) -> Self {
        let values: Vec<String> = spec
            .fields
            .iter()
            .map(|field| match &field.kind {
                FormFieldKind::Text { default } => default.clone(),
                FormFieldKind::Choice { options, default } => default
                    .clone()
                    .or_else(|| options.first().cloned())
                    .unwrap_or_default(),
                FormFieldKind::Date { default } if default == "today" => {
                    wayexpand_core::today_local_date()
                }
                FormFieldKind::Date { default } => default.clone(),
            })
            .collect();
        let date_months = values
            .iter()
            .map(|value| {
                parse_date(value)
                    .map(|(year, month, _)| (year, month))
                    .unwrap_or((2026, 1))
            })
            .collect();
        Self {
            spec,
            values,
            palette,
            outcome,
            focused_once: false,
            error: None,
            date_months,
        }
    }

    fn submit(&mut self, ctx: &egui::Context) {
        for (field, value) in self.spec.fields.iter().zip(&self.values) {
            if let Err(error) = field.validate_value(value) {
                self.error = Some(format!("{}: {error}", field.label));
                return;
            }
        }
        self.error = None;
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
            .show_inside(root, |ui| {
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
                                FormFieldKind::Choice { options, .. } => {
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
                                FormFieldKind::Date { .. } => {
                                    date_picker(ui, value, &mut self.date_months[index], index);
                                }
                            }
                            ui.end_row();
                        }
                    });
                ui.add_space(12.0);
                if let Some(error) = &self.error {
                    ui.label(RichText::new(error).color(palette.danger));
                }
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

fn date_picker(ui: &mut egui::Ui, value: &mut String, month: &mut (i32, u32), index: usize) {
    let mut parsed = *month;
    let mut selection = None;
    ui.menu_button(value.clone(), |ui| {
        ui.horizontal(|ui| {
            if ui.small_button("‹").clicked() {
                if parsed.1 == 1 {
                    parsed.0 -= 1;
                    parsed.1 = 12;
                } else {
                    parsed.1 -= 1;
                }
                *month = parsed;
            }
            ui.label(format!("{}-{:02}", parsed.0, parsed.1));
            if ui.small_button("›").clicked() {
                if parsed.1 == 12 {
                    parsed.0 += 1;
                    parsed.1 = 1;
                } else {
                    parsed.1 += 1;
                }
                *month = parsed;
            }
        });
        egui::Grid::new(("date_calendar", index)).show(ui, |ui| {
            for label in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                ui.label(label);
            }
            ui.end_row();
            let offset = weekday_monday(parsed.0, parsed.1);
            let days = days_in_month(parsed.0, parsed.1);
            let mut cell = 0;
            for _ in 0..offset {
                ui.label("");
                cell += 1;
            }
            for day in 1..=days {
                if ui.button(day.to_string()).clicked() {
                    selection = Some(format!("{:04}-{:02}-{:02}", parsed.0, parsed.1, day));
                    ui.close();
                }
                cell += 1;
                if cell % 7 == 0 {
                    ui.end_row();
                }
            }
        });
    });
    if let Some(selected) = selection {
        *value = selected;
    }
}

fn parse_date(value: &str) -> Option<(i32, u32, u32)> {
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
    {
        return None;
    }
    let mut parts = value.split('-');
    let date = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if parts.next().is_some()
        || !(1..=12).contains(&date.1)
        || !(1..=days_in_month(date.0, date.1)).contains(&date.2)
    {
        return None;
    }
    Some(date)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn weekday_monday(year: i32, month: u32) -> usize {
    let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let adjusted = year - i32::from(month < 3);
    ((adjusted + adjusted / 4 - adjusted / 100 + adjusted / 400 + offsets[(month - 1) as usize] + 1)
        .rem_euclid(7) as usize
        + 6)
        % 7
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_grid_uses_monday_first_and_leap_days() {
        // 2024-02-01 was a Thursday (Monday-first column 3).
        assert_eq!(weekday_monday(2024, 2), 3);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
    }

    #[test]
    fn date_parser_requires_three_numeric_components() {
        assert_eq!(parse_date("2026-10-27"), Some((2026, 10, 27)));
        assert_eq!(parse_date("2026-10"), None);
        assert_eq!(parse_date("today"), None);
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2025-02-29"), None);
    }
}
