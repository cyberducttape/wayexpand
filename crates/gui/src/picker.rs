//! Quick Picker: `wayexpand-gui --picker`.
//!
//! Search and choose a saved text snippet. The picker delegates insertion to
//! the daemon only when it can verify the original target and the route
//! supports atomic replacement; the daemon applies its normal policy and
//! sensitive-field checks. Otherwise, the picker offers copy-and-paste when
//! organization policy permits it. In that mode app-restricted snippets are
//! hidden because the destination application cannot be verified, and the
//! picker stays open to retain Wayland clipboard ownership until the user
//! pastes and closes it.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
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
/// How long the picker waits for the daemon to confirm a scheduled insert.
/// Simulated-key backends type long snippets slowly, so this is generous.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);
const DELIVERY_POLL_INTERVAL: Duration = Duration::from_millis(30);
const PREVIEW_CONTEXT_REFRESH: Duration = Duration::from_secs(1);
const MAX_RESULTS: usize = 60;
/// Sinks an unusable snippet below every usable match.
const UNAVAILABLE_PENALTY: i64 = 1_000_000;

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
    app_scope: String,
    availability: Availability,
}

/// Whether the picker can insert or copy a snippet, and if not, why. A
/// snippet the picker cannot use is listed with its reason rather than
/// silently missing; only snippets the user turned off are left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Availability {
    Available,
    /// Disabled, or generate-only: turned off on purpose.
    Hidden,
    /// Command-backed snippets run only when their trigger is typed.
    RunsCommand,
    /// Form fields need input the picker cannot ask for yet.
    NeedsInput,
    /// Copy-only mode cannot check which app receives the paste.
    AppRestricted,
    /// Copy-only mode would put clipboard or secret content on the clipboard.
    ReadsClipboard,
}

fn picker_availability(expansion: &ExpansionConfig, safe_direct_insert: bool) -> Availability {
    if !expansion.is_keyboard_matchable() {
        Availability::Hidden
    } else if expansion.command.is_some() {
        Availability::RunsCommand
    } else if !form_fields(&expansion.replacement).is_ok_and(|fields| fields.is_empty()) {
        Availability::NeedsInput
    } else if !safe_direct_insert && !expansion.app_filter.is_empty() {
        Availability::AppRestricted
    } else if !safe_direct_insert
        && template_variables(&expansion.replacement)
            .iter()
            .any(|variable| matches!(*variable, "clipboard" | "secret"))
    {
        Availability::ReadsClipboard
    } else {
        Availability::Available
    }
}

impl SearchEntry {
    fn from_expansion(expansion: &ExpansionConfig, safe_direct_insert: bool) -> Self {
        Self {
            trigger: crate::library::trigger_search_text(expansion),
            description: expansion.description.to_lowercase(),
            app_scope: format_app_scope(&expansion.app_filter),
            // Eligibility does not change while the picker is open. Parse
            // forms and template variables once here instead of repeating
            // that work for every search edit.
            availability: picker_availability(expansion, safe_direct_insert),
        }
    }

    fn eligible(&self) -> bool {
        self.availability == Availability::Available
    }
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
    ranked_scratch: Vec<(i64, usize)>,
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
        receiver: Receiver<InsertOutcome>,
        handoff: InsertHandoff,
        /// The user asked to cancel after the daemon already had the insert.
        cancel_refused: bool,
    },
    Failed {
        trigger: String,
        text: String,
        error: String,
    },
}

/// The parts of `InsertState` the picker shows, copied for one frame.
enum ShownInsertState {
    Waiting {
        trigger: String,
        cancel_refused: bool,
    },
    Failed {
        trigger: String,
        error: String,
        text: String,
    },
    Ready,
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
                .map(|expansion| SearchEntry::from_expansion(expansion, safe_direct_insert))
                .collect(),
            template_context: config.template_context(None),
            config,
            config_revision,
            cached_query: None,
            cached_revision: None,
            preview_cache: HashMap::new(),
            last_preview_context_refresh: Instant::now(),
            ranked_scratch: Vec::new(),
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

    /// Cache matching snippet indices, best match first. Snippets the picker
    /// cannot use rank after every usable one and show why; snippets the user
    /// turned off are left out.
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
        self.ranked_scratch.clear();
        for (index, entry) in self.search_index.iter().enumerate() {
            if entry.availability == Availability::Hidden {
                continue;
            }
            if let Some(score) = score_normalized(&query, &entry.trigger, &entry.description) {
                let score = if entry.eligible() {
                    score
                } else {
                    score - UNAVAILABLE_PENALTY
                };
                self.ranked_scratch.push((score, index));
            }
        }
        top_ranked_into(
            &mut self.ranked_scratch,
            MAX_RESULTS,
            &mut self.result_indices,
        );
        self.cached_query = Some(query);
        self.cached_revision = Some(self.config_revision.clone());
    }

    fn choose(&mut self, ctx: &egui::Context, expansion: &ExpansionConfig) {
        if self.safe_direct_insert {
            let Some(target_focus) = self.target_focus.clone() else {
                self.insert_state = InsertState::Failed {
                    trigger: expansion.trigger.clone(),
                    text: String::new(),
                    error: self.strings.picker_message(&PickerMessage::FocusUnknown),
                };
                return;
            };
            let (sender, receiver) = mpsc::channel();
            let trigger = expansion.trigger.clone();
            let worker_trigger = trigger.clone();
            let handoff = InsertHandoff::new();
            let worker_handoff = handoff.clone();
            let spawned = thread::Builder::new()
                .name("wayexpand-picker-insert".into())
                .spawn(move || {
                    let outcome = wait_for_focus_and_insert(
                        &mut ControlSocket,
                        &target_focus,
                        &worker_trigger,
                        &worker_handoff,
                        INSERT_TIMING,
                    );
                    let _ = sender.send(outcome);
                });
            if let Err(error) = spawned {
                self.insert_state = InsertState::Failed {
                    trigger,
                    text: String::new(),
                    error: self
                        .strings
                        .picker_message(&PickerMessage::WorkerStartFailed(error.to_string())),
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
                handoff,
                cancel_refused: false,
            };
            // Step aside so the compositor returns focus to the window the
            // picker was opened from; the worker inserts only once that
            // exact window has focus again.
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            return;
        }
        // Preview context is intentionally separate from execution context.
        // Refresh this at selection time so date/time values do not reflect
        // when the picker happened to open. Clipboard fallback has no daemon
        // reader, so it renders with the values available to this process.
        let text = match self.clipboard_text(expansion) {
            Ok(text) => text,
            Err(error) => {
                self.insert_state = InsertState::Failed {
                    trigger: expansion.trigger.clone(),
                    text: String::new(),
                    error,
                };
                return;
            }
        };
        // No daemon to type for us: offer the text on the clipboard. The
        // window stays open because a Wayland clipboard is served by the
        // process that set it.
        self.copied = Some(copy_snippet(ctx, text, &expansion.trigger));
    }

    /// Render a snippet for the clipboard with the values available to this
    /// process; the clipboard has no daemon to render it.
    fn clipboard_text(&self, expansion: &ExpansionConfig) -> Result<String, String> {
        let context = self.config.template_context(None);
        render_template_with_cursor(&expansion.replacement, &context)
            .map(|(text, _)| text)
            .map_err(|error| {
                self.strings
                    .picker_message(&PickerMessage::RenderFailed(error.to_string()))
            })
    }

    /// Why "Copy instead" is not offered for `trigger` after a failed
    /// insert, if it is not: organization policy, or a snippet copy-only mode
    /// would hide (app-restricted, or reading the clipboard or a secret).
    /// A failed insert must never widen what can reach the clipboard.
    fn fallback_copy_blocked(&self, trigger: &str) -> Option<String> {
        if !self.clipboard_fallback_allowed {
            return Some(
                self.strings
                    .picker_message(&PickerMessage::CopyNotAllowedByPolicy),
            );
        }
        let expansion = self
            .config
            .expansion
            .iter()
            .find(|expansion| expansion.trigger == trigger)?;
        (!picker_expansion_is_eligible(expansion, false)).then(|| {
            self.strings
                .picker_message(&PickerMessage::CopyNotAllowedForSnippet)
        })
    }

    /// The text "Copy instead" puts on the clipboard, after the same checks
    /// as [`Self::fallback_copy_blocked`].
    fn fallback_copy_text(&self, trigger: &str, text: &str) -> Result<String, String> {
        if let Some(reason) = self.fallback_copy_blocked(trigger) {
            return Err(reason);
        }
        let expansion = self
            .config
            .expansion
            .iter()
            .find(|expansion| expansion.trigger == trigger)
            .ok_or_else(|| self.strings.picker_message(&PickerMessage::SnippetGone))?;
        if text.is_empty() {
            self.clipboard_text(expansion)
        } else {
            Ok(text.to_owned())
        }
    }

    /// The cached one-line preview of the snippet at `index`. Takes an index
    /// rather than a snippet so the result list can borrow each snippet
    /// instead of cloning it on every repaint.
    fn preview_at(&mut self, index: usize) -> String {
        if self.last_preview_context_refresh.elapsed() >= PREVIEW_CONTEXT_REFRESH {
            let context = self.config.template_context(None);
            if context.unix_timestamp != self.template_context.unix_timestamp {
                self.template_context = context;
                self.preview_cache.clear();
            }
            self.last_preview_context_refresh = Instant::now();
        }
        let expansion = &self.config.expansion[index];
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
            handoff,
            cancel_refused,
        } = state
        else {
            self.insert_state = state;
            return;
        };
        let failed = |error: String| InsertState::Failed {
            trigger: trigger.clone(),
            text: text.clone(),
            error,
        };
        match receiver.try_recv() {
            Ok(InsertOutcome::OutputAcknowledged | InsertOutcome::Cancelled) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close)
            }
            Ok(InsertOutcome::Failed(message)) => {
                self.insert_state = failed(self.strings.picker_message(&message));
                bring_back(ctx);
            }
            Err(TryRecvError::Empty) => {
                self.insert_state = InsertState::Waiting {
                    trigger,
                    text,
                    receiver,
                    handoff,
                    cancel_refused,
                };
                ctx.request_repaint_after(FOCUS_POLL_INTERVAL);
            }
            Err(TryRecvError::Disconnected) => {
                self.insert_state =
                    failed(self.strings.picker_message(&PickerMessage::WorkerStopped));
                bring_back(ctx);
            }
        }
    }

    /// Escape, Cancel, or closing the window. A pending insert is cancelled
    /// if the daemon does not have it yet; otherwise the picker stays open
    /// and reports what really happened.
    /// Decide a close request while an insert may be pending, cancelling
    /// it when nothing has been sent yet.
    fn close_requested(&mut self) -> CloseDecision {
        if let InsertState::Waiting {
            handoff,
            cancel_refused,
            ..
        } = &mut self.insert_state
        {
            if !handoff.cancel() {
                *cancel_refused = true;
                return CloseDecision::KeepOpen;
            }
        }
        CloseDecision::Close
    }

    fn cancel(&mut self, ctx: &egui::Context) {
        if let InsertState::Waiting {
            handoff,
            cancel_refused,
            ..
        } = &mut self.insert_state
        {
            if !handoff.cancel() {
                *cancel_refused = true;
                return;
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Whether a request to close the picker may go ahead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseDecision {
    Close,
    /// The daemon already has the insert: stay until its result is shown.
    KeepOpen,
}

/// Show a minimized picker again after a failed insert, so its explanation
/// and Copy instead are not lost. Wayland may not let a client restore
/// itself, so it also asks for the user's attention.
fn bring_back(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
        egui::UserAttentionType::Critical,
    ));
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
            self.cancel(&ctx);
        }
        // Closing the window follows the same policy as Escape: an insert
        // not yet sent is cancelled and the window closes; one the daemon
        // already has keeps the picker open until its result is known.
        if ctx.input(|input| input.viewport().close_requested())
            && self.close_requested() == CloseDecision::KeepOpen
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            bring_back(&ctx);
        }
        if down && count > 0 {
            self.selected = (self.selected + 1).min(count - 1);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        self.selected = self.selected.min(count.saturating_sub(1));
        // Enter and clicks pick only usable snippets; an unusable row shows
        // why instead of doing something unexpected.
        let usable = |index: usize| self.search_index[index].eligible();
        let mut chosen = (ready && enter && count > 0)
            .then(|| result_indices[self.selected])
            .filter(|index| usable(*index));

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

                let mut fallback_error = None;
                // Copy what the cards show out of the state, so the failure
                // card's buttons can update the picker while it renders.
                let shown_state = match &self.insert_state {
                    InsertState::Waiting {
                        trigger,
                        cancel_refused,
                        ..
                    } => ShownInsertState::Waiting {
                        trigger: trigger.clone(),
                        cancel_refused: *cancel_refused,
                    },
                    InsertState::Failed {
                        trigger,
                        error,
                        text,
                    } => ShownInsertState::Failed {
                        trigger: trigger.clone(),
                        error: error.clone(),
                        text: text.clone(),
                    },
                    InsertState::Ready => ShownInsertState::Ready,
                };
                match &shown_state {
                    ShownInsertState::Waiting {
                        trigger,
                        cancel_refused,
                    } => {
                        theme::card(ui, &palette, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(self.strings.picker_inserting(trigger)).strong(),
                            );
                            if *cancel_refused {
                                ui.colored_label(
                                    palette.warning,
                                    self.strings.picker_too_late_to_cancel(),
                                );
                            } else {
                                ui.label(self.strings.picker_waiting_for_focus());
                                if ui.button(self.strings.picker_cancel()).clicked() {
                                    self.cancel(&ctx);
                                }
                            }
                        });
                        ui.add_space(10.0);
                    }
                    ShownInsertState::Failed {
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
                            let copy_blocked = self.fallback_copy_blocked(trigger);
                            if let Some(reason) = &copy_blocked {
                                ui.label(RichText::new(reason).small().color(palette.muted));
                            }
                            ui.horizontal(|ui| {
                                // A failed direct insert keeps no rendered text
                                // (the daemon renders for itself), so render
                                // the clipboard copy now, under the same rules
                                // as copy-only mode.
                                if copy_blocked.is_none()
                                    && ui.button(self.strings.picker_copy_instead()).clicked()
                                {
                                    match self.fallback_copy_text(trigger, text) {
                                        Ok(rendered) => {
                                            self.copied =
                                                Some(copy_snippet(&ctx, rendered, trigger));
                                        }
                                        Err(message) => fallback_error = Some(message),
                                    }
                                }
                                if ui.button(self.strings.picker_cancel()).clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                        });
                        ui.add_space(10.0);
                    }
                    ShownInsertState::Ready => {}
                }
                if let Some(error) = fallback_error {
                    if let InsertState::Failed { error: shown, .. } = &mut self.insert_state {
                        *shown = error;
                    }
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
                            let preview = self.preview_at(expansion_index);
                            let expansion = &self.config.expansion[expansion_index];
                            let selected = index == self.selected;
                            let availability = self.search_index[expansion_index].availability;
                            let response = picker_row(
                                ui,
                                &palette,
                                &preview,
                                &self.search_index[expansion_index].app_scope,
                                &self.strings,
                                expansion,
                                selected,
                                self.strings.picker_unavailable_reason(availability),
                            );
                            if selected && (up || down) {
                                response.scroll_to_me(None);
                            }
                            if response.hovered() && ui.input(|input| input.pointer.is_moving()) {
                                self.selected = index;
                            }
                            if ready
                                && response.clicked()
                                && availability == Availability::Available
                            {
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

#[allow(clippy::too_many_arguments)]
fn picker_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    preview: &str,
    app_scope: &str,
    strings: &Strings,
    expansion: &ExpansionConfig,
    selected: bool,
    unavailable: Option<&str>,
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
                        .color(if unavailable.is_some() {
                            palette.muted
                        } else {
                            palette.accent
                        }),
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
            if let Some(reason) = unavailable {
                ui.label(RichText::new(reason).small().color(palette.warning));
                return;
            }
            ui.add(
                egui::Label::new(RichText::new(first_line).small().color(palette.muted)).truncate(),
            );
            if !app_scope.is_empty() {
                ui.label(
                    RichText::new(strings.picker_app_scope(app_scope))
                        .small()
                        .color(palette.warning),
                );
            }
        });
    // An unusable row is not a button: no click, no pointing hand, and
    // assistive technology hears its reason rather than an action.
    let usable = unavailable.is_none();
    let response = ui.interact(
        frame.response.rect,
        ui.id().with(("picker_row", &expansion.trigger)),
        if usable {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| {
        if usable {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                selected,
                expansion.trigger.as_str(),
            )
        } else {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                false,
                format!("{}: {}", expansion.trigger, unavailable.unwrap_or_default()),
            )
        }
    });
    if usable {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn format_app_scope(filters: &[String]) -> String {
    filters
        .iter()
        .map(|filter| match wayexpand_core::AppFilter::parse(filter) {
            Some(wayexpand_core::AppFilter::AppIdExact(value)) => {
                format!("app ID {value}")
            }
            Some(wayexpand_core::AppFilter::AppIdGlob(value)) => {
                format!("app ID matching {value}")
            }
            Some(wayexpand_core::AppFilter::TitleContains(value)) => {
                format!("window title containing {value:?}")
            }
            Some(wayexpand_core::AppFilter::ProcessExecutableExact(value)) => {
                format!("process executable {value:?}")
            }
            Some(wayexpand_core::AppFilter::ProcessNameExact(value)) => {
                format!("mutable process name (lower assurance) {value:?}")
            }
            None => format!("configured filter {filter:?}"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Who decides a pending direct insert: the user cancelling it, or the
/// worker handing it to the daemon. Both change the state under one lock, so
/// exactly one of them wins. Once the daemon has the request it cannot be
/// withdrawn, and the picker says so instead of claiming a cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandoffState {
    Waiting,
    Cancelled,
    Submitted,
}

#[derive(Clone)]
struct InsertHandoff(Arc<Mutex<HandoffState>>);

impl InsertHandoff {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(HandoffState::Waiting)))
    }

    fn state(&self) -> HandoffState {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The user cancels. True when nothing was or will be sent; false when
    /// the worker had already handed the insert to the daemon.
    fn cancel(&self) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *state {
            HandoffState::Waiting | HandoffState::Cancelled => {
                *state = HandoffState::Cancelled;
                true
            }
            HandoffState::Submitted => false,
        }
    }

    /// The worker is about to send the insert. False when it was cancelled,
    /// in which case nothing may be sent.
    fn begin_submit(&self) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *state == HandoffState::Waiting {
            *state = HandoffState::Submitted;
            true
        } else {
            false
        }
    }
}

/// What the insert worker reports back to the picker.
#[derive(Debug, PartialEq, Eq)]
enum InsertOutcome {
    /// The daemon's output backend acknowledged the text. The application's
    /// resulting document is not observed; only the user can confirm it.
    OutputAcknowledged,
    /// The user cancelled before anything was sent.
    Cancelled,
    /// Nothing, or possibly part, was inserted; the message says which.
    Failed(PickerMessage),
}

/// Everything the picker tells the user about an insert or a copy. The
/// worker thread has no language settings, so it reports one of these and
/// the window renders it in the user's language (`Strings::picker_message`).
/// Details carried as strings come from the system and stay as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PickerMessage {
    FocusUnknown,
    WorkerStartFailed(String),
    WorkerStopped,
    RenderFailed(String),
    CopyNotAllowedByPolicy,
    CopyNotAllowedForSnippet,
    SnippetGone,
    OlderService,
    CallFailed(String),
    Refused(String),
    FocusNotReturned,
    LostContact(String),
    UnexpectedReply(String),
    NotConfirmed(u64),
    LostTrack,
    Uncertain,
    NotInserted(wayexpand_core::InsertFailure),
}

/// The daemon calls the insert worker makes, so tests can drive it.
trait PickerDaemon {
    fn focus(&mut self) -> Option<FocusTarget>;
    fn insert_tracked(&mut self, target: &FocusTarget, trigger: &str) -> Result<String, String>;
    fn insert_result(&mut self, id: u64) -> Result<String, String>;
}

struct ControlSocket;

impl PickerDaemon for ControlSocket {
    fn focus(&mut self) -> Option<FocusTarget> {
        crate::runtime::control_command(wayexpand_core::DaemonOperation::Focus)
            .ok()
            .and_then(|response| focus_target_from_response(&response))
            .and_then(|info| info.target)
    }

    fn insert_tracked(&mut self, target: &FocusTarget, trigger: &str) -> Result<String, String> {
        crate::runtime::insert_target_tracked(target.generation, &target.token, trigger)
            .map_err(|error| error.to_string())
    }

    fn insert_result(&mut self, id: u64) -> Result<String, String> {
        crate::runtime::insert_result(id).map_err(|error| error.to_string())
    }
}

/// Timing for the insert worker; tests shorten it.
#[derive(Clone, Copy)]
struct InsertTiming {
    focus_timeout: Duration,
    focus_poll: Duration,
    delivery_timeout: Duration,
    delivery_poll: Duration,
}

const INSERT_TIMING: InsertTiming = InsertTiming {
    focus_timeout: FOCUS_RETURN_TIMEOUT,
    focus_poll: FOCUS_POLL_INTERVAL,
    delivery_timeout: DELIVERY_TIMEOUT,
    delivery_poll: DELIVERY_POLL_INTERVAL,
};

/// Wait for focus to return to the window the picker was opened from, send
/// the insert unless the user cancelled first, and wait for the daemon to
/// report its outcome (an output acknowledgement at best; the application's
/// document is not observed).
fn wait_for_focus_and_insert(
    daemon: &mut dyn PickerDaemon,
    target_focus: &FocusTarget,
    trigger: &str,
    handoff: &InsertHandoff,
    timing: InsertTiming,
) -> InsertOutcome {
    let deadline = Instant::now() + timing.focus_timeout;
    loop {
        if handoff.state() == HandoffState::Cancelled {
            return InsertOutcome::Cancelled;
        }
        if let Some(current) = daemon.focus() {
            if focus_returned_to_target(target_focus, &current) {
                if !handoff.begin_submit() {
                    return InsertOutcome::Cancelled;
                }
                let response = match daemon.insert_tracked(&current, trigger) {
                    Ok(response) => response,
                    Err(error) => return InsertOutcome::Failed(PickerMessage::CallFailed(error)),
                };
                if let Some(id) = wayexpand_core::parse_insert_scheduled_id(&response) {
                    return wait_for_delivery(daemon, id, timing);
                }
                if response.starts_with("unknown command") {
                    // An older daemon cannot report delivery. Closing on its
                    // untracked "insert scheduled" would claim a success
                    // nobody observed, so send nothing.
                    return InsertOutcome::Failed(PickerMessage::OlderService);
                }
                return InsertOutcome::Failed(PickerMessage::Refused(
                    response.trim_end().to_owned(),
                ));
            }
        }
        if Instant::now() >= deadline {
            return InsertOutcome::Failed(PickerMessage::FocusNotReturned);
        }
        thread::sleep(timing.focus_poll);
    }
}

/// Poll the daemon until it reports what became of insert `id`.
fn wait_for_delivery(
    daemon: &mut dyn PickerDaemon,
    id: u64,
    timing: InsertTiming,
) -> InsertOutcome {
    let deadline = Instant::now() + timing.delivery_timeout;
    loop {
        let response = match daemon.insert_result(id) {
            Ok(response) => response,
            Err(error) => return InsertOutcome::Failed(PickerMessage::LostContact(error)),
        };
        let Ok(state) = wayexpand_core::InsertDelivery::parse_response(&response, id) else {
            return InsertOutcome::Failed(PickerMessage::UnexpectedReply(
                response.trim_end().to_owned(),
            ));
        };
        match delivery_verdict(state) {
            Some(Ok(())) => return InsertOutcome::OutputAcknowledged,
            Some(Err(message)) => return InsertOutcome::Failed(message),
            None => {}
        }
        if Instant::now() >= deadline {
            return InsertOutcome::Failed(PickerMessage::NotConfirmed(
                timing.delivery_timeout.as_secs(),
            ));
        }
        thread::sleep(timing.delivery_poll);
    }
}

/// What the picker does with a reported insert state: `None` keeps waiting,
/// `Ok` closes the picker, `Err` shows the failure with "Copy instead".
fn delivery_verdict(
    state: Option<wayexpand_core::InsertDelivery>,
) -> Option<Result<(), PickerMessage>> {
    use wayexpand_core::InsertDelivery;
    let Some(state) = state else {
        return Some(Err(PickerMessage::LostTrack));
    };
    match state {
        InsertDelivery::Pending => None,
        InsertDelivery::Delivered => Some(Ok(())),
        InsertDelivery::Uncertain => Some(Err(PickerMessage::Uncertain)),
        InsertDelivery::Failed(failure) => Some(Err(PickerMessage::NotInserted(failure))),
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
    let loaded = Config::load_versioned(&path)
        .map_err(|error| anyhow::anyhow!("configuration invalid: {}", error.safe_summary()))?;
    let mut config = loaded.config;
    config
        .apply_administrator_policy(&policy)
        .map_err(|error| {
            anyhow::anyhow!("configuration policy invalid: {}", error.safe_summary())
        })?;
    // The default external policy preserves user-embedded policy. Evaluate the
    // merged config so copy-only delivery observes the same effective policy
    // as automatic matching and insertion.
    let clipboard_fallback_allowed = clipboard_fallback_allowed_by_policy(&config.organization);
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
    picker_availability(expansion, safe_direct_insert) == Availability::Available
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
#[cfg(test)]
fn top_ranked(mut ranked: Vec<(i64, usize)>, limit: usize) -> Vec<usize> {
    let mut result = Vec::new();
    top_ranked_into(&mut ranked, limit, &mut result);
    result
}

/// Keep the ranking allocation reusable across query edits. Large libraries
/// can produce many matches, while the picker only displays a small prefix;
/// clearing this scratch vector retains its high-water capacity for the next
/// keystroke.
fn top_ranked_into(ranked: &mut Vec<(i64, usize)>, limit: usize, result: &mut Vec<usize>) {
    result.clear();
    let order = |a: &(i64, usize), b: &(i64, usize)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
    if limit == 0 {
        ranked.clear();
        return;
    }
    if ranked.len() > limit {
        ranked.select_nth_unstable_by(limit - 1, order);
        ranked.truncate(limit);
    }
    ranked.sort_unstable_by(order);
    result.extend(ranked.iter().map(|(_, index)| *index));
    ranked.clear();
}

#[cfg(test)]
mod tests {
    use super::PickerMessage;
    use super::{
        clipboard_fallback_allowed_by_policy, copy_snippet, delivery_verdict,
        focus_returned_to_target, focus_target_from_response, format_app_scope,
        picker_expansion_is_eligible, picker_focus_state, picker_needs_daemon_focus, score,
        top_ranked, top_ranked_into, FocusInfo, FocusTarget, SearchEntry, MAX_RESULTS,
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

    #[test]
    fn top_ranked_reuses_scratch_capacity_between_queries() {
        let mut ranked: Vec<(i64, usize)> = (0..10_000)
            .map(|index| ((index % 31) as i64, index))
            .collect();
        let mut result = Vec::with_capacity(MAX_RESULTS);
        let expected = top_ranked(ranked.clone(), MAX_RESULTS);

        top_ranked_into(&mut ranked, MAX_RESULTS, &mut result);

        assert_eq!(result, expected);
        let retained_capacity = ranked.capacity();
        assert!(retained_capacity >= 10_000);
        assert!(ranked.is_empty());

        ranked.extend((0..10_000).map(|index| ((index % 17) as i64, index)));
        top_ranked_into(&mut ranked, MAX_RESULTS, &mut result);
        assert_eq!(result.len(), MAX_RESULTS);
        assert_eq!(ranked.capacity(), retained_capacity);
    }

    use wayexpand_core::{Config, ExpansionConfig, MatchMode};

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
            delivery: wayexpand_core::ExpansionDelivery::Insert,
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
                delivery: wayexpand_core::ExpansionDelivery::Insert,
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
            delivery: wayexpand_core::ExpansionDelivery::Insert,
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
    fn search_entry_caches_delivery_eligibility() {
        let expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":host".into(),
            replacement: "admin@host".into(),
            delivery: wayexpand_core::ExpansionDelivery::Insert,
            description: "SSH prompt".into(),
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

        assert!(SearchEntry::from_expansion(&expansion, false).eligible());

        let mut app_scoped = expansion;
        app_scoped
            .app_filter
            .push("app_id_exact:org.example.Terminal".into());
        assert!(!SearchEntry::from_expansion(&app_scoped, false).eligible());
        assert!(SearchEntry::from_expansion(&app_scoped, true).eligible());
    }

    #[test]
    fn app_scope_explains_each_supported_filter_kind() {
        let scope = format_app_scope(&[
            "app_id_exact:org.example.Editor".into(),
            "app_id_glob:org.example.*".into(),
            "title_contains:production".into(),
            "process_executable_exact:/usr/bin/konsole".into(),
            "process_name_exact:Konsole".into(),
            "unrecognized:value".into(),
        ]);

        assert_eq!(
            scope,
            "app ID org.example.editor; app ID matching org.example.*; window title containing \"production\"; process executable \"/usr/bin/konsole\"; mutable process name (lower assurance) \"konsole\"; configured filter \"unrecognized:value\""
        );
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
    fn copy_only_mode_respects_embedded_verified_target_policy() {
        let mut config = Config::parse(
            "[organization]\nsafe_mode = true\nrequire_verified_insert_target = true\n",
        )
        .unwrap();
        config
            .apply_administrator_policy(&wayexpand_core::OrganizationPolicy::default())
            .unwrap();

        assert!(!clipboard_fallback_allowed_by_policy(&config.organization));
    }

    #[test]
    fn copy_only_mode_respects_enforced_external_verified_target_policy() {
        let mut config = Config::parse("[organization]\nsafe_mode = false\n").unwrap();
        let enforced_policy = wayexpand_core::OrganizationPolicy {
            safe_mode: true,
            require_verified_insert_target: true,
            ..wayexpand_core::OrganizationPolicy::default()
        };
        config.apply_administrator_policy(&enforced_policy).unwrap();

        assert!(!clipboard_fallback_allowed_by_policy(&config.organization));
    }

    #[test]
    fn form_snippets_are_hidden_until_picker_form_submission_exists() {
        let expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":form".into(),
            replacement: "Hello {{field:name}}".into(),
            delivery: wayexpand_core::ExpansionDelivery::Insert,
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

    #[test]
    fn the_picker_closes_only_after_a_confirmed_delivery() {
        use wayexpand_core::{InsertDelivery, InsertFailure};
        assert_eq!(delivery_verdict(Some(InsertDelivery::Pending)), None);
        assert_eq!(
            delivery_verdict(Some(InsertDelivery::Delivered)),
            Some(Ok(()))
        );
        assert_eq!(
            delivery_verdict(Some(InsertDelivery::Uncertain)),
            Some(Err(PickerMessage::Uncertain))
        );
        assert_eq!(delivery_verdict(None), Some(Err(PickerMessage::LostTrack)));
        for failure in [
            InsertFailure::FocusChanged,
            InsertFailure::PolicyRefused,
            InsertFailure::InputArrived,
            InsertFailure::NoBackend,
            InsertFailure::Refused,
            InsertFailure::NotApplied,
            InsertFailure::Superseded,
        ] {
            assert_eq!(
                delivery_verdict(Some(InsertDelivery::Failed(failure))),
                Some(Err(PickerMessage::NotInserted(failure)))
            );
        }
    }

    #[test]
    fn every_picker_message_is_translated() {
        use crate::lang::{Language, Strings};
        use wayexpand_core::InsertFailure;
        let mut messages = vec![
            PickerMessage::FocusUnknown,
            PickerMessage::WorkerStartFailed("x".into()),
            PickerMessage::WorkerStopped,
            PickerMessage::RenderFailed("x".into()),
            PickerMessage::CopyNotAllowedByPolicy,
            PickerMessage::CopyNotAllowedForSnippet,
            PickerMessage::SnippetGone,
            PickerMessage::OlderService,
            PickerMessage::CallFailed("x".into()),
            PickerMessage::Refused("x".into()),
            PickerMessage::FocusNotReturned,
            PickerMessage::LostContact("x".into()),
            PickerMessage::UnexpectedReply("x".into()),
            PickerMessage::NotConfirmed(15),
            PickerMessage::LostTrack,
            PickerMessage::Uncertain,
        ];
        messages.extend(
            [
                InsertFailure::FocusChanged,
                InsertFailure::PolicyRefused,
                InsertFailure::InputArrived,
                InsertFailure::NoBackend,
                InsertFailure::Refused,
                InsertFailure::NotApplied,
                InsertFailure::Superseded,
            ]
            .map(PickerMessage::NotInserted),
        );
        let english = Strings::new(Language::English);
        for message in &messages {
            let en = english.picker_message(message);
            assert!(!en.is_empty(), "{message:?}");
            for language in [Language::German, Language::Indonesian] {
                let other = Strings::new(language).picker_message(message);
                assert_ne!(other, en, "{message:?} is not translated");
            }
        }
        // Commands stay commands in every language.
        for language in [Language::English, Language::German, Language::Indonesian] {
            assert!(Strings::new(language)
                .picker_message(&PickerMessage::OlderService)
                .contains("wayexpand service restart --yes"));
        }
    }

    mod insert_handoff {
        use super::super::{
            wait_for_focus_and_insert, FocusTarget, InsertHandoff, InsertOutcome, InsertTiming,
            PickerApp, PickerDaemon, PickerMessage,
        };
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        const TIMING: InsertTiming = InsertTiming {
            focus_timeout: Duration::from_secs(5),
            focus_poll: Duration::from_millis(1),
            delivery_timeout: Duration::from_secs(5),
            delivery_poll: Duration::from_millis(1),
        };

        fn target(generation: u64) -> FocusTarget {
            FocusTarget {
                generation,
                token: "abcd".into(),
            }
        }

        /// Focus returns once `focus_returned` is set; records every insert.
        struct FakeDaemon {
            focus_returned: Arc<Mutex<bool>>,
            inserts: Arc<Mutex<Vec<String>>>,
            insert_reply: String,
            result_reply: String,
        }

        impl FakeDaemon {
            fn new(insert_reply: &str, result_reply: &str) -> Self {
                Self {
                    focus_returned: Arc::new(Mutex::new(false)),
                    inserts: Arc::new(Mutex::new(Vec::new())),
                    insert_reply: insert_reply.into(),
                    result_reply: result_reply.into(),
                }
            }
        }

        impl PickerDaemon for FakeDaemon {
            fn focus(&mut self) -> Option<FocusTarget> {
                Some(target(if *self.focus_returned.lock().unwrap() {
                    2
                } else {
                    1
                }))
            }
            fn insert_tracked(
                &mut self,
                _target: &FocusTarget,
                trigger: &str,
            ) -> Result<String, String> {
                self.inserts.lock().unwrap().push(trigger.into());
                Ok(self.insert_reply.clone())
            }
            fn insert_result(&mut self, _id: u64) -> Result<String, String> {
                Ok(self.result_reply.clone())
            }
        }

        #[test]
        fn cancelling_before_focus_returns_sends_nothing() {
            let mut daemon = FakeDaemon::new("insert scheduled id=1\n", "");
            let inserts = Arc::clone(&daemon.inserts);
            let focus_returned = Arc::clone(&daemon.focus_returned);
            let handoff = InsertHandoff::new();
            let worker_handoff = handoff.clone();
            let worker = std::thread::spawn(move || {
                wait_for_focus_and_insert(&mut daemon, &target(1), ";sig", &worker_handoff, TIMING)
            });
            std::thread::sleep(Duration::from_millis(20));
            assert!(handoff.cancel(), "nothing was sent yet");
            // Closing the picker returns focus to the original window, which
            // is exactly when the old worker would have inserted.
            *focus_returned.lock().unwrap() = true;
            assert_eq!(worker.join().unwrap(), InsertOutcome::Cancelled);
            assert!(inserts.lock().unwrap().is_empty());
        }

        #[test]
        fn a_cancel_after_the_daemon_has_the_insert_is_refused() {
            let mut daemon = FakeDaemon::new(
                "insert scheduled id=1\n",
                &wayexpand_core::InsertDelivery::Delivered.response_line(1),
            );
            *daemon.focus_returned.lock().unwrap() = true;
            let inserts = Arc::clone(&daemon.inserts);
            let handoff = InsertHandoff::new();
            let outcome =
                wait_for_focus_and_insert(&mut daemon, &target(1), ";sig", &handoff, TIMING);
            assert_eq!(outcome, InsertOutcome::OutputAcknowledged);
            assert_eq!(*inserts.lock().unwrap(), [";sig"]);
            assert!(!handoff.cancel(), "the insert was already sent");
        }

        #[test]
        fn closing_the_window_cancels_or_waits_for_the_result() {
            use super::super::{CloseDecision, HandoffState, InsertState};
            let loaded = wayexpand_core::Config::parse(
                "[[expansion]]\ntrigger = ';sig'\nreplacement = 'x'\n",
            )
            .unwrap();
            let mut app = PickerApp::new(
                loaded,
                wayexpand_core::Config::load_versioned(&{
                    let path = std::env::temp_dir().join(format!(
                        "wayexpand-picker-close-{}.toml",
                        std::process::id()
                    ));
                    std::fs::write(&path, "").unwrap();
                    std::fs::set_permissions(
                        &path,
                        std::os::unix::fs::PermissionsExt::from_mode(0o600),
                    )
                    .unwrap();
                    path
                })
                .unwrap()
                .revision,
                crate::lang::Language::English,
                crate::colorpack::ColorPack::Default,
                true,
                true,
                true,
                Some(target(1)),
            );
            let waiting = |handoff: InsertHandoff| InsertState::Waiting {
                trigger: ";sig".into(),
                text: String::new(),
                receiver: std::sync::mpsc::channel().1,
                handoff,
                cancel_refused: false,
            };

            // Not sent yet: closing cancels it.
            let handoff = InsertHandoff::new();
            app.insert_state = waiting(handoff.clone());
            assert_eq!(app.close_requested(), CloseDecision::Close);
            assert_eq!(handoff.state(), HandoffState::Cancelled);

            // Already sent: the picker stays to show what happens.
            let handoff = InsertHandoff::new();
            assert!(handoff.begin_submit());
            app.insert_state = waiting(handoff);
            assert_eq!(app.close_requested(), CloseDecision::KeepOpen);
            assert!(matches!(
                app.insert_state,
                InsertState::Waiting {
                    cancel_refused: true,
                    ..
                }
            ));

            // Nothing pending: close.
            app.insert_state = InsertState::Ready;
            assert_eq!(app.close_requested(), CloseDecision::Close);
        }

        #[test]
        fn exactly_one_side_wins_the_handoff() {
            let handoff = InsertHandoff::new();
            assert!(handoff.cancel());
            assert!(!handoff.begin_submit(), "a cancelled insert is never sent");
            let handoff = InsertHandoff::new();
            assert!(handoff.begin_submit());
            assert!(!handoff.cancel());
        }

        #[test]
        fn an_older_daemon_is_never_reported_as_a_success() {
            let mut daemon = FakeDaemon::new("unknown command; expected status\n", "");
            *daemon.focus_returned.lock().unwrap() = true;
            let inserts = Arc::clone(&daemon.inserts);
            let outcome = wait_for_focus_and_insert(
                &mut daemon,
                &target(1),
                ";sig",
                &InsertHandoff::new(),
                TIMING,
            );
            assert_eq!(
                outcome,
                InsertOutcome::Failed(PickerMessage::OlderService),
                "an unconfirmable insert must not close the picker"
            );
            // Only the tracked request was sent; no untracked fallback.
            assert_eq!(inserts.lock().unwrap().len(), 1);
        }

        #[test]
        fn copy_instead_applies_the_copy_only_rules() {
            let path = std::env::temp_dir().join(format!(
                "wayexpand-picker-copy-rules-{}.toml",
                std::process::id()
            ));
            std::fs::write(
                &path,
                "[settings]\nallow_clipboard = true\n\n\
                 [[expansion]]\ntrigger = ';plain'\nreplacement = 'plain text'\n\n\
                 [[expansion]]\ntrigger = ';scoped'\nreplacement = 'scoped text'\n\
                 app_filter = ['org.example.Editor']\n\n\
                 [[expansion]]\ntrigger = ';clip'\nreplacement = 'from {{clipboard}}'\n",
            )
            .unwrap();
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
                .unwrap();
            let loaded = wayexpand_core::Config::load_versioned(&path).unwrap();
            let _ = std::fs::remove_file(&path);
            let app = PickerApp::new(
                loaded.config,
                loaded.revision,
                crate::lang::Language::English,
                crate::colorpack::ColorPack::Default,
                true,
                true,
                true,
                Some(target(1)),
            );
            assert_eq!(app.fallback_copy_blocked(";plain"), None);
            assert_eq!(app.fallback_copy_text(";plain", "").unwrap(), "plain text");
            for trigger in [";scoped", ";clip"] {
                assert!(app.fallback_copy_blocked(trigger).is_some(), "{trigger}");
                assert!(app.fallback_copy_text(trigger, "").is_err(), "{trigger}");
                assert!(
                    app.fallback_copy_text(trigger, "pre-rendered").is_err(),
                    "{trigger}: rendered text does not bypass the rules"
                );
            }
        }
    }

    mod availability {
        use super::super::{picker_availability, Availability, PickerApp};

        fn picker(toml: &str, safe_direct_insert: bool) -> PickerApp {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "wayexpand-picker-availability-{}-{}.toml",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::write(&path, toml).unwrap();
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
                .unwrap();
            let loaded = wayexpand_core::Config::load_versioned(&path).unwrap();
            let _ = std::fs::remove_file(&path);
            PickerApp::new(
                loaded.config,
                loaded.revision,
                crate::lang::Language::English,
                crate::colorpack::ColorPack::Default,
                true,
                safe_direct_insert,
                true,
                None,
            )
        }

        const LIBRARY: &str =
            "[[expansion]]\ntrigger = ';form'\nreplacement = 'Hi {{field:name}}'\n\n\
             [[expansion]]\ntrigger = ';cmd'\nreplacement = ''\n\
             [expansion.command]\nprogram = '/usr/bin/true'\n\n\
             [[expansion]]\ntrigger = ';scoped'\nreplacement = 'scoped'\n\
             app_filter = ['org.example.Editor']\n\n\
             [[expansion]]\ntrigger = ';off'\nreplacement = 'off'\nenabled = false\n\n\
             [[expansion]]\ntrigger = ';sig'\nreplacement = 'Best regards'\n";

        #[test]
        fn unusable_snippets_are_listed_after_usable_ones_with_a_reason() {
            let mut app = picker(LIBRARY, false);
            app.refresh_results();
            let listed: Vec<&str> = app
                .result_indices
                .iter()
                .map(|index| app.config.expansion[*index].trigger.as_str())
                .collect();
            assert_eq!(listed.first(), Some(&";sig"), "{listed:?}");
            assert!(!listed.contains(&";off"), "turned-off snippets stay out");
            for trigger in [";form", ";cmd", ";scoped"] {
                assert!(listed.contains(&trigger), "{trigger} is listed: {listed:?}");
            }
            for index in &app.result_indices {
                let availability = app.search_index[*index].availability;
                let reason = app.strings.picker_unavailable_reason(availability);
                assert_eq!(
                    reason.is_some(),
                    availability != Availability::Available,
                    "{availability:?}"
                );
            }
        }

        #[test]
        fn the_reason_depends_on_how_the_picker_delivers() {
            let copy_only = picker(LIBRARY, false);
            let direct = picker(LIBRARY, true);
            let scoped = |app: &PickerApp| {
                let index = app
                    .config
                    .expansion
                    .iter()
                    .position(|expansion| expansion.trigger == ";scoped")
                    .unwrap();
                picker_availability(&app.config.expansion[index], app.safe_direct_insert)
            };
            assert_eq!(scoped(&copy_only), Availability::AppRestricted);
            assert_eq!(scoped(&direct), Availability::Available);
        }
    }
}
