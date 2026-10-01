//! Quick-insert picker: `wayexpand-gui --picker`.
//!
//! Bind it to a desktop shortcut, type a few letters, press Enter, and the
//! snippet is typed into the application you were using. The picker never
//! injects text itself: when the window closes, focus returns to that
//! application and the running daemon -- which already holds the approved
//! injection backend -- types the snippet (`insert <trigger>` on the control
//! socket), under the same pause, password-field, and app-filter rules as a
//! typed trigger. Without a daemon, Enter copies the snippet instead.

use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, RichText, TextEdit};
use wayexpand_core::{
    render_template_with_cursor, Config, ConfigRevision, ExpansionConfig, TemplateContext,
};

use crate::{
    colorpack::ColorPack,
    lang::{Language, Strings},
    theme::{self, Palette},
};

/// Bound the authoritative focus-return wait. We never insert into an
/// unverified window after this deadline.
const FOCUS_RETURN_TIMEOUT: Duration = Duration::from_secs(2);
const FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_RESULTS: usize = 60;

/// Rank `entry` against `query` (both compared case-insensitively). A
/// trigger match beats a description match, an earlier match beats a later
/// one, and letters that merely appear in order ("sg" for ";sig") still
/// match, below any substring. `None` means the entry is hidden.
pub(crate) fn score(query: &str, trigger: &str, description: &str) -> Option<i64> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let trigger = trigger.to_lowercase();
    let description = description.to_lowercase();
    score_normalized(&query, &trigger, &description)
}

fn score_normalized(query: &str, trigger: &str, description: &str) -> Option<i64> {
    if let Some(position) = trigger.find(&query) {
        return Some(3_000 - position as i64 + i64::from(trigger == query) * 1_000);
    }
    if let Some(position) = description.find(&query) {
        return Some(2_000 - position as i64);
    }
    let mut gaps = 0_i64;
    let mut last = None;
    let mut characters = trigger
        .chars()
        .chain(std::iter::once(' '))
        .chain(description.chars());
    let mut character_index = 0usize;
    for wanted in query.chars().filter(|character| !character.is_whitespace()) {
        let mut found = None;
        for character in characters.by_ref() {
            let index = character_index;
            character_index += 1;
            if character == wanted {
                found = Some(index);
                break;
            }
        }
        let index = found?;
        if let Some(previous) = last {
            gaps += (index - previous - 1) as i64;
        }
        last = Some(index);
    }
    Some(1_000 - gaps.min(999))
}

#[derive(Debug, Clone)]
struct SearchEntry {
    trigger: String,
    description: String,
}

/// What the user chose, handed back to `main` after the window closes.
#[derive(Default)]
pub(crate) struct Outcome {
    pub trigger: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FocusTarget {
    generation: u64,
    token: String,
}

pub(crate) struct PickerApp {
    config: Config,
    config_revision: ConfigRevision,
    search_index: Vec<SearchEntry>,
    cached_query: Option<String>,
    cached_revision: Option<ConfigRevision>,
    result_indices: Vec<usize>,
    strings: Strings,
    palette: Palette,
    query: String,
    selected: usize,
    daemon_available: bool,
    copied: Option<String>,
    focus_requested: bool,
    outcome: std::sync::Arc<std::sync::Mutex<Outcome>>,
}

impl PickerApp {
    pub(crate) fn new(
        config: Config,
        config_revision: ConfigRevision,
        language: Language,
        colorpack: ColorPack,
        dark: bool,
        daemon_available: bool,
        outcome: std::sync::Arc<std::sync::Mutex<Outcome>>,
    ) -> Self {
        Self {
            search_index: config
                .expansion
                .iter()
                .map(|expansion| SearchEntry {
                    trigger: expansion.trigger.to_lowercase(),
                    description: expansion.description.to_lowercase(),
                })
                .collect(),
            config,
            config_revision,
            cached_query: None,
            cached_revision: None,
            result_indices: Vec::new(),
            strings: Strings::new(language),
            palette: Palette::for_pack(colorpack, dark),
            query: String::new(),
            selected: 0,
            daemon_available,
            copied: None,
            focus_requested: false,
            outcome,
        }
    }

    /// Cache enabled plain-text snippet indices, best match first. Command
    /// snippets are left out: they only run when their trigger is typed.
    fn refresh_results(&mut self) {
        if self.cached_query.as_deref() == Some(self.query.as_str())
            && self.cached_revision.as_ref() == Some(&self.config_revision)
        {
            return;
        }
        let query = self.query.trim().to_lowercase();
        let mut ranked: Vec<(i64, usize)> = self
            .config
            .expansion
            .iter()
            .enumerate()
            .filter(|(_, expansion)| expansion.enabled && expansion.command.is_none())
            .filter_map(|(index, _)| {
                score_normalized(
                    &query,
                    &self.search_index[index].trigger,
                    &self.search_index[index].description,
                )
                .map(|score| (score, index))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.result_indices = ranked
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, index)| index)
            .collect();
        self.cached_query = Some(self.query.clone());
        self.cached_revision = Some(self.config_revision.clone());
    }

    fn choose(&mut self, ctx: &egui::Context, expansion: &ExpansionConfig) {
        if self.daemon_available {
            if let Ok(mut outcome) = self.outcome.lock() {
                outcome.trigger = Some(expansion.trigger.clone());
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // No daemon to type for us: offer the text on the clipboard. The
        // window stays open because a Wayland clipboard is served by the
        // process that set it.
        match render_template_with_cursor(&expansion.replacement, &TemplateContext::system()) {
            Ok((text, _)) => {
                ctx.copy_text(text);
                self.copied = Some(expansion.trigger.clone());
            }
            Err(_) => self.copied = None,
        }
    }
}

impl eframe::App for PickerApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let palette = self.palette;
        self.refresh_results();
        let result_indices = self.result_indices.clone();
        let count = result_indices.len();
        let (up, down, enter, escape) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if escape {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if down && count > 0 {
            self.selected = (self.selected + 1).min(count - 1);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        self.selected = self.selected.min(count.saturating_sub(1));
        let mut chosen = (enter && count > 0).then(|| result_indices[self.selected]);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.background)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(root, |ui| {
                let search_id = egui::Id::new("picker_search");
                let response = ui.add(
                    TextEdit::singleline(&mut self.query)
                        .id(search_id)
                        .hint_text(self.strings.picker_hint())
                        .font(egui::TextStyle::Heading)
                        .margin(egui::Margin::symmetric(12, 10))
                        .desired_width(f32::INFINITY),
                );
                if response.changed() {
                    self.selected = 0;
                    self.copied = None;
                }
                if !self.focus_requested || !response.has_focus() {
                    response.request_focus();
                    self.focus_requested = true;
                }
                ui.add_space(10.0);

                let footer_height = 30.0;
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height() - footer_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if result_indices.is_empty() {
                            ui.add_space(24.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(self.strings.picker_no_results())
                                        .color(palette.muted),
                                );
                            });
                        }
                        for (index, expansion_index) in result_indices.iter().copied().enumerate() {
                            let expansion = &self.config.expansion[expansion_index];
                            let selected = index == self.selected;
                            let response = picker_row(ui, &palette, expansion, selected);
                            if selected && (up || down) {
                                response.scroll_to_me(None);
                            }
                            if response.hovered() && ui.input(|input| input.pointer.is_moving()) {
                                self.selected = index;
                            }
                            if response.clicked() {
                                chosen = Some(expansion_index);
                            }
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    let (text, color) = match (&self.copied, self.daemon_available) {
                        (Some(trigger), _) => {
                            (self.strings.picker_copied(trigger), palette.success)
                        }
                        (None, true) => (
                            self.strings.picker_footer_daemon().to_owned(),
                            palette.muted,
                        ),
                        (None, false) => (
                            self.strings.picker_footer_clipboard().to_owned(),
                            palette.warning,
                        ),
                    };
                    ui.label(RichText::new(text).small().color(color));
                });
            });
        if let Some(expansion_index) = chosen {
            let expansion = self.config.expansion[expansion_index].clone();
            self.choose(&ctx, &expansion);
        }
    }
}

fn picker_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    expansion: &ExpansionConfig,
    selected: bool,
) -> egui::Response {
    // Show what will be typed, not template syntax: `{{date}}` becomes the
    // date and the `{{cursor}}` marker disappears.
    let rendered = render_template_with_cursor(&expansion.replacement, &TemplateContext::system())
        .map(|(text, _)| text)
        .unwrap_or_else(|_| expansion.replacement.clone());
    let first_line = rendered
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(90)
        .collect::<String>();
    let fill = if selected {
        palette.accent_weak
    } else {
        Color32::TRANSPARENT
    };
    let frame = egui::Frame::new()
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&expansion.trigger)
                        .monospace()
                        .strong()
                        .color(palette.accent),
                );
                if !expansion.description.is_empty() {
                    ui.label(&expansion.description);
                }
                if !expansion.category.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::pill(
                            ui,
                            &expansion.category,
                            palette.accent,
                            theme::tint(palette.accent, 30),
                        );
                    });
                }
            });
            ui.add(
                egui::Label::new(RichText::new(first_line).small().color(palette.muted)).truncate(),
            );
        });
    let response = ui.interact(
        frame.response.rect,
        ui.id().with(("picker_row", &expansion.trigger)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            selected,
            expansion.trigger.as_str(),
        )
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Run the picker, then hand the chosen trigger to the daemon.
pub(crate) fn run(path: PathBuf) -> anyhow::Result<()> {
    let loaded = Config::load_versioned(&path)
        .map_err(|error| anyhow::anyhow!("configuration invalid: {}", error.safe_summary()))?;
    let config = loaded.config;
    let config_revision = loaded.revision;
    let prefs = crate::settings::load_gui_prefs();
    let target_focus = crate::runtime::control_command("focus")
        .ok()
        .and_then(|response| focus_target_from_response(&response));
    let daemon_available = target_focus.is_some();
    let outcome = std::sync::Arc::new(std::sync::Mutex::new(Outcome::default()));
    let app_outcome = std::sync::Arc::clone(&outcome);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WayExpand — Insert snippet")
            .with_app_id("io.github.cyberducttape.WayExpand.Picker")
            .with_inner_size([640.0, 440.0])
            .with_min_inner_size([420.0, 260.0]),
        ..Default::default()
    };
    eframe::run_native(
        "WayExpand picker",
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
            Ok(Box::new(PickerApp::new(
                config,
                config_revision,
                prefs.language,
                prefs.colorpack,
                dark,
                daemon_available,
                app_outcome,
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!("picker failed: {error}"))?;

    let trigger = outcome
        .lock()
        .ok()
        .and_then(|mut outcome| outcome.trigger.take());
    if let Some(trigger) = trigger {
        let Some(target_focus) = target_focus else {
            anyhow::bail!("could not identify the original focused window; copied instead");
        };
        let deadline = Instant::now() + FOCUS_RETURN_TIMEOUT;
        loop {
            let current = crate::runtime::control_command("focus")
                .ok()
                .and_then(|response| focus_target_from_response(&response));
            if let Some(current) = current {
                if current.token == target_focus.token
                    && current.generation > target_focus.generation
                {
                    let response = crate::runtime::control_command(&format!(
                        "insert-target {} {} {trigger}",
                        current.generation, current.token
                    ))?;
                    if response.trim_end() != "insert scheduled" {
                        anyhow::bail!("daemon refused the insert: {}", response.trim_end());
                    }
                    break;
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("focus did not return to the original window; refusing to insert");
            }
            thread::sleep(FOCUS_POLL_INTERVAL);
        }
    }
    Ok(())
}

fn focus_target_from_response(response: &str) -> Option<FocusTarget> {
    let generation = response
        .lines()
        .find_map(|line| line.strip_prefix("focus_generation="))?
        .parse()
        .ok()?;
    let token = response
        .lines()
        .find_map(|line| line.strip_prefix("focus_token="))?;
    if token.is_empty() {
        return None;
    }
    let token = token.to_owned();
    Some(FocusTarget { generation, token })
}

#[cfg(test)]
mod tests {
    use super::{focus_target_from_response, score, FocusTarget};

    #[test]
    fn trigger_matches_outrank_description_and_subsequence_matches() {
        let exact = score(";sig", ";sig", "Email signature").unwrap();
        let prefix = score("sig", ";sig", "Email signature").unwrap();
        let described = score("email", ";s", "Email signature").unwrap();
        let scattered = score("sg", ";sig", "").unwrap();
        assert!(exact > prefix, "{exact} {prefix}");
        assert!(prefix > described, "{prefix} {described}");
        assert!(described > scattered, "{described} {scattered}");
        assert_eq!(score("zz", ";sig", "Email signature"), None);
        assert_eq!(score("  ", ";sig", ""), Some(0));
        assert!(score("SIG", ";sig", "").is_some(), "matching ignores case");
    }

    #[test]
    fn focus_target_parser_fails_closed_for_missing_or_empty_tokens() {
        assert_eq!(
            focus_target_from_response("focus_generation=4\nfocus_token=abcd\n"),
            Some(FocusTarget {
                generation: 4,
                token: "abcd".into()
            })
        );
        assert_eq!(
            focus_target_from_response("focus_generation=4\nfocus_token=\n"),
            None
        );
        assert_eq!(focus_target_from_response("running\n"), None);
    }
}
