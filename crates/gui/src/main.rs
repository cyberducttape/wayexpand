mod theme;

use anyhow::{Context, Result};
use eframe::egui::{self, Color32, RichText, ScrollArea, TextEdit};
use std::{
    env, fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};
use theme::Palette;
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{
    default_config_path, discover_backends, import_espanso, BackendState, BackendStatus,
    CommandConfig, Config, ConfigError, ExpansionConfig, ExpansionEngine, InputEvent, MatchMode,
    Settings,
};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTROL_RESPONSE_BYTES: usize = 4096;
const MAX_UNDO_HISTORY: usize = 32;
const TEMPLATE_VARIABLES: &[(&str, &str)] = &[
    ("{{date}}", "UTC date"),
    ("{{time}}", "UTC time"),
    ("{{datetime}}", "UTC date and time"),
    ("{{username}}", "current user"),
    ("{{hostname}}", "local hostname"),
    ("{{unix_timestamp}}", "Unix timestamp"),
    ("{{newline}}", "line break"),
    ("{{tab}}", "tab character"),
];

struct Draft {
    trigger: String,
    description: String,
    tags: String,
    replacement: String,
    enabled: bool,
    match_mode: MatchMode,
    command_enabled: bool,
    command_program: String,
    command_args: String,
    command_timeout_ms: String,
    command_cache_ms: String,
}

enum PendingAction {
    Select(usize),
    New,
    Duplicate,
    Delete,
    Reload,
}

struct GuiApp {
    path: PathBuf,
    config: Config,
    selected: Option<usize>,
    filter: String,
    preview_input: String,
    draft: Option<Draft>,
    undo: Vec<Config>,
    message: String,
    paused: bool,
    diagnostics_open: bool,
    daemon_status: String,
    backend_status: Vec<BackendStatus>,
    protocol_probes: Vec<(String, String)>,
    pending_action: Option<PendingAction>,
    import_open: bool,
    import_path: String,
    import_preview: Option<(Config, usize)>,
    settings_open: bool,
    settings_buffer: String,
    settings_error: Option<String>,
    dark_mode: bool,
}

impl GuiApp {
    fn load(path: PathBuf) -> Result<Self> {
        let config = match Config::load(&path) {
            Ok(config) => config,
            Err(ConfigError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "could not create configuration directory {}",
                            parent.display()
                        )
                    })?;
                }
                let config = Config {
                    expansion: Vec::new(),
                    hotkey: Vec::new(),
                    settings: Settings::default(),
                };
                config.save_atomic(&path).map_err(|error| {
                    anyhow::anyhow!(
                        "could not initialize configuration: {}",
                        error.safe_summary()
                    )
                })?;
                config
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "configuration invalid: {}",
                    error.safe_summary()
                ))
            }
        };
        let selected = (!config.expansion.is_empty()).then_some(0);
        let draft = selected.map(|index| Draft::from_expansion(&config.expansion[index]));
        let preview_input = selected
            .map(|index| config.expansion[index].trigger.clone())
            .unwrap_or_default();
        let settings_buffer = config.settings.max_buffer_chars.to_string();
        Ok(Self {
            path,
            config,
            selected,
            filter: String::new(),
            preview_input,
            draft,
            undo: Vec::new(),
            message: "Ready".into(),
            paused: false,
            diagnostics_open: false,
            daemon_status: "Not checked".into(),
            backend_status: discover_backends(),
            protocol_probes: Vec::new(),
            pending_action: None,
            import_open: false,
            import_path: String::new(),
            import_preview: None,
            settings_open: false,
            settings_buffer,
            settings_error: None,
            dark_mode: true,
        })
    }

    fn refresh_diagnostics(&mut self) {
        self.backend_status = discover_backends();
        self.protocol_probes.clear();
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            self.protocol_probes.push((
                "input-method-v2".into(),
                match InputMethodSource::probe() {
                    Ok(()) => "manager and seat available".into(),
                    Err(error) => format!("unavailable: {error}"),
                },
            ));
            self.protocol_probes.push((
                "wlroots-virtual-keyboard".into(),
                match WlrootsInjector::probe() {
                    Ok(()) => "manager and seat available".into(),
                    Err(error) => format!("unavailable: {error}"),
                },
            ));
        } else {
            self.protocol_probes.push((
                "Wayland protocol probes".into(),
                "skipped: no Wayland session detected".into(),
            ));
        }
        self.daemon_status = match control_command("status") {
            Ok(response) => response.trim().replace('\n', " · "),
            Err(error) => format!("Unavailable: {error}"),
        };
        self.message = "Diagnostics refreshed".into();
    }

    fn remember_undo(&mut self, previous: Config) {
        self.undo.push(previous);
        if self.undo.len() > MAX_UNDO_HISTORY {
            self.undo.remove(0);
        }
    }

    fn save_settings(&mut self) {
        let max_buffer_chars = match self.settings_buffer.trim().parse::<usize>() {
            Ok(value) => value,
            Err(error) => {
                let error = format!("buffer limit must be an integer ({error})");
                self.message = format!("Settings invalid: {error}");
                self.settings_error = Some(error);
                return;
            }
        };
        let mut candidate = self.config.clone();
        candidate.settings.max_buffer_chars = max_buffer_chars;
        if let Err(error) = candidate.validate() {
            let error = error.safe_summary();
            self.message = format!("Settings rejected: {error}");
            self.settings_error = Some(error);
            return;
        }
        match candidate.save_atomic(&self.path) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.remember_undo(previous);
                self.settings_buffer = self.config.settings.max_buffer_chars.to_string();
                self.settings_open = false;
                self.settings_error = None;
                self.message = "Settings saved atomically".into();
                let _ = control_command("reload");
            }
            Err(error) => {
                let error = error.safe_summary();
                self.message = format!("Settings save failed: {error}");
                self.settings_error = Some(error);
            }
        }
    }

    fn preview_import(&mut self) {
        let source = expand_user_path(self.import_path.trim());
        match import_espanso(&source) {
            Ok(imported) => {
                self.import_preview = Some((imported.config, imported.skipped));
                self.message = "Espanso library loaded for review".into();
            }
            Err(error) => self.message = format!("Import failed: {error}"),
        }
    }

    fn apply_import(&mut self) {
        if self.draft_is_dirty() {
            self.message = "Save or discard the current draft before importing".into();
            return;
        }
        let Some((imported, skipped)) = self.import_preview.take() else {
            return;
        };
        if let Err(error) = imported.validate() {
            self.message = format!("Import rejected: {}", error.safe_summary());
            return;
        }
        match imported.save_atomic(&self.path) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, imported);
                self.remember_undo(previous);
                self.selected = (!self.config.expansion.is_empty()).then_some(0);
                self.draft = self
                    .selected
                    .map(|index| Draft::from_expansion(&self.config.expansion[index]));
                self.import_open = false;
                self.message = if skipped == 0 {
                    "Espanso library imported".into()
                } else {
                    format!("Espanso library imported; skipped {skipped} unsupported match(es)")
                };
                let _ = control_command("reload");
            }
            Err(error) => {
                self.message = format!("Import save failed: {}", error.safe_summary());
                self.import_preview = Some((imported, skipped));
            }
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        let query = self.filter.to_lowercase();
        self.config
            .expansion
            .iter()
            .enumerate()
            .filter(|(_, expansion)| {
                query.is_empty()
                    || format!(
                        "{} {} {}",
                        expansion.trigger,
                        expansion.description,
                        expansion.tags.join(" ")
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.draft = Some(Draft::from_expansion(&self.config.expansion[index]));
        self.preview_input = self.config.expansion[index].trigger.clone();
        self.pending_action = None;
    }

    fn draft_is_dirty(&self) -> bool {
        let (Some(index), Some(draft)) = (self.selected, self.draft.as_ref()) else {
            return false;
        };
        let expansion = &self.config.expansion[index];
        let command = draft.command_config().ok().flatten();
        draft.trigger != expansion.trigger
            || draft.description != expansion.description
            || draft.tags
                != expansion
                    .tags
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            || draft.replacement != expansion.replacement
            || draft.enabled != expansion.enabled
            || draft.match_mode != expansion.match_mode
            || command != expansion.command
    }

    fn request_action(&mut self, action: PendingAction) {
        if matches!(&action, PendingAction::Select(index) if self.selected == Some(*index)) {
            return;
        }
        if matches!(&action, PendingAction::Delete) && !self.draft_is_dirty() {
            self.pending_action = Some(action);
            return;
        }
        if self.draft_is_dirty() {
            self.pending_action = Some(action);
        } else {
            self.execute_action(action);
        }
    }

    fn execute_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Select(index) => self.select(index),
            PendingAction::New => self.create_new_snippet(),
            PendingAction::Duplicate => self.duplicate_selected(),
            PendingAction::Delete => self.perform_delete_selected(),
            PendingAction::Reload => self.perform_reload(),
        }
    }

    fn discard_pending(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        self.execute_action(action);
    }

    fn save_and_execute_pending(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        self.save_selected();
        if !self.draft_is_dirty() {
            self.execute_action(action);
        } else {
            self.pending_action = Some(action);
        }
    }

    fn perform_reload(&mut self) {
        match Config::load(&self.path) {
            Ok(config) => {
                self.config = config;
                self.selected = (!self.config.expansion.is_empty()).then_some(0);
                self.draft = self
                    .selected
                    .map(|index| Draft::from_expansion(&self.config.expansion[index]));
                self.preview_input = self
                    .selected
                    .map(|index| self.config.expansion[index].trigger.clone())
                    .unwrap_or_default();
                self.undo.clear();
                self.message = "Configuration reloaded".into();
            }
            Err(error) => self.message = format!("Reload failed: {}", error.safe_summary()),
        }
    }

    fn save_selected(&mut self) {
        let (Some(index), Some(draft)) = (self.selected, self.draft.as_ref()) else {
            self.message = "No snippet selected".into();
            return;
        };
        let mut candidate = self.config.clone();
        candidate.expansion[index].trigger = draft.trigger.clone();
        candidate.expansion[index].description = draft.description.clone();
        candidate.expansion[index].tags = draft
            .tags
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect();
        candidate.expansion[index].replacement = draft.replacement.clone();
        candidate.expansion[index].enabled = draft.enabled;
        candidate.expansion[index].match_mode = draft.match_mode;
        candidate.expansion[index].command = match draft.command_config() {
            Ok(command) => command,
            Err(error) => {
                self.message = format!("Command settings invalid: {error}");
                return;
            }
        };
        if let Err(error) = candidate.validate() {
            self.message = format!("Save rejected: {}", error.safe_summary());
            return;
        }
        match candidate.save_atomic(&self.path) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.remember_undo(previous);
                self.draft = self
                    .selected
                    .map(|selected| Draft::from_expansion(&self.config.expansion[selected]));
                self.message = "Snippet saved atomically".into();
                let _ = control_command("reload");
            }
            Err(error) => self.message = format!("Save failed: {}", error.safe_summary()),
        }
    }

    fn undo(&mut self) {
        let Some(previous) = self.undo.pop() else {
            self.message = "Nothing to undo".into();
            return;
        };
        self.config = previous;
        self.selected = self
            .selected
            .filter(|index| *index < self.config.expansion.len());
        self.draft = self
            .selected
            .map(|index| Draft::from_expansion(&self.config.expansion[index]));
        self.preview_input = self
            .selected
            .map(|index| self.config.expansion[index].trigger.clone())
            .unwrap_or_default();
        match self.config.save_atomic(&self.path) {
            Ok(()) => self.message = "Undid the last saved change".into(),
            Err(error) => self.message = format!("Undo save failed: {}", error.safe_summary()),
        }
    }

    fn create_new_snippet(&mut self) {
        let mut trigger = ":new".to_owned();
        let mut suffix = 2;
        while self
            .config
            .expansion
            .iter()
            .any(|item| item.trigger == trigger)
        {
            trigger = format!(":new-{suffix}");
            suffix += 1;
        }
        let mut candidate = self.config.clone();
        candidate.expansion.push(ExpansionConfig {
            trigger,
            replacement: String::new(),
            description: "New snippet".into(),
            tags: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
        });
        match candidate.save_atomic(&self.path) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.remember_undo(previous);
                self.select(self.config.expansion.len() - 1);
                self.message = "Created a new snippet".into();
            }
            Err(error) => self.message = format!("Create failed: {}", error.safe_summary()),
        }
    }

    fn duplicate_selected(&mut self) {
        let Some(index) = self.selected else {
            self.message = "No snippet selected".into();
            return;
        };
        let mut duplicate = self.config.expansion[index].clone();
        let base = format!("{}-copy", duplicate.trigger);
        let mut trigger = base.clone();
        let mut suffix = 2;
        while self
            .config
            .expansion
            .iter()
            .any(|item| item.trigger == trigger)
        {
            trigger = format!("{base}-{suffix}");
            suffix += 1;
        }
        duplicate.trigger = trigger;
        if !duplicate.description.is_empty() {
            duplicate.description.push_str(" (copy)");
        }
        let mut candidate = self.config.clone();
        candidate.expansion.push(duplicate);
        match candidate.save_atomic(&self.path) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.remember_undo(previous);
                self.select(self.config.expansion.len() - 1);
                self.message = "Duplicated snippet".into();
            }
            Err(error) => self.message = format!("Duplicate failed: {}", error.safe_summary()),
        }
    }

    fn perform_delete_selected(&mut self) {
        let Some(index) = self.selected else {
            self.message = "No snippet selected".into();
            return;
        };
        let mut candidate = self.config.clone();
        let trigger = candidate.expansion[index].trigger.clone();
        candidate.expansion.remove(index);
        match candidate.save_atomic(&self.path) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.remember_undo(previous);
                self.selected = (!self.config.expansion.is_empty())
                    .then_some(index.min(self.config.expansion.len() - 1));
                self.draft = self
                    .selected
                    .map(|selected| Draft::from_expansion(&self.config.expansion[selected]));
                self.message = format!("Deleted {trigger}");
            }
            Err(error) => self.message = format!("Delete failed: {}", error.safe_summary()),
        }
    }

    fn preview(&self) -> String {
        let Some(index) = self.selected else {
            return "No snippet selected".into();
        };
        let mut candidate = self.config.clone();
        if let Some(draft) = &self.draft {
            candidate.expansion[index].trigger = draft.trigger.clone();
            candidate.expansion[index].replacement = draft.replacement.clone();
            candidate.expansion[index].match_mode = draft.match_mode;
        }
        let Ok(mut engine) = ExpansionEngine::new(candidate) else {
            return "Configuration is invalid".into();
        };
        let mut results = engine.process(InputEvent::Text(self.preview_input.clone()));
        results.extend(engine.process(InputEvent::Boundary));
        results
            .last()
            .map(|result| result.insert.clone())
            .unwrap_or_else(|| "No expansion matched".into())
    }

    fn toggle_pause(&mut self) {
        let command = if self.paused { "resume" } else { "pause" };
        match control_command(command) {
            Ok(_) => {
                self.paused = !self.paused;
                self.message = if self.paused {
                    "Expansion paused"
                } else {
                    "Expansion resumed"
                }
                .into();
            }
            Err(error) => self.message = format!("Control unavailable: {error}"),
        }
    }
}

impl Draft {
    fn from_expansion(expansion: &ExpansionConfig) -> Self {
        let (command_enabled, command_program, command_args, command_timeout_ms, command_cache_ms) =
            match &expansion.command {
                Some(command) => (
                    true,
                    command.program.clone(),
                    command.args.join("\n"),
                    command.timeout_ms.to_string(),
                    command.cache_ms.to_string(),
                ),
                None => (
                    false,
                    String::new(),
                    String::new(),
                    "500".into(),
                    "0".into(),
                ),
            };
        Self {
            trigger: expansion.trigger.clone(),
            description: expansion.description.clone(),
            tags: expansion.tags.join(", "),
            replacement: expansion.replacement.clone(),
            enabled: expansion.enabled,
            match_mode: expansion.match_mode,
            command_enabled,
            command_program,
            command_args,
            command_timeout_ms,
            command_cache_ms,
        }
    }

    fn command_config(&self) -> Result<Option<CommandConfig>> {
        if !self.command_enabled {
            return Ok(None);
        }
        let program = self.command_program.trim();
        if program.is_empty() {
            anyhow::bail!("program is required when command expansion is enabled");
        }
        let timeout_ms = self
            .command_timeout_ms
            .trim()
            .parse::<u64>()
            .context("timeout must be an integer in milliseconds")?;
        let cache_ms = self
            .command_cache_ms
            .trim()
            .parse::<u64>()
            .context("cache duration must be an integer in milliseconds")?;
        Ok(Some(CommandConfig {
            program: program.to_owned(),
            args: self
                .command_args
                .lines()
                .map(str::trim)
                .filter(|arg| !arg.is_empty())
                .map(str::to_owned)
                .collect(),
            timeout_ms,
            cache_ms,
        }))
    }
}

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let palette = Palette::for_mode(self.dark_mode);
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .stroke(egui::Stroke::NONE)
                    .shadow(egui::Shadow {
                        offset: [0, 6],
                        blur: 14,
                        spread: 0,
                        color: Color32::from_black_alpha(if self.dark_mode { 60 } else { 18 }),
                    }),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚡").size(20.0).color(palette.accent));
                    ui.label(RichText::new("WayExpand").heading().strong());
                    ui.label(RichText::new("Snippet library").color(palette.muted));
                    ui.add_space(6.0);
                    theme::pill(
                        ui,
                        format!("{} snippets", self.config.expansion.len()),
                        palette.muted,
                        palette.surface_hover,
                    );
                    if !self.config.hotkey.is_empty() {
                        theme::pill(
                            ui,
                            format!("{} hotkeys", self.config.hotkey.len()),
                            palette.muted,
                            palette.surface_hover,
                        );
                    }
                    if self.draft_is_dirty() {
                        theme::pill(
                            ui,
                            "● Unsaved changes",
                            palette.warning,
                            theme::tint(palette.warning, 38),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(if self.dark_mode { "☀" } else { "🌙" })
                            .on_hover_text("Toggle light/dark theme")
                            .clicked()
                        {
                            self.dark_mode = !self.dark_mode;
                            ui.ctx().set_theme(if self.dark_mode {
                                egui::ThemePreference::Dark
                            } else {
                                egui::ThemePreference::Light
                            });
                        }
                        if ui.button("⚙ Settings").clicked() {
                            self.settings_buffer = self.config.settings.max_buffer_chars.to_string();
                            self.settings_error = None;
                            self.settings_open = true;
                        }
                        if ui.button("📥 Import Espanso").clicked() {
                            self.import_open = true;
                            self.import_preview = None;
                        }
                        if ui.button("🖥 Diagnostics").clicked() {
                            self.diagnostics_open = true;
                            self.refresh_diagnostics();
                        }
                        if ui
                            .button(if self.paused { "▶ Resume" } else { "⏸ Pause" })
                            .clicked()
                        {
                            self.toggle_pause();
                        }
                        if ui.button("↻ Reload").clicked() {
                            self.request_action(PendingAction::Reload);
                        }
                        ui.add(
                            TextEdit::singleline(&mut self.filter)
                                .hint_text("Search triggers, descriptions, or tags…")
                                .desired_width(220.0),
                        );
                    });
                });
            });
        if self.diagnostics_open {
            let mut open = self.diagnostics_open;
            egui::Window::new("🖥  WayExpand diagnostics")
                .open(&mut open)
                .resizable(true)
                .min_width(420.0)
                .show(ui.ctx(), |ui| {
                    theme::section_header(ui, "🖥", "Runtime health");
                    ui.add_space(4.0);
                    ui.label(RichText::new("Daemon").color(palette.muted).small());
                    egui::Frame::group(ui.style())
                        .fill(palette.surface_hover)
                        .show(ui, |ui| {
                            ui.label(RichText::new(&self.daemon_status).monospace());
                        });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        theme::section_header(ui, "🔌", "Backends");
                        if ui.small_button("↻ Refresh").clicked() {
                            self.refresh_diagnostics();
                        }
                    });
                    ui.add_space(4.0);
                    for status in &self.backend_status {
                        let color = match status.state {
                            BackendState::Available | BackendState::Implemented => palette.success,
                            BackendState::RequiresPermission => palette.warning,
                            BackendState::Unavailable | BackendState::NotImplemented => {
                                palette.muted
                            }
                        };
                        ui.horizontal(|ui| {
                            theme::pill(
                                ui,
                                format!("{:?}", status.state),
                                color,
                                theme::tint(color, 32),
                            );
                            ui.label(RichText::new(status.kind.to_string()).strong());
                        });
                        ui.label(RichText::new(&status.detail).small().color(palette.muted));
                        ui.add_space(4.0);
                    }
                    ui.separator();
                    theme::section_header(ui, "📡", "Non-mutating protocol probes");
                    ui.add_space(4.0);
                    for (name, detail) in &self.protocol_probes {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).strong());
                            ui.label(RichText::new(detail).color(palette.muted));
                        });
                    }
                });
            self.diagnostics_open = open;
        }
        if self.import_open {
            let mut open = self.import_open;
            egui::Window::new("📥  Import Espanso library")
                .open(&mut open)
                .resizable(false)
                .min_width(420.0)
                .show(ui.ctx(), |ui| {
                    ui.label("Source YAML file");
                    ui.add(
                        TextEdit::singleline(&mut self.import_path)
                            .hint_text("~/.config/espanso/match/base.yml")
                            .desired_width(520.0),
                    );
                    ui.label(
                        RichText::new(
                            "Import is previewed first and replaces this library only after explicit confirmation.",
                        )
                        .small()
                        .color(palette.muted),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, &palette, "Load preview").clicked() {
                            self.preview_import();
                        }
                        if ui.button("Cancel").clicked() {
                            self.import_preview = None;
                            self.import_open = false;
                        }
                    });
                    if let Some((config, skipped)) = self.import_preview.as_ref() {
                        ui.separator();
                        ui.label(format!(
                            "Preview: {} expansion(s), {} skipped unsupported match(es)",
                            config.expansion.len(),
                            skipped
                        ));
                        if theme::primary_button(ui, &palette, "Replace current library").clicked()
                        {
                            self.apply_import();
                        }
                    }
                });
            self.import_open = open && self.import_open;
        }
        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new("⚙  WayExpand settings")
                .open(&mut open)
                .resizable(false)
                .min_width(360.0)
                .show(ui.ctx(), |ui| {
                    ui.label("Matcher buffer limit");
                    ui.add(TextEdit::singleline(&mut self.settings_buffer).desired_width(120.0));
                    ui.label(
                        RichText::new("Characters retained while looking for a trigger (1–4096).")
                            .small()
                            .color(palette.muted),
                    );
                    if let Some(error) = &self.settings_error {
                        ui.add_space(4.0);
                        ui.colored_label(palette.danger, format!("⚠ {error}"));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, &palette, "Save settings").clicked() {
                            self.save_settings();
                        }
                        if ui.button("Close").clicked() {
                            self.settings_open = false;
                        }
                    });
                });
            self.settings_open = open && self.settings_open;
        }
        egui::Panel::left("snippets")
            .resizable(true)
            .default_size(340.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(14, 14)),
            )
            .show(ui, |ui| {
                ui.label(
                    RichText::new(if self.filter.is_empty() {
                        "Your reusable text library"
                    } else {
                        "Filtered snippets"
                    })
                    .small()
                    .color(palette.muted),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, &palette, "+ New").clicked() {
                        self.request_action(PendingAction::New);
                    }
                    if ui.button("⎘ Duplicate").clicked() {
                        self.request_action(PendingAction::Duplicate);
                    }
                    if ui.button(format!("↺ Undo ({})", self.undo.len())).clicked() {
                        self.undo();
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(4.0);
                let visible_indices = self.visible_indices();
                ScrollArea::vertical().show(ui, |ui| {
                    for index in visible_indices {
                        let expansion = &self.config.expansion[index];
                        let detail = if expansion.command.is_some() {
                            "Command-backed snippet".to_owned()
                        } else {
                            expansion.description.clone()
                        };
                        let response = theme::snippet_row(
                            ui,
                            &palette,
                            theme::SnippetRow {
                                selected: self.selected == Some(index),
                                enabled: expansion.enabled,
                                command_backed: expansion.command.is_some(),
                                trigger: &expansion.trigger,
                                detail: &detail,
                            },
                        );
                        if response.clicked() {
                            self.request_action(PendingAction::Select(index));
                        }
                    }
                    if self.config.expansion.is_empty() {
                        ui.add_space(16.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("📭").size(28.0));
                            ui.label(RichText::new("No snippets yet.").color(palette.muted));
                            ui.add_space(6.0);
                            if theme::primary_button(ui, &palette, "Create your first snippet")
                                .clicked()
                            {
                                self.request_action(PendingAction::New);
                            }
                        });
                    } else if self.visible_indices().is_empty() {
                        ui.add_space(16.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("🔍").size(28.0));
                            ui.label(
                                RichText::new("No snippets match this search.")
                                    .color(palette.muted),
                            );
                            ui.add_space(6.0);
                            if ui.button("Clear search").clicked() {
                                self.filter.clear();
                            }
                        });
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(if self.dark_mode {
                        Color32::from_rgb(0x14, 0x16, 0x1A)
                    } else {
                        Color32::from_rgb(0xF5, 0xF6, 0xF8)
                    })
                    .inner_margin(egui::Margin::symmetric(22, 18)),
            )
            .show(ui, |ui| {
            let Some(index) = self.selected else {
                ui.vertical_centered(|ui| {
                    ui.add_space(70.0);
                    ui.label(RichText::new("✨").size(40.0));
                    ui.add_space(6.0);
                    ui.heading("Build your first expansion");
                    ui.label(
                        RichText::new("Turn repetitive text into a fast, reliable shortcut.")
                            .color(palette.muted),
                    );
                    ui.add_space(10.0);
                    if theme::primary_button(ui, &palette, "+ Create snippet").clicked() {
                        self.request_action(PendingAction::New);
                    }
                });
                return;
            };
            if index >= self.config.expansion.len() {
                self.selected = None;
                self.draft = None;
                ui.label("Selection is out of date; choose a snippet again.");
                return;
            }
            if self.draft.is_none() {
                self.draft = Some(Draft::from_expansion(&self.config.expansion[index]));
            }
            let command_backed = self.config.expansion[index].command.is_some();
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                theme::section_header(ui, "✏", "Snippet details");
                ui.add_space(6.0);
                let Some(draft) = self.draft.as_mut() else {
                    ui.label("Snippet draft unavailable; choose a snippet again.");
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label("Trigger");
                    ui.add(
                        TextEdit::singleline(&mut draft.trigger)
                            .hint_text(";;hello")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(300.0),
                    );
                });
                ui.label(
                    RichText::new("Tip: use a distinctive prefix such as ;; or : to avoid accidental matches.")
                        .small()
                        .color(palette.muted),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Description");
                    ui.add(TextEdit::singleline(&mut draft.description).desired_width(420.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Tags");
                    ui.add(
                        TextEdit::singleline(&mut draft.tags)
                            .hint_text("email, support, ops")
                            .desired_width(420.0),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut draft.enabled, "Enabled");
                    ui.separator();
                    ui.radio_value(&mut draft.match_mode, MatchMode::Immediate, "Immediate");
                    ui.radio_value(
                        &mut draft.match_mode,
                        MatchMode::WordBoundary,
                        "Word boundary",
                    );
                });
                ui.add_space(4.0);
                ui.label("Replacement");
                ui.add(
                    TextEdit::multiline(&mut draft.replacement)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(9)
                        .desired_width(f32::INFINITY),
                );
            });
            if command_backed {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "This snippet is command-backed; replacement is stored fallback text.",
                    )
                    .italics()
                    .color(palette.muted),
                );
            }
            ui.add_space(10.0);
            ui.collapsing("🔣 Template variables", |ui| {
                ui.label(
                    RichText::new("Insert a safe built-in value into the replacement.")
                        .small()
                        .color(palette.muted),
                );
                ui.horizontal_wrapped(|ui| {
                    for (variable, description) in TEMPLATE_VARIABLES {
                        if ui.button(*variable).on_hover_text(*description).clicked() {
                            if let Some(draft) = self.draft.as_mut() {
                                draft.replacement.push_str(variable);
                            }
                        }
                    }
                });
            });
            ui.add_space(4.0);
            ui.collapsing("🛠 Dynamic command (optional)", |ui| {
                let Some(draft) = self.draft.as_mut() else {
                    ui.label("Snippet draft unavailable; choose a snippet again.");
                    return;
                };
                ui.checkbox(
                    &mut draft.command_enabled,
                    "Run a direct program when this snippet matches",
                );
                ui.label(
                    RichText::new(
                        "Only the configured executable is run; shell syntax is never interpreted. "
                            .to_owned()
                            + "Arguments are entered one per line.",
                    )
                    .small()
                    .color(palette.muted),
                );
                if draft.command_enabled {
                    egui::Frame::new()
                        .fill(theme::tint(palette.warning, 30))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.colored_label(
                                palette.warning,
                                "⚠ Advanced: this runs a local executable when the trigger matches.",
                            );
                        });
                }
                ui.add_enabled_ui(draft.command_enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Program");
                        ui.add(
                            TextEdit::singleline(&mut draft.command_program)
                                .hint_text("uname")
                                .desired_width(300.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Timeout ms");
                        ui.add(
                            TextEdit::singleline(&mut draft.command_timeout_ms).desired_width(90.0),
                        );
                        ui.label("Cache ms");
                        ui.add(
                            TextEdit::singleline(&mut draft.command_cache_ms).desired_width(90.0),
                        );
                    });
                    ui.label("Arguments (one per line)");
                    ui.add(
                        TextEdit::multiline(&mut draft.command_args)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if theme::primary_button(ui, &palette, "💾 Save changes").clicked() {
                    self.save_selected();
                }
                if theme::danger_button(ui, &palette, "🗑 Delete…").clicked() {
                    self.request_action(PendingAction::Delete);
                }
            });
            ui.add_space(14.0);
            theme::section_header(ui, "▶", "Preview");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Input");
                ui.add(
                    TextEdit::singleline(&mut self.preview_input)
                        .hint_text("text containing the trigger")
                        .desired_width(420.0),
                );
                if ui.button("Use trigger").clicked() {
                    self.preview_input = self
                        .draft
                        .as_ref()
                        .map(|draft| draft.trigger.clone())
                        .unwrap_or_default();
                }
            });
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(if self.dark_mode {
                    Color32::from_rgb(0x0F, 0x11, 0x15)
                } else {
                    Color32::from_rgb(0xFB, 0xFB, 0xFC)
                })
                .stroke(egui::Stroke::new(1.0, palette.accent))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.label(RichText::new(self.preview()).monospace());
                });
            ui.add_space(12.0);
            let (message_color, message_bg) = status_tone(&self.message, &palette);
            egui::Frame::new()
                .fill(message_bg)
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.label(RichText::new(&self.message).color(message_color));
                });
        });
        if self.pending_action.is_some() {
            egui::Window::new("⚠  Unsaved changes")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    let action = match self.pending_action {
                        Some(PendingAction::Select(_)) => "switching snippets",
                        Some(PendingAction::New) => "creating a snippet",
                        Some(PendingAction::Duplicate) => "duplicating a snippet",
                        Some(PendingAction::Delete) => "deleting a snippet",
                        Some(PendingAction::Reload) => "reloading the configuration",
                        None => "continuing",
                    };
                    if self.draft_is_dirty() {
                        ui.label(format!("Save changes before {action}?"));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::primary_button(ui, &palette, "Save and continue").clicked() {
                                self.save_and_execute_pending();
                            }
                            if ui.button("Discard").clicked() {
                                self.discard_pending();
                            }
                            if ui.button("Cancel").clicked() {
                                self.pending_action = None;
                            }
                        });
                    } else if matches!(self.pending_action, Some(PendingAction::Delete)) {
                        ui.label(
                            "Delete this snippet? This cannot be recovered except through Undo.",
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::danger_button(ui, &palette, "Delete snippet").clicked() {
                                self.pending_action = None;
                                self.execute_action(PendingAction::Delete);
                            }
                            if ui.button("Cancel").clicked() {
                                self.pending_action = None;
                            }
                        });
                    }
                });
        }
    }
}

fn control_command(command: &str) -> Result<String> {
    let path = env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        })
        .context("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required")?;
    let mut stream =
        UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))?;
    stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
    writeln!(stream, "{command}")?;
    let mut response = Vec::new();
    stream
        .take((MAX_CONTROL_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)?;
    if response.len() > MAX_CONTROL_RESPONSE_BYTES {
        anyhow::bail!("daemon control response exceeded {MAX_CONTROL_RESPONSE_BYTES} bytes");
    }
    String::from_utf8(response).context("daemon returned a non-UTF-8 control response")
}

/// Colors the status bar from the free-form message text set throughout this
/// file (e.g. "Snippet saved atomically", "Save failed: ..."). Wording
/// changes to a message stay in whatever function sets it; this only needs
/// to recognize the handful of words those messages already consistently
/// use for success versus failure.
fn status_tone(message: &str, palette: &Palette) -> (Color32, Color32) {
    let lower = message.to_lowercase();
    let is_failure = ["failed", "invalid", "rejected", "unavailable", "error"]
        .iter()
        .any(|word| lower.contains(word));
    let is_success = !is_failure
        && [
            "saved", "created", "duplicated", "reloaded", "imported", "deleted", "undid",
            "paused", "resumed", "refreshed",
        ]
        .iter()
        .any(|word| lower.contains(word));
    if is_failure {
        (palette.danger, theme::tint(palette.danger, 26))
    } else if is_success {
        (palette.success, theme::tint(palette.success, 26))
    } else {
        (palette.muted, Color32::TRANSPARENT)
    }
}

fn expand_user_path(value: &str) -> PathBuf {
    let Some(home) = env::var_os("HOME") else {
        return PathBuf::from(value);
    };
    if value == "~" {
        PathBuf::from(home)
    } else if let Some(remainder) = value.strip_prefix("~/") {
        PathBuf::from(home).join(remainder)
    } else {
        PathBuf::from(value)
    }
}

fn main() -> Result<()> {
    if matches!(env::args().nth(1).as_deref(), Some("--help" | "-h")) {
        println!("Usage: wayexpand-gui [CONFIG]\n\nNative Wayland settings editor for WayExpand.");
        return Ok(());
    }
    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let mut app = GuiApp::load(path)?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "WayExpand",
        options,
        Box::new(|creation_context| {
            theme::install(&creation_context.egui_ctx);
            app.dark_mode = creation_context.egui_ctx.theme() == egui::Theme::Dark;
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("GUI failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn draft() -> Draft {
        Draft {
            trigger: ":cmd".into(),
            description: String::new(),
            tags: String::new(),
            replacement: "fallback".into(),
            enabled: true,
            match_mode: MatchMode::Immediate,
            command_enabled: true,
            command_program: "uname".into(),
            command_args: "-s\n-r\n".into(),
            command_timeout_ms: "500".into(),
            command_cache_ms: "1000".into(),
        }
    }

    #[test]
    fn command_editor_builds_direct_program_configuration() {
        let command = draft().command_config().unwrap().unwrap();
        assert_eq!(command.program, "uname");
        assert_eq!(command.args, ["-s", "-r"]);
        assert_eq!(command.timeout_ms, 500);
        assert_eq!(command.cache_ms, 1000);
    }

    #[test]
    fn command_editor_rejects_non_numeric_limits() {
        let mut draft = draft();
        draft.command_timeout_ms = "half a second".into();
        let error = draft.command_config().unwrap_err().to_string();
        assert!(error.contains("timeout must be an integer"));
    }

    #[test]
    fn disabled_command_editor_removes_command() {
        let mut draft = draft();
        draft.command_enabled = false;
        assert!(draft.command_config().unwrap().is_none());
    }

    #[test]
    fn missing_configuration_is_initialized_without_replacing_existing_files() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-first-run-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let app = GuiApp::load(path.clone()).unwrap();
        assert!(app.config.expansion.is_empty());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn switching_snippets_preserves_unsaved_draft_until_decision() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-gui-dirty-{}.toml", std::process::id()));
        let config = Config {
            expansion: vec![
                ExpansionConfig {
                    trigger: ":one".into(),
                    replacement: "one".into(),
                    description: String::new(),
                    tags: Vec::new(),
                    match_mode: MatchMode::Immediate,
                    command: None,
                    enabled: true,
                },
                ExpansionConfig {
                    trigger: ":two".into(),
                    replacement: "two".into(),
                    description: String::new(),
                    tags: Vec::new(),
                    match_mode: MatchMode::Immediate,
                    command: None,
                    enabled: true,
                },
            ],
            hotkey: Vec::new(),
            settings: Settings::default(),
        };
        let _ = fs::remove_file(&path);
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        app.draft.as_mut().unwrap().replacement = "changed".into();
        app.request_action(PendingAction::Select(1));
        assert_eq!(app.selected, Some(0));
        assert!(matches!(app.pending_action, Some(PendingAction::Select(1))));
        app.discard_pending();
        assert_eq!(app.selected, Some(1));
        assert!(app.pending_action.is_none());
        app.duplicate_selected();
        assert_eq!(app.config.expansion.len(), 3);
        assert_eq!(app.config.expansion[2].trigger, ":two-copy");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn import_path_expands_home_prefix_without_shell_evaluation() {
        std::env::set_var("HOME", "/tmp/wayexpand-home");
        assert_eq!(
            expand_user_path("~/matches.yml"),
            PathBuf::from("/tmp/wayexpand-home/matches.yml")
        );
        assert_eq!(
            expand_user_path("/tmp/matches.yml"),
            PathBuf::from("/tmp/matches.yml")
        );
    }

    #[test]
    fn undo_history_is_bounded() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-gui-undo-{}.toml", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        for _ in 0..(MAX_UNDO_HISTORY + 8) {
            app.remember_undo(Config {
                expansion: Vec::new(),
                hotkey: Vec::new(),
                settings: Settings::default(),
            });
        }
        assert_eq!(app.undo.len(), MAX_UNDO_HISTORY);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn settings_editor_rejects_out_of_range_buffer_limit() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-settings-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        app.settings_buffer = "0".into();
        app.save_settings();
        assert_eq!(app.config.settings.max_buffer_chars, 128);
        assert!(app.message.contains("outside the allowed range"));
        fs::remove_file(path).unwrap();
    }
}
