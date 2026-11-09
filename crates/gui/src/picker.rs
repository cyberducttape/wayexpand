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
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, RichText, TextEdit};
use wayexpand_core::{
    form_fields, render_template_preview, render_template_with_cursor, template_variables, Config,
    ConfigRevision, ExpansionConfig, TemplateContext,
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
const PREVIEW_CONTEXT_REFRESH: Duration = Duration::from_secs(1);
const MAX_RESULTS: usize = 60;

/// Rank `entry` against `query` (both compared case-insensitively). A
/// trigger match beats a description match, an earlier match beats a later
/// one, and letters that merely appear in order ("sg" for ";sig") still
/// match, below any substring. `None` means the entry is hidden.
#[cfg(test)]
fn score(query: &str, trigger: &str, description: &str) -> Option<i64> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let trigger = trigger.to_lowercase();
    let description = description.to_lowercase();
    score_normalized(&query, &trigger, &description)
}

fn score_normalized(query: &str, trigger: &str, description: &str) -> Option<i64> {
    if let Some(position) = trigger.find(query) {
        return Some(3_000 - position as i64 + i64::from(trigger == query) * 1_000);
    }
    if let Some(position) = description.find(query) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FocusTarget {
    generation: u64,
    token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FocusInfo {
    target: Option<FocusTarget>,
    atomic_replace_available: bool,
}

pub(crate) struct PickerApp {
    config: Config,
    /// Built once per configuration: includes and allowlisted environment
    /// variables resolve the same way the daemon resolves them.
    template_context: TemplateContext,
    config_revision: ConfigRevision,
    search_index: Vec<SearchEntry>,
    cached_query: Option<String>,
    cached_revision: Option<ConfigRevision>,
    preview_cache: HashMap<String, String>,
    last_preview_context_refresh: Instant,
    result_indices: Vec<usize>,
    strings: Strings,
    palette: Palette,
    query: String,
    selected: usize,
    safe_direct_insert: bool,
    clipboard_fallback_allowed: bool,
    copied: Option<String>,
    focus_requested: bool,
    target_focus: Option<FocusTarget>,
    insert_state: InsertState,
    /// Debug builds only: capture the window and exit (see `screenshot`).
    screenshot: Option<crate::screenshot::ScreenshotCapture>,
}

enum InsertState {
    Ready,
    Waiting {
        trigger: String,
        text: String,
        receiver: Receiver<Result<(), String>>,
    },
    Failed {
        trigger: String,
        text: String,
        error: String,
    },
}

impl PickerApp {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: Config,
        config_revision: ConfigRevision,
        language: Language,
        colorpack: ColorPack,
        dark: bool,
        safe_direct_insert: bool,
        clipboard_fallback_allowed: bool,
        target_focus: Option<FocusTarget>,
    ) -> Self {
        Self {
            search_index: config
                .expansion
                .iter()
                .map(|expansion| SearchEntry {
                    trigger: crate::library::trigger_search_text(expansion),
                    description: expansion.description.to_lowercase(),
                })
                .collect(),
            template_context: config.template_context(None),
            config,
            config_revision,
            cached_query: None,
            cached_revision: None,
            preview_cache: HashMap::new(),
            last_preview_context_refresh: Instant::now(),
            result_indices: Vec::new(),
            strings: Strings::new(language),
            palette: Palette::for_pack(colorpack, dark),
            query: String::new(),
            selected: 0,
            safe_direct_insert,
            clipboard_fallback_allowed,
            copied: None,
            focus_requested: false,
            target_focus,
            screenshot: None,
            insert_state: InsertState::Ready,
        }
    }

    /// Cache enabled plain-text snippet indices, best match first. Command
    /// snippets are left out: they only run when their trigger is typed.
    fn refresh_results(&mut self) {
        let query = self.query.trim().to_lowercase();
        if self.cached_query.as_deref() == Some(query.as_str())
            && self.cached_revision.as_ref() == Some(&self.config_revision)
        {
            return;
        }
        if self.cached_revision.as_ref() != Some(&self.config_revision) {
            self.preview_cache.clear();
        }
        let ranked: Vec<(i64, usize)> = self
            .config
            .expansion
            .iter()
            .enumerate()
            .filter(|(_, expansion)| {
                picker_expansion_is_eligible(expansion, self.safe_direct_insert)
            })
            .filter_map(|(index, _)| {
                score_normalized(
                    &query,
                    &self.search_index[index].trigger,
                    &self.search_index[index].description,
                )
                .map(|score| (score, index))
            })
            .collect();
        self.result_indices = top_ranked(ranked, MAX_RESULTS);
        self.cached_query = Some(query);
        self.cached_revision = Some(self.config_revision.clone());
    }

    fn choose(&mut self, ctx: &egui::Context, expansion: &ExpansionConfig) {
        if self.safe_direct_insert {
            let Some(target_focus) = self.target_focus.clone() else {
                self.insert_state = InsertState::Failed {
                    trigger: expansion.trigger.clone(),
                    text: String::new(),
                    error: "Could not identify the original focused window; refusing to insert."
                        .to_owned(),
                };
                return;
            };
            let (sender, receiver) = mpsc::channel();
            let trigger = expansion.trigger.clone();
            let worker_trigger = trigger.clone();
            let spawned = thread::Builder::new()
                .name("wayexpand-picker-insert".into())
                .spawn(move || {
                    let result = wait_for_focus_and_insert(target_focus, &worker_trigger);
                    let _ = sender.send(result);
                });
            if let Err(error) = spawned {
                self.insert_state = InsertState::Failed {
                    trigger,
                    text: String::new(),
                    error: format!("Could not start insertion worker: {error}"),
                };
                return;
            }
            self.insert_state = InsertState::Waiting {
                trigger,
                // The daemon owns the execution context, including the
                // current clipboard and dynamic values. Do not render a
                // second, incomplete copy in the picker before delegating.
                text: String::new(),
                receiver,
            };
            return;
        }
        // Preview context is intentionally separate from execution context.
        // Refresh this at selection time so date/time values do not reflect
        // when the picker happened to open. Clipboard fallback has no daemon
        // reader, so it renders with the values available to this process.
        let context = self.config.template_context(None);
        let text = match render_template_with_cursor(&expansion.replacement, &context) {
            Ok((text, _)) => text,
            Err(error) => {
                self.insert_state = InsertState::Failed {
                    trigger: expansion.trigger.clone(),
                    text: String::new(),
                    error: format!("Could not render this snippet: {error}"),
                };
                return;
            }
        };
        // No daemon to type for us: offer the text on the clipboard. The
        // window stays open because a Wayland clipboard is served by the
        // process that set it.
        self.copied = Some(copy_snippet(ctx, text, &expansion.trigger));
    }

    fn preview_for(&mut self, expansion: &ExpansionConfig) -> String {
        if self.last_preview_context_refresh.elapsed() >= PREVIEW_CONTEXT_REFRESH {
            let context = self.config.template_context(None);
            if context.unix_timestamp != self.template_context.unix_timestamp {
                self.template_context = context;
                self.preview_cache.clear();
            }
            self.last_preview_context_refresh = Instant::now();
        }
        if let Some(preview) = self.preview_cache.get(&expansion.id) {
            return preview.clone();
        }
        let rendered = render_template_preview(&expansion.replacement, &self.template_context, 512)
            .unwrap_or_else(|_| "(preview unavailable)".to_owned());
        let preview = rendered
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(90)
            .collect::<String>();
        self.preview_cache
            .insert(expansion.id.clone(), preview.clone());
        preview
    }

    fn poll_insert(&mut self, ctx: &egui::Context) {
        let state = std::mem::replace(&mut self.insert_state, InsertState::Ready);
        let InsertState::Waiting {
            trigger,
            text,
            receiver,
        } = state
        else {
            self.insert_state = state;
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(())) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Ok(Err(error)) => {
                self.insert_state = InsertState::Failed {
                    trigger,
                    text,
                    error,
                };
            }
            Err(TryRecvError::Empty) => {
                self.insert_state = InsertState::Waiting {
                    trigger,
                    text,
                    receiver,
                };
                ctx.request_repaint_after(FOCUS_POLL_INTERVAL);
            }
            Err(TryRecvError::Disconnected) => {
                self.insert_state = InsertState::Failed {
                    trigger,
                    text,
                    error: "The insertion worker stopped before the insert was confirmed."
                        .to_owned(),
                };
            }
        }
    }
}

impl eframe::App for PickerApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let palette = self.palette;
        if let Some(capture) = self.screenshot.as_mut() {
            // `search` stages a query so the capture shows ranked results.
            if capture.take_view().as_deref() == Some("search") {
                self.query = "c".into();
            }
            capture.tick(&ctx);
        }
        self.poll_insert(&ctx);
        self.refresh_results();
        let result_indices = self.result_indices.clone();
        let count = result_indices.len();
        let ready = matches!(self.insert_state, InsertState::Ready);
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
        let mut chosen = (ready && enter && count > 0).then(|| result_indices[self.selected]);

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
                                if !self.safe_direct_insert
                                    && self.config.expansion.iter().any(|expansion| {
                                        expansion.enabled && !expansion.app_filter.is_empty()
                                    })
                {
                    ui.label(
                                        RichText::new("App-restricted snippets are hidden in copy-only mode because manual paste cannot verify the destination application.")
                            .small()
                            .color(palette.muted),
                    );
                }

                match &self.insert_state {
                    InsertState::Waiting { trigger, .. } => {
                        theme::card(ui, &palette, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(self.strings.picker_inserting(trigger)).strong(),
                            );
                            ui.label(self.strings.picker_waiting_for_focus());
                        });
                        ui.add_space(10.0);
                    }
                    InsertState::Failed {
                        trigger,
                        error,
                        text,
                    } => {
                        theme::card(ui, &palette, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(self.strings.picker_insert_failed())
                                    .strong()
                                    .color(palette.warning),
                            );
                            ui.label(error);
                            ui.horizontal(|ui| {
                                if self.clipboard_fallback_allowed
                                    && !text.is_empty()
                                    && ui.button(self.strings.picker_copy_instead()).clicked()
                                {
                                    self.copied = Some(copy_snippet(&ctx, text.clone(), trigger));
                                }
                                if ui.button(self.strings.picker_cancel()).clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                        });
                        ui.add_space(10.0);
                    }
                    InsertState::Ready => {}
                }

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
                            let expansion = self.config.expansion[expansion_index].clone();
                            let preview = self.preview_for(&expansion);
                            let selected = index == self.selected;
                            let response = picker_row(ui, &palette, &preview, &expansion, selected);
                            if selected && (up || down) {
                                response.scroll_to_me(None);
                            }
                            if response.hovered() && ui.input(|input| input.pointer.is_moving()) {
                                self.selected = index;
                            }
                            if ready && response.clicked() {
                                chosen = Some(expansion_index);
                            }
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    let (text, color) = match (&self.copied, self.safe_direct_insert) {
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
    preview: &str,
    expansion: &ExpansionConfig,
    selected: bool,
) -> egui::Response {
    // Show what will be typed, not template syntax: `{{date}}` becomes the
    // date and the `{{cursor}}` marker disappears.
    let first_line = preview;
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
                        // Blend against the row's own fill: `pill` otherwise
                        // assumes the panel color and draws a dark box on
                        // the highlighted row.
                        let row = theme::blend_over(fill, palette.background);
                        theme::pill(
                            ui,
                            &expansion.category,
                            palette.accent,
                            theme::blend_over(theme::tint(palette.accent, 56), row),
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
            egui::WidgetType::Button,
            true,
            selected,
            expansion.trigger.as_str(),
        )
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn wait_for_focus_and_insert(target_focus: FocusTarget, trigger: &str) -> Result<(), String> {
    let deadline = Instant::now() + FOCUS_RETURN_TIMEOUT;
    loop {
        let current = crate::runtime::control_command(wayexpand_core::DaemonOperation::Focus)
            .ok()
            .and_then(|response| focus_target_from_response(&response));
        if let Some(current) = current.and_then(|info| info.target) {
            if focus_returned_to_target(&target_focus, &current) {
                let response =
                    crate::runtime::insert_target(current.generation, &current.token, trigger)
                        .map_err(|error| error.to_string())?;
                if response.trim_end() != "insert scheduled" {
                    return Err(format!(
                        "daemon refused the insert: {}",
                        response.trim_end()
                    ));
                }
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err(
                "Focus did not return to the original window; refusing to insert.".to_owned(),
            );
        }
        thread::sleep(FOCUS_POLL_INTERVAL);
    }
}

fn focus_returned_to_target(target: &FocusTarget, current: &FocusTarget) -> bool {
    current.token == target.token && current.generation > target.generation
}

/// Run the picker. Insertion remains inside the picker until the guarded
/// focus handoff succeeds, so a safe failure remains visible to the user.
pub(crate) fn run(path: PathBuf, copy_only: bool) -> anyhow::Result<()> {
    let policy = wayexpand_core::load_organization_policy()
        .map_err(|error| anyhow::anyhow!("could not load organization policy: {error}"))?;
    let clipboard_fallback_allowed = clipboard_fallback_allowed_by_policy(&policy);
    let loaded = Config::load_versioned(&path)
        .map_err(|error| anyhow::anyhow!("configuration invalid: {}", error.safe_summary()))?;
    let config = loaded.config;
    let config_revision = loaded.revision;
    let prefs = crate::settings::load_gui_prefs();
    let focus_info = if !picker_needs_daemon_focus(copy_only) {
        None
    } else {
        crate::runtime::control_command(wayexpand_core::DaemonOperation::Focus)
            .ok()
            .and_then(|response| focus_target_from_response(&response))
    };
    let (target_focus, safe_direct_insert) = picker_focus_state(focus_info, copy_only);
    if !safe_direct_insert && !clipboard_fallback_allowed {
        anyhow::bail!(
            "Quick Picker cannot copy snippets under organization policy requiring a verified insertion target"
        );
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WayExpand — Insert snippet")
            .with_app_id("io.github.cyberducttape.WayExpand.Picker")
            .with_inner_size([640.0, 440.0])
            .with_min_inner_size([420.0, 260.0])
            .with_icon(crate::app_icon()),
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
            let mut app = PickerApp::new(
                config,
                config_revision,
                prefs.language,
                prefs.colorpack,
                dark,
                safe_direct_insert,
                clipboard_fallback_allowed,
                target_focus,
            );
            app.screenshot = crate::screenshot::ScreenshotCapture::from_env();
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("picker failed: {error}"))?;
    Ok(())
}

fn picker_focus_state(
    focus_info: Option<FocusInfo>,
    copy_only: bool,
) -> (Option<FocusTarget>, bool) {
    if copy_only {
        return (None, false);
    }
    focus_info
        .map(|info| {
            // A target-verification signal alone does not prevent concurrent
            // input from interleaving between erase and insert. Keep picker
            // output on the clipboard/manual-paste path until replacement is
            // atomic or a non-atomic route is certified end to end.
            let safe_direct_insert = info.atomic_replace_available && info.target.is_some();
            (info.target, safe_direct_insert)
        })
        // A missing or malformed focus response proves neither daemon
        // availability nor exact window identity. Keep the picker honest in
        // clipboard-only mode instead of presenting a false guarantee.
        .unwrap_or((None, false))
}

fn picker_needs_daemon_focus(copy_only: bool) -> bool {
    !copy_only
}

fn copy_snippet(ctx: &egui::Context, text: String, trigger: &str) -> String {
    // Keep the picker alive after this command: Wayland clipboard ownership
    // belongs to this process, so closing it here can invalidate the paste.
    ctx.copy_text(text);
    trigger.to_owned()
}

fn clipboard_fallback_allowed_by_policy(policy: &wayexpand_core::OrganizationPolicy) -> bool {
    !policy
        .effective_enforcement_policy()
        .require_verified_insert_target
}

fn picker_expansion_is_eligible(expansion: &ExpansionConfig, safe_direct_insert: bool) -> bool {
    expansion.enabled
        && expansion.command.is_none()
        && form_fields(&expansion.replacement).is_ok_and(|fields| fields.is_empty())
        && (safe_direct_insert || expansion.app_filter.is_empty())
        && (safe_direct_insert
            || !template_variables(&expansion.replacement)
                .iter()
                .any(|variable| matches!(*variable, "clipboard" | "secret")))
}

fn focus_target_from_response(response: &str) -> Option<FocusInfo> {
    let focus = wayexpand_core::DaemonFocus::parse(response)?;
    Some(FocusInfo {
        target: focus
            .verified_target()
            .map(|(generation, token)| FocusTarget {
                generation,
                token: token.to_owned(),
            }),
        atomic_replace_available: focus.atomic_replace,
    })
}

/// Indices of the `limit` best `(score, index)` pairs: best score first,
/// then library order. Only the kept results are fully sorted, so a large
/// library pays O(n) per query change instead of O(n log n).
fn top_ranked(mut ranked: Vec<(i64, usize)>, limit: usize) -> Vec<usize> {
    let order = |a: &(i64, usize), b: &(i64, usize)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
    if limit == 0 {
        return Vec::new();
    }
    if ranked.len() > limit {
        ranked.select_nth_unstable_by(limit - 1, order);
        ranked.truncate(limit);
    }
    ranked.sort_unstable_by(order);
    ranked.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        clipboard_fallback_allowed_by_policy, copy_snippet, focus_returned_to_target,
        focus_target_from_response, picker_expansion_is_eligible, picker_focus_state,
        picker_needs_daemon_focus, score, top_ranked, FocusInfo, FocusTarget, MAX_RESULTS,
    };

    #[test]
    fn top_ranked_matches_a_full_sort() {
        // Many ties exercise the library-order tie-break across the cut.
        let ranked: Vec<(i64, usize)> = (0..500)
            .map(|index| ((index * 37 % 11) as i64, index))
            .collect();
        let mut expected = ranked.clone();
        expected.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for limit in [0, 1, 60, 499, 500, 600] {
            let want: Vec<usize> = expected
                .iter()
                .take(limit)
                .map(|&(_, index)| index)
                .collect();
            assert_eq!(top_ranked(ranked.clone(), limit), want, "limit {limit}");
        }
    }
    use wayexpand_core::{ExpansionConfig, MatchMode};

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
            focus_target_from_response(
                "focus_generation=4\nfocus_token=abcd\nfocus_identity=exact\natomic_replace=true\nverified_replace=true\n",
            ),
            Some(FocusInfo {
                target: Some(FocusTarget {
                    generation: 4,
                    token: "abcd".into(),
                }),
                atomic_replace_available: true,
            })
        );
        assert_eq!(
            focus_target_from_response(
                "focus_generation=4\nfocus_token=\nfocus_identity=unavailable\natomic_replace=false\nverified_replace=false\n",
            ),
            Some(FocusInfo {
                target: None,
                atomic_replace_available: false,
            })
        );
        assert_eq!(focus_target_from_response("running\n"), None);
    }

    #[test]
    fn missing_focus_response_does_not_claim_exact_identity() {
        assert_eq!(picker_focus_state(None, false), (None, false));
    }

    #[test]
    fn picker_uses_direct_insertion_only_for_atomic_exact_targets() {
        let atomic = focus_target_from_response(
            "focus_generation=4\nfocus_token=abcd\nfocus_identity=exact\natomic_replace=true\nverified_replace=true\n",
        )
        .unwrap();
        assert!(picker_focus_state(Some(atomic), false).1);

        let focus = focus_target_from_response(
            "focus_generation=4\nfocus_token=abcd\nfocus_identity=exact\natomic_replace=false\nverified_replace=true\n",
        )
        .unwrap();
        assert_eq!(
            picker_focus_state(Some(focus), false),
            (
                Some(FocusTarget {
                    generation: 4,
                    token: "abcd".into(),
                }),
                false
            )
        );

        let unverified = focus_target_from_response(
            "focus_generation=4\nfocus_token=abcd\nfocus_identity=exact\natomic_replace=false\nverified_replace=false\n",
        )
        .unwrap();
        assert!(!picker_focus_state(Some(unverified), false).1);
    }

    #[test]
    fn picker_handoff_rejects_a_different_window_with_the_same_app_and_title() {
        // The app/title metadata is intentionally identical; the focus token
        // represents the backend's stronger per-window identity.
        let target = FocusTarget {
            generation: 4,
            token: "window-a".into(),
        };
        let same_app_and_title_other_window = FocusTarget {
            generation: 5,
            token: "window-b".into(),
        };
        let original_window_regained = FocusTarget {
            generation: 6,
            token: "window-a".into(),
        };

        assert!(!focus_returned_to_target(
            &target,
            &same_app_and_title_other_window
        ));
        assert!(focus_returned_to_target(&target, &original_window_regained));
    }

    #[test]
    fn clipboard_snippets_are_hidden_without_a_daemon_reader() {
        let expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":clip".into(),
            replacement: "{{clipboard}}".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
            output_file: None,
        };
        assert!(!picker_expansion_is_eligible(&expansion, false));
        assert!(picker_expansion_is_eligible(&expansion, true));
    }

    #[test]
    fn copy_only_picker_never_requests_direct_insertion() {
        let (target, direct_insert) = picker_focus_state(
            Some(FocusInfo {
                target: Some(FocusTarget {
                    generation: 7,
                    token: "opaque-test-token".into(),
                }),
                atomic_replace_available: true,
            }),
            true,
        );
        assert!(target.is_none());
        assert!(!direct_insert);
    }

    #[test]
    fn copy_only_picker_does_not_require_a_running_daemon() {
        assert!(!picker_needs_daemon_focus(true));
        assert!(picker_needs_daemon_focus(false));
        assert_eq!(picker_focus_state(None, true), (None, false));
    }

    #[test]
    fn copy_action_transfers_the_rendered_text_to_the_clipboard_and_keeps_trigger_state() {
        let context = eframe::egui::Context::default();
        let output = context.run_ui(Default::default(), |ui| {
            assert_eq!(
                copy_snippet(ui.ctx(), "safe command".into(), ":check"),
                ":check"
            );
        });
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, eframe::egui::OutputCommand::CopyText(text) if text == "safe command")
        }));
    }

    #[test]
    fn empty_library_produces_an_empty_search_result() {
        assert!(top_ranked(Vec::new(), MAX_RESULTS).is_empty());
    }

    #[test]
    fn copy_only_mode_hides_clipboard_and_secret_templates() {
        for variable in ["clipboard", "secret"] {
            let expansion = ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: format!(":{variable}"),
                replacement: format!("{{{{{variable}}}}}"),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
                aliases: Vec::new(),
                output_file: None,
            };
            assert!(!picker_expansion_is_eligible(&expansion, false));
        }
    }

    #[test]
    fn picker_hides_app_scoped_snippets_from_manual_copy_paths() {
        let expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":production".into(),
            replacement: "restricted operational text".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: vec!["app_id_exact:org.example.Terminal".into()],
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
            output_file: None,
        };
        assert!(!picker_expansion_is_eligible(&expansion, false));
        assert!(picker_expansion_is_eligible(&expansion, true));
    }

    #[test]
    fn copy_only_mode_respects_verified_target_organization_policy() {
        let enforcing = wayexpand_core::OrganizationPolicy {
            safe_mode: true,
            require_verified_insert_target: true,
            ..wayexpand_core::OrganizationPolicy::default()
        };
        assert!(!clipboard_fallback_allowed_by_policy(&enforcing));

        let audit_only = wayexpand_core::OrganizationPolicy {
            safe_mode: false,
            require_verified_insert_target: true,
            ..wayexpand_core::OrganizationPolicy::default()
        };
        assert!(clipboard_fallback_allowed_by_policy(&audit_only));
    }

    #[test]
    fn form_snippets_are_hidden_until_picker_form_submission_exists() {
        let expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":form".into(),
            replacement: "Hello {{field:name}}".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
            output_file: None,
        };

        assert!(!picker_expansion_is_eligible(&expansion, false));
        assert!(!picker_expansion_is_eligible(&expansion, true));
    }
}
