mod theme;
mod lang;
mod colorpack;

use anyhow::{Context, Result};
use lang::{Language, Strings};
use colorpack::ColorPack;
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
    CommandConfig, Config, ConfigError, ExpansionConfig, ExpansionEngine, FontScale, InputEvent, MatchMode,
    Settings,
};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTROL_RESPONSE_BYTES: usize = 4096;
const MAX_UNDO_HISTORY: usize = 32;
const TEMPLATE_VARIABLES: &[(&str, &str)] = &[
    ("{{date}}", "UTC date"),
    ("{{time}}", "UTC time"),
    ("{{datetime}}", "UTC date and time"),
    ("{{date+1d}}", "tomorrow's date (also: -1d, +1w, date/time/datetime, d/w/h/m units)"),
    ("{{cursor}}", "place the cursor here after expanding (supported on the libei and wlroots backends)"),
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
    category: String,
    app_filter: String,
    replacement: String,
    enabled: bool,
    match_mode: MatchMode,
    propagate_case: bool,
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
    Undo,
}

struct GuiApp {
    path: PathBuf,
    config: Config,
    selected: Option<usize>,
    filter: String,
    category_filter: Option<String>,
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
    settings_undo_chord: String,
    settings_font_scale: FontScale,
    settings_error: Option<String>,
    dark_mode: bool,
    language: Language,
    strings: Strings,
    language_selector_open: bool,
    colorpack: ColorPack,
    colorpack_selector_open: bool,
    /// Cached result of an explicit, user-triggered "Run once" command
    /// preview (`Ok` output or a `Err` message to display). `None` means no
    /// run has happened yet for the current draft. Cleared on selection
    /// change so a stale result from a different snippet is never shown.
    command_preview_result: Option<Result<String, String>>,
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
        let settings_undo_chord = config.settings.undo_chord.clone().unwrap_or_default();
        let settings_font_scale = config.settings.font_scale;
        let prefs = load_gui_prefs();
        let strings = Strings::new(prefs.language);
        Ok(Self {
            path,
            config,
            selected,
            filter: String::new(),
            category_filter: None,
            preview_input,
            draft,
            undo: Vec::new(),
            message: strings.ready().into(),
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
            settings_undo_chord,
            settings_font_scale,
            settings_error: None,
            dark_mode: prefs.dark_mode.unwrap_or(true),
            language: prefs.language,
            strings,
            language_selector_open: false,
            colorpack: prefs.colorpack,
            colorpack_selector_open: false,
            command_preview_result: None,
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
        candidate.settings.font_scale = self.settings_font_scale;
        let undo_chord_input = self.settings_undo_chord.trim();
        candidate.settings.undo_chord = (!undo_chord_input.is_empty())
            .then(|| undo_chord_input.to_owned());
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
                self.settings_undo_chord =
                    self.config.settings.undo_chord.clone().unwrap_or_default();
                self.settings_font_scale = self.config.settings.font_scale;
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
                self.category_filter
                    .as_deref()
                    .is_none_or(|category| expansion.category == category)
            })
            .filter(|(_, expansion)| {
                query.is_empty()
                    || format!(
                        "{} {} {} {}",
                        expansion.trigger,
                        expansion.description,
                        expansion.tags.join(" "),
                        expansion.category
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Distinct, sorted, non-empty categories currently in use — drives the
    /// sidebar filter chips and the editor's "pick existing" combo box.
    fn categories(&self) -> Vec<String> {
        self.config
            .expansion
            .iter()
            .map(|expansion| expansion.category.clone())
            .filter(|category| !category.is_empty())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.draft = Some(Draft::from_expansion(&self.config.expansion[index]));
        self.preview_input = self.config.expansion[index].trigger.clone();
        self.pending_action = None;
        self.command_preview_result = None;
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
            || draft.category != expansion.category
            || draft.app_filter
                != expansion
                    .app_filter
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            || draft.replacement != expansion.replacement
            || draft.enabled != expansion.enabled
            || draft.match_mode != expansion.match_mode
            || draft.propagate_case != expansion.propagate_case
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
            PendingAction::Undo => self.undo(),
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
            self.message = self.strings.no_selection().into();
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
        candidate.expansion[index].category = draft.category.clone();
        candidate.expansion[index].app_filter = draft
            .app_filter
            .split(',')
            .map(str::trim)
            .filter(|filter| !filter.is_empty())
            .map(str::to_owned)
            .collect();
        candidate.expansion[index].replacement = draft.replacement.clone();
        candidate.expansion[index].enabled = draft.enabled;
        candidate.expansion[index].match_mode = draft.match_mode;
        candidate.expansion[index].propagate_case = draft.propagate_case;
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
                self.command_preview_result = None;
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
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
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
            self.message = self.strings.no_selection().into();
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
            self.message = self.strings.no_selection().into();
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

    /// Flips a snippet's enabled flag directly from the sidebar dot and
    /// saves immediately, independent of selection or any in-progress
    /// unsaved draft. If the toggled row is the one currently being edited,
    /// only its `enabled` field is synced so other unsaved edits survive.
    fn toggle_enabled(&mut self, index: usize) {
        let mut candidate = self.config.clone();
        candidate.expansion[index].enabled = !candidate.expansion[index].enabled;
        let now_enabled = candidate.expansion[index].enabled;
        let trigger = candidate.expansion[index].trigger.clone();
        match candidate.save_atomic(&self.path) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.remember_undo(previous);
                if self.selected == Some(index) {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.enabled = now_enabled;
                    }
                }
                self.message = format!(
                    "{trigger} {}",
                    if now_enabled { "enabled" } else { "disabled" }
                );
                let _ = control_command("reload");
            }
            Err(error) => self.message = format!("Toggle failed: {}", error.safe_summary()),
        }
    }

    /// Renders a live preview for a plain (non-command) draft. Must never be
    /// called for a command-backed draft: it builds a real `ExpansionEngine`
    /// and calls it on every repaint, and for a command-backed expansion
    /// that would mean spawning the configured program continuously while
    /// the editor is simply open -- including any side-effecting script the
    /// user has not even saved yet. Command previews are explicit and
    /// user-triggered instead; see `run_command_preview`.
    fn preview(&self) -> String {
        let Some(index) = self.selected else {
            return self.strings.no_selection().into();
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

    /// Runs the draft's configured command exactly once, on explicit user
    /// request (a button click), and caches the result for display. This is
    /// the only place a command-backed draft's program should ever run
    /// before it is saved.
    fn run_command_preview(&mut self) {
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        let result = match draft.command_config() {
            Ok(Some(command)) => wayexpand_core::run_command(&command)
                .map_err(|error| format!("Command failed: {error}")),
            Ok(None) => Err("Enable the dynamic command first".into()),
            Err(error) => Err(format!("Command settings invalid: {error}")),
        };
        self.command_preview_result = Some(result);
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
            category: expansion.category.clone(),
            app_filter: expansion.app_filter.join(", "),
            replacement: expansion.replacement.clone(),
            enabled: expansion.enabled,
            match_mode: expansion.match_mode,
            propagate_case: expansion.propagate_case,
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
        let palette = Palette::for_pack(self.colorpack, self.dark_mode);
        let modal_open = self.diagnostics_open
            || self.import_open
            || self.settings_open
            || self.pending_action.is_some();
        let (want_save, want_new, want_escape) = ui.ctx().input(|input| {
            (
                !modal_open && input.modifiers.command && input.key_pressed(egui::Key::S),
                !modal_open && input.modifiers.command && input.key_pressed(egui::Key::N),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if want_save && self.selected.is_some() {
            self.save_selected();
        }
        if want_new {
            self.request_action(PendingAction::New);
        }
        if want_escape {
            if self.diagnostics_open {
                self.diagnostics_open = false;
            } else if self.import_open && self.import_preview.is_none() {
                self.import_open = false;
            }
        }
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
                    ui.label(RichText::new(self.strings.title()).color(palette.muted));
                    ui.add_space(6.0);
                    theme::pill(
                        ui,
                        self.strings.snippets_count(self.config.expansion.len()),
                        palette.muted,
                        palette.surface_hover,
                    );
                    if !self.config.hotkey.is_empty() {
                        theme::pill(
                            ui,
                            self.strings.hotkeys_count(self.config.hotkey.len()),
                            palette.muted,
                            palette.surface_hover,
                        );
                    }
                    if self.draft_is_dirty() {
                        theme::pill(
                            ui,
                            self.strings.unsaved_changes(),
                            palette.warning,
                            theme::tint(palette.warning, 38),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(if self.dark_mode { "☀" } else { "🌙" })
                            .on_hover_text(self.strings.toggle_theme())
                            .clicked()
                        {
                            self.dark_mode = !self.dark_mode;
                            ui.ctx().set_theme(if self.dark_mode {
                                egui::ThemePreference::Dark
                            } else {
                                egui::ThemePreference::Light
                            });
                            save_gui_prefs(self.language, self.colorpack, self.dark_mode);
                        }
                        if ui.button("🌐 EN/DE").on_hover_text("Switch language").clicked() {
                            self.language_selector_open = !self.language_selector_open;
                        }
                        if ui.button("🎨 Theme").on_hover_text("Switch color pack").clicked() {
                            self.colorpack_selector_open = !self.colorpack_selector_open;
                        }
                        if ui.button(self.strings.settings()).clicked() {
                            self.settings_buffer =
                                self.config.settings.max_buffer_chars.to_string();
                            self.settings_undo_chord =
                                self.config.settings.undo_chord.clone().unwrap_or_default();
                            self.settings_font_scale = self.config.settings.font_scale;
                            self.settings_error = None;
                            self.settings_open = true;
                        }
                        if ui.button(self.strings.import_espanso()).clicked() {
                            self.import_open = true;
                            self.import_preview = None;
                        }
                        if ui.button(self.strings.diagnostics()).clicked() {
                            self.diagnostics_open = true;
                            self.refresh_diagnostics();
                        }
                        if ui
                            .button(if self.paused {
                                self.strings.resume()
                            } else {
                                self.strings.pause()
                            })
                            .clicked()
                        {
                            self.toggle_pause();
                        }
                        if ui.button(self.strings.reload()).clicked() {
                            self.request_action(PendingAction::Reload);
                        }
                        ui.add(
                            TextEdit::singleline(&mut self.filter)
                                .hint_text(self.strings.search_placeholder())
                                .desired_width(220.0),
                        );
                    });
                });
            });
        if self.diagnostics_open {
            let mut open = self.diagnostics_open;
            egui::Window::new(self.strings.diagnostics_title())
                .open(&mut open)
                .resizable(true)
                .min_width(420.0)
                .show(ui.ctx(), |ui| {
                    theme::section_header(ui, "🖥", self.strings.runtime_health());
                    ui.add_space(4.0);
                    ui.label(RichText::new(self.strings.daemon()).color(palette.muted).small());
                    egui::Frame::group(ui.style())
                        .fill(palette.surface_hover)
                        .show(ui, |ui| {
                            ui.label(RichText::new(&self.daemon_status).monospace());
                        });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        theme::section_header(ui, "🔌", self.strings.backends());
                        if ui.small_button(self.strings.refresh()).clicked() {
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
                    theme::section_header(ui, "📡", self.strings.protocol_probes());
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
            egui::Window::new(self.strings.import_dialog_title())
                .open(&mut open)
                .resizable(false)
                .min_width(420.0)
                .show(ui.ctx(), |ui| {
                    ui.label(self.strings.source_yaml());
                    ui.add(
                        TextEdit::singleline(&mut self.import_path)
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
                        if theme::primary_button(ui, &palette, self.strings.load_preview()).clicked() {
                            self.preview_import();
                        }
                        if ui.button(self.strings.cancel()).clicked() {
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
                        if theme::primary_button(ui, &palette, self.strings.replace_library()).clicked()
                        {
                            self.apply_import();
                        }
                    }
                });
            self.import_open = open && self.import_open;
        }
        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new(self.strings.settings_title())
                .open(&mut open)
                .resizable(false)
                .min_width(360.0)
                .show(ui.ctx(), |ui| {
                    ui.label(self.strings.buffer_limit());
                    ui.add(TextEdit::singleline(&mut self.settings_buffer).desired_width(120.0));
                    ui.label(
                        RichText::new(self.strings.buffer_limit_help())
                            .small()
                            .color(palette.muted),
                    );

                    ui.add_space(8.0);
                    ui.label(self.strings.undo_chord());
                    ui.add(
                        TextEdit::singleline(&mut self.settings_undo_chord)
                            .hint_text("Ctrl+Z")
                            .desired_width(120.0),
                    );
                    ui.label(
                        RichText::new(self.strings.undo_chord_help())
                            .small()
                            .color(palette.muted),
                    );

                    ui.add_space(8.0);
                    ui.label("🔤 Font Size");
                    ui.horizontal(|ui| {
                        for scale in &[FontScale::Small, FontScale::Normal, FontScale::Large, FontScale::ExtraLarge, FontScale::Huge] {
                            let label = match scale {
                                FontScale::Small => "Small (80%)",
                                FontScale::Normal => "Normal",
                                FontScale::Large => "Large (120%)",
                                FontScale::ExtraLarge => "Extra Large (150%)",
                                FontScale::Huge => "Huge (200%)",
                            };
                            if ui.selectable_label(*scale == self.settings_font_scale, label).clicked() {
                                self.settings_font_scale = *scale;
                            }
                        }
                    });
                    ui.label(
                        RichText::new("Adjust text size for readability on your display")
                            .small()
                            .color(palette.muted),
                    );

                    if let Some(error) = &self.settings_error {
                        ui.add_space(4.0);
                        ui.colored_label(palette.danger, format!("⚠ {error}"));
                    }

                    ui.add_space(12.0);
                    theme::section_header(ui, "🖥", self.strings.backend_status());
                    ui.add_space(6.0);

                    for status in &self.backend_status {
                        use wayexpand_core::BackendState;
                        let status_color = match status.state {
                            BackendState::Available | BackendState::Implemented => palette.success,
                            _ => palette.muted,
                        };
                        let state_text = format!("{:?}", status.state);
                        ui.horizontal(|ui| {
                            ui.colored_label(status_color, "●");
                            ui.label(format!("{:?}: {}", status.kind, state_text));
                        });
                    }

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, &palette, self.strings.save_settings()).clicked() {
                            self.save_settings();
                        }
                        if ui.button(self.strings.close()).clicked() {
                            self.settings_open = false;
                        }
                    });
                });
            self.settings_open = open && self.settings_open;
        }
        if self.language_selector_open {
            let mut open = self.language_selector_open;
            egui::Window::new("🌐 Language / Sprache")
                .open(&mut open)
                .resizable(false)
                .min_width(200.0)
                .show(ui.ctx(), |ui| {
                    ui.label("Choose your language:");
                    ui.add_space(6.0);
                    if ui.selectable_label(self.language == Language::English, "English").clicked() {
                        self.language = Language::English;
                        self.strings.set_language(Language::English);
                        save_gui_prefs(self.language, self.colorpack, self.dark_mode);
                    }
                    if ui.selectable_label(self.language == Language::German, "Deutsch").clicked() {
                        self.language = Language::German;
                        self.strings.set_language(Language::German);
                        save_gui_prefs(self.language, self.colorpack, self.dark_mode);
                    }
                    ui.add_space(6.0);
                    if ui.button(self.strings.close()).clicked() {
                        self.language_selector_open = false;
                    }
                });
            self.language_selector_open = open && self.language_selector_open;
        }
        if self.colorpack_selector_open {
            let mut open = self.colorpack_selector_open;
            egui::Window::new("🎨 Color Pack / Farbschema")
                .open(&mut open)
                .resizable(true)
                .min_width(340.0)
                .show(ui.ctx(), |ui| {
                    ui.label("Choose your color scheme:");
                    ui.add_space(6.0);
                    ui.separator();
                    for pack in ColorPack::all() {
                        let selected = self.colorpack == *pack;
                        if ui.selectable_label(selected, format!("{}  —  {}", pack.name(), pack.description())).clicked() {
                            self.colorpack = *pack;
                            theme::install_pack(ui.ctx(), *pack, self.config.settings.font_scale);
                            save_gui_prefs(self.language, self.colorpack, self.dark_mode);
                        }
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    ui.label("Retro PC themes:");
                    ui.label("  🟢 Classic Green — VT220 CRT terminal glow");
                    ui.label("  🟠 Classic Amber — Vintage Apple monitor");
                    ui.label("  ⚪ Classic White — Monochrome classic");
                    ui.add_space(6.0);
                    if ui.button(self.strings.close()).clicked() {
                        self.colorpack_selector_open = false;
                    }
                });
            self.colorpack_selector_open = open && self.colorpack_selector_open;
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
                        self.strings.your_library()
                    } else {
                        self.strings.filtered_snippets()
                    })
                    .small()
                    .color(palette.muted),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, &palette, self.strings.new_button())
                        .on_hover_text(self.strings.new_tooltip())
                        .clicked()
                    {
                        self.request_action(PendingAction::New);
                    }
                    if ui.button(self.strings.duplicate()).clicked() {
                        self.request_action(PendingAction::Duplicate);
                    }
                    if ui.button(self.strings.undo_button(self.undo.len())).clicked() {
                        self.request_action(PendingAction::Undo);
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(4.0);
                let font_scale = self.config.settings.font_scale.multiplier();
                let categories = self.categories();
                if !categories.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        if theme::chip_scaled(
                            ui,
                            &palette,
                            self.strings.all(),
                            self.category_filter.is_none(),
                            font_scale,
                        )
                        .clicked()
                        {
                            self.category_filter = None;
                        }
                        for category in &categories {
                            let selected =
                                self.category_filter.as_deref() == Some(category.as_str());
                            if theme::chip_scaled(ui, &palette, category, selected, font_scale).clicked() {
                                self.category_filter = if selected {
                                    None
                                } else {
                                    Some(category.clone())
                                };
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
                let visible_indices = self.visible_indices();
                ScrollArea::vertical().show(ui, |ui| {
                    for index in visible_indices {
                        let expansion = &self.config.expansion[index];
                        let detail = if expansion.command.is_some() {
                            "Command-backed snippet".to_owned()
                        } else {
                            expansion.description.clone()
                        };
                        let response = theme::snippet_row_scaled(
                            ui,
                            &palette,
                            theme::SnippetRow {
                                selected: self.selected == Some(index),
                                enabled: expansion.enabled,
                                command_backed: expansion.command.is_some(),
                                trigger: &expansion.trigger,
                                detail: &detail,
                                category: &expansion.category,
                            },
                            font_scale,
                        );
                        if response.toggle.clicked() {
                            self.toggle_enabled(index);
                        } else if response.row.clicked() {
                            self.request_action(PendingAction::Select(index));
                        }
                    }
                    if self.config.expansion.is_empty() {
                        ui.add_space(16.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("📭").size(28.0));
                            ui.label(RichText::new(self.strings.no_snippets()).color(palette.muted));
                            ui.add_space(6.0);
                            if theme::primary_button(ui, &palette, self.strings.create_first())
                                .clicked()
                            {
                                self.request_action(PendingAction::New);
                            }
                        });
                    } else if self.visible_indices().is_empty() {
                        ui.add_space(16.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("🔍").size(28.0));
                            let reason = match (self.filter.is_empty(), &self.category_filter) {
                                (false, Some(category)) => {
                                    self.strings.no_matches_category(&self.filter, category)
                                }
                                (false, None) => self.strings.no_matches_filter(&self.filter),
                                (true, Some(category)) => {
                                    self.strings.no_snippets_category(category)
                                }
                                (true, None) => "No snippets match this filter.".to_owned(),
                            };
                            ui.label(RichText::new(reason).color(palette.muted));
                            ui.add_space(6.0);
                            if ui.button(self.strings.clear_filters()).clicked() {
                                self.filter.clear();
                                self.category_filter = None;
                            }
                        });
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.background)
                    .inner_margin(egui::Margin::symmetric(22, 18)),
            )
            .show(ui, |ui| {
            let Some(index) = self.selected else {
                ui.vertical_centered(|ui| {
                    ui.add_space(70.0);
                    ui.label(RichText::new("✨").size(40.0));
                    ui.add_space(6.0);
                    ui.heading(self.strings.build_first());
                    ui.label(
                        RichText::new(self.strings.build_description())
                            .color(palette.muted),
                    );
                    ui.add_space(10.0);
                    if theme::primary_button(ui, &palette, self.strings.create_snippet()).clicked() {
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
            let mut detect_app_clicked = false;
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("editor_scroll")
                .show(ui, |ui| {
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                theme::section_header(ui, "✏", self.strings.snippet_details());
                ui.add_space(6.0);
                let categories = self.categories();
                let Some(draft) = self.draft.as_mut() else {
                    ui.label("Snippet draft unavailable; choose a snippet again.");
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(self.strings.trigger());
                    ui.add(
                        TextEdit::singleline(&mut draft.trigger)
                            .hint_text(self.strings.trigger_hint())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(300.0),
                    );
                });
                let duplicate_trigger = !draft.trigger.is_empty()
                    && self
                        .config
                        .expansion
                        .iter()
                        .enumerate()
                        .any(|(other_index, other)| {
                            other_index != index && other.trigger == draft.trigger
                        });
                if duplicate_trigger {
                    ui.colored_label(
                        palette.danger,
                        self.strings.duplicate_trigger(),
                    );
                } else {
                    ui.label(
                        RichText::new(self.strings.trigger_tip())
                            .small()
                            .color(palette.muted),
                    );
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(self.strings.description());
                    ui.add(TextEdit::singleline(&mut draft.description).desired_width(420.0));
                });
                ui.horizontal(|ui| {
                    ui.label(self.strings.tags());
                    ui.add(
                        TextEdit::singleline(&mut draft.tags)
                            .hint_text(self.strings.tags_hint())
                            .desired_width(420.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(self.strings.category());
                    ui.add(
                        TextEdit::singleline(&mut draft.category)
                            .hint_text(self.strings.category_hint())
                            .desired_width(260.0),
                    );
                    if !categories.is_empty() {
                        egui::ComboBox::from_id_salt("category_picker")
                            .selected_text(self.strings.existing())
                            .width(140.0)
                            .show_ui(ui, |ui| {
                                for category in &categories {
                                    if ui
                                        .selectable_label(draft.category == *category, category)
                                        .clicked()
                                    {
                                        draft.category = category.clone();
                                    }
                                }
                            });
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(self.strings.app_filter());
                    ui.add(
                        TextEdit::singleline(&mut draft.app_filter)
                            .hint_text(self.strings.app_filter_hint())
                            .desired_width(300.0),
                    );
                    if ui
                        .button(self.strings.detect_app())
                        .on_hover_text(self.strings.detect_app_tooltip())
                        .clicked()
                    {
                        detect_app_clicked = true;
                    }
                });
                if !draft.app_filter.trim().is_empty() {
                    ui.label(
                        RichText::new(self.strings.window_tracking_warning())
                        .small()
                        .color(palette.muted),
                    );
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut draft.enabled, self.strings.enabled());
                    ui.separator();
                    ui.radio_value(&mut draft.match_mode, MatchMode::Immediate, self.strings.immediate());
                    ui.radio_value(
                        &mut draft.match_mode,
                        MatchMode::WordBoundary,
                        self.strings.word_boundary(),
                    );
                    ui.separator();
                    ui.checkbox(&mut draft.propagate_case, self.strings.propagate_case())
                        .on_hover_text(self.strings.propagate_case_tooltip());
                });
                ui.add_space(4.0);
                ui.label(self.strings.replacement());
                ui.add(
                    TextEdit::multiline(&mut draft.replacement)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(9)
                        .desired_width(f32::INFINITY),
                );
            });
            if detect_app_clicked {
                // Bounded so a KWin version mismatch, a D-Bus hiccup, or any
                // other reason the tracker's script never calls back cannot
                // freeze the GUI: this runs synchronously on the UI thread.
                use wayexpand_backend_kwin_window::KwinWindowTracker;
                use wayexpand_core::{WindowContext, WindowTracker};
                enum Detection {
                    Found(WindowContext),
                    NoWindow,
                    Unavailable,
                }
                let detection = match KwinWindowTracker::new() {
                    Ok(mut tracker) => {
                        match tracker.next_window_timeout(std::time::Duration::from_secs(5)) {
                            Ok(Some(Some(window))) => Detection::Found(window),
                            Ok(Some(None)) => Detection::NoWindow,
                            Ok(None) | Err(_) => Detection::Unavailable,
                        }
                    }
                    Err(_) => Detection::Unavailable,
                };
                match detection {
                    Detection::Found(window) => {
                        let value = window.app_id.or(window.title).unwrap_or_default();
                        if value.is_empty() {
                            self.message = "Could not identify the focused window".into();
                        } else if let Some(draft) = self.draft.as_mut() {
                            if draft.app_filter.trim().is_empty() {
                                draft.app_filter = value.clone();
                            } else {
                                draft.app_filter.push_str(", ");
                                draft.app_filter.push_str(&value);
                            }
                            self.message = format!("Added \"{value}\" to the app filter");
                        }
                    }
                    Detection::NoWindow => {
                        self.message = "No focused window to detect (focus is on the desktop)"
                            .into();
                    }
                    Detection::Unavailable => {
                        self.message =
                            "Window detection is unavailable here (KDE Plasma only for now)"
                                .into();
                    }
                }
            }
            if command_backed {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(self.strings.command_backed_help())
                    .italics()
                    .color(palette.muted),
                );
            }
            ui.add_space(10.0);
            ui.collapsing(format!("🔣 {}", self.strings.template_variables()), |ui| {
                ui.label(
                    RichText::new(self.strings.template_help())
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
            ui.collapsing(format!("🛠 {}", self.strings.dynamic_command()), |ui| {
                let Some(draft) = self.draft.as_mut() else {
                    ui.label("Snippet draft unavailable; choose a snippet again.");
                    return;
                };
                ui.checkbox(
                    &mut draft.command_enabled,
                    self.strings.command_checkbox(),
                );
                ui.label(
                    RichText::new(self.strings.command_help())
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
                                self.strings.command_warning(),
                            );
                        });
                }
                ui.add_enabled_ui(draft.command_enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(self.strings.program());
                        ui.add(
                            TextEdit::singleline(&mut draft.command_program)
                                .hint_text(self.strings.program_hint())
                                .desired_width(300.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label(self.strings.timeout_ms());
                        ui.add(
                            TextEdit::singleline(&mut draft.command_timeout_ms).desired_width(90.0),
                        );
                        ui.label(self.strings.cache_ms());
                        ui.add(
                            TextEdit::singleline(&mut draft.command_cache_ms).desired_width(90.0),
                        );
                    });
                    ui.label(self.strings.arguments());
                    ui.add(
                        TextEdit::multiline(&mut draft.command_args)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if theme::primary_button(ui, &palette, self.strings.save_changes())
                    .on_hover_text(self.strings.save_tooltip())
                    .clicked()
                {
                    self.save_selected();
                }
                if theme::danger_button(ui, &palette, self.strings.delete()).clicked() {
                    self.request_action(PendingAction::Delete);
                }
            });
            ui.add_space(14.0);
            theme::section_header(ui, "▶", self.strings.preview());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(self.strings.input());
                ui.add(
                    TextEdit::singleline(&mut self.preview_input)
                        .hint_text(self.strings.input_hint())
                        .desired_width(420.0),
                );
                if ui.button(self.strings.use_trigger()).clicked() {
                    self.preview_input = self
                        .draft
                        .as_ref()
                        .map(|draft| draft.trigger.clone())
                        .unwrap_or_default();
                }
            });
            ui.add_space(4.0);
            let command_backed = self.draft.as_ref().is_some_and(|draft| draft.command_enabled);
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
                        ui.label(
                            "This snippet runs a program instead of inserting fixed text. \
                             Its output is not shown automatically -- run it once to see \
                             what it currently produces.",
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("▶ Run once").clicked() {
                                self.run_command_preview();
                            }
                            match &self.command_preview_result {
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
                                    self.message = "Preview copied to clipboard".into();
                                }
                            });
                        });
                    });
            }
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
        });
        if self.pending_action.is_some() {
            egui::Window::new(self.strings.unsaved_title())
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    let action = match self.pending_action {
                        Some(PendingAction::Select(_)) => self.strings.unsaved_switching(),
                        Some(PendingAction::New) => self.strings.unsaved_creating(),
                        Some(PendingAction::Duplicate) => self.strings.unsaved_duplicating(),
                        Some(PendingAction::Delete) => self.strings.unsaved_deleting(),
                        Some(PendingAction::Reload) => self.strings.unsaved_reloading(),
                        Some(PendingAction::Undo) => self.strings.unsaved_undoing(),
                        None => "continuing",
                    };
                    if self.draft_is_dirty() {
                        ui.label(self.strings.save_before(action));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::primary_button(ui, &palette, self.strings.save_continue()).clicked() {
                                self.save_and_execute_pending();
                            }
                            if ui.button(self.strings.discard()).clicked() {
                                self.discard_pending();
                            }
                            if ui.button(self.strings.cancel()).clicked() {
                                self.pending_action = None;
                            }
                        });
                    } else if matches!(self.pending_action, Some(PendingAction::Delete)) {
                        ui.label(self.strings.delete_confirm());
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::danger_button(ui, &palette, self.strings.delete_button()).clicked() {
                                self.pending_action = None;
                                self.execute_action(PendingAction::Delete);
                            }
                            if ui.button(self.strings.cancel()).clicked() {
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
            "saved",
            "created",
            "duplicated",
            "reloaded",
            "imported",
            "deleted",
            "undid",
            "enabled",
            "copied",
            "paused",
            "resumed",
            "refreshed",
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

/// GUI-only display preferences (language, color pack, dark/light mode).
/// Deliberately separate from `expansions.toml`: this file holds no
/// expansion data and carries none of that file's stability guarantees, so a
/// parse failure here should never block snippet editing -- callers fall
/// back to defaults rather than surfacing an error.
struct GuiPrefs {
    language: Language,
    colorpack: ColorPack,
    /// `None` means no preference has ever been saved: the caller should
    /// auto-detect from the desktop's theme instead of forcing one, so a
    /// first run still matches the user's system light/dark setting.
    dark_mode: Option<bool>,
}

fn gui_prefs_path() -> PathBuf {
    default_config_path().with_file_name("gui-prefs.toml")
}

fn load_gui_prefs() -> GuiPrefs {
    let mut prefs = GuiPrefs {
        language: Language::from_env(),
        colorpack: ColorPack::Default,
        dark_mode: None,
    };
    let Ok(contents) = fs::read_to_string(gui_prefs_path()) else {
        return prefs;
    };
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "language" => {
                if let Some(language) = Language::from_code(value) {
                    prefs.language = language;
                }
            }
            "colorpack" => {
                if let Some(colorpack) = ColorPack::from_code(value) {
                    prefs.colorpack = colorpack;
                }
            }
            "dark_mode" => prefs.dark_mode = Some(value == "true"),
            _ => {}
        }
    }
    prefs
}

/// Best-effort save: display preferences are not load-bearing, so a failure
/// (read-only filesystem, missing directory permissions, ...) is silently
/// ignored rather than surfaced as an error the user has to dismiss.
fn save_gui_prefs(language: Language, colorpack: ColorPack, dark_mode: bool) {
    let path = gui_prefs_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let contents = format!(
        "language = \"{}\"\ncolorpack = \"{}\"\ndark_mode = {}\n",
        language.code(),
        colorpack.code(),
        dark_mode
    );
    let _ = fs::write(path, contents);
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
    let saved_dark_mode = load_gui_prefs().dark_mode;
    let colorpack = app.colorpack;
    let icon = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/icon/hicolor/256x256/apps/wayexpand.png"
    ))
    .expect("bundled app icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "WayExpand",
        options,
        Box::new(move |creation_context| {
            theme::install_pack(&creation_context.egui_ctx, colorpack, app.config.settings.font_scale);
            // Only override with the OS-detected theme when the user has
            // never explicitly chosen one; otherwise a saved preference
            // would flip back to the system default on every launch.
            app.dark_mode = saved_dark_mode
                .unwrap_or_else(|| creation_context.egui_ctx.theme() == egui::Theme::Dark);
            creation_context.egui_ctx.set_theme(if app.dark_mode {
                egui::ThemePreference::Dark
            } else {
                egui::ThemePreference::Light
            });
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
            category: String::new(),
            app_filter: String::new(),
            replacement: "fallback".into(),
            enabled: true,
            match_mode: MatchMode::Immediate,
            propagate_case: false,
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
                    category: String::new(),
                    app_filter: Vec::new(),
                    match_mode: MatchMode::Immediate,
                    command: None,
                    enabled: true,
                    propagate_case: false,
                },
                ExpansionConfig {
                    trigger: ":two".into(),
                    replacement: "two".into(),
                    description: String::new(),
                    tags: Vec::new(),
                    category: String::new(),
                    app_filter: Vec::new(),
                    match_mode: MatchMode::Immediate,
                    command: None,
                    enabled: true,
                    propagate_case: false,
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
