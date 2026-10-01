mod colorpack;
mod diagnostics;
mod dialogs;
mod editor;
mod fonts;
mod import;
mod lang;
mod library;
mod persistence;
mod preview;
mod runtime;
mod settings;
mod status;
mod theme;

use anyhow::{Context, Result};
use colorpack::{ColorPack, ColorScheme};
use dialogs::{AppDetection, PendingAction};
use editor::Draft;
use eframe::egui::{self, Color32, RichText, ScrollArea, TextEdit};
use lang::{Language, Strings};
use settings::{load_gui_prefs, save_gui_prefs};
use status::Status;
use std::{
    env, fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use theme::Palette;
use wayexpand_core::{
    default_config_path, BackendState, BackendStatus, Config, ConfigError, ExpansionConfig,
    FontScale, MatchMode, OrganizationPolicy, Settings,
};

/// Stable source for the toolbar search field's id, so Ctrl+F can focus it.
const SEARCH_FIELD_SALT: &str = "wayexpand-search-field";
/// Every font scale, in the order the settings dialog offers them.
const FONT_SCALES: &[FontScale] = &[
    FontScale::Small,
    FontScale::Normal,
    FontScale::Large,
    FontScale::ExtraLarge,
    FontScale::Huge,
];
const MAX_UNDO_HISTORY: usize = 64;
const MAX_UNDO_BYTES: usize = 16 * 1024 * 1024;
const TEMPLATE_VARIABLES: &[(&str, &str)] = &[
    ("{{date}}", "UTC date"),
    ("{{time}}", "UTC time"),
    ("{{datetime}}", "UTC date and time"),
    (
        "{{date+1d}}",
        "tomorrow's date (also: -1d, +1w, date/time/datetime, d/w/h/m units)",
    ),
    (
        "{{cursor}}",
        "place the cursor here after expanding (supported on the libei and wlroots backends)",
    ),
    ("{{username}}", "current user"),
    ("{{hostname}}", "local hostname"),
    ("{{unix_timestamp}}", "Unix timestamp"),
    ("{{newline}}", "line break"),
    ("{{tab}}", "tab character"),
];

struct GuiApp {
    path: PathBuf,
    config_revision: wayexpand_core::ConfigRevision,
    pending_reload_revision: Option<wayexpand_core::ConfigRevision>,
    config_document: toml_edit::DocumentMut,
    config: Config,
    selected: Option<usize>,
    selected_id: Option<String>,
    filter: String,
    search_fields: library::SearchFields,
    search_index: library::SearchIndex,
    category_filter: Option<String>,
    preview_input: String,
    preview_app: String,
    draft: Option<Draft>,
    /// A new snippet is held only in the editor until a valid explicit save.
    new_draft: bool,
    new_draft_origin: Option<String>,
    undo: Vec<UndoEntry>,
    undo_bytes: usize,
    status: Status,
    paused: bool,
    daemon_reachable: Option<bool>,
    route_state: Option<runtime::RouteState>,
    diagnostics_open: bool,
    evdev_setup_open: bool,
    evdev_setup_acknowledged: bool,
    daemon_status: String,
    daemon_capabilities: Option<runtime::DaemonCapabilities>,
    fleet_status: String,
    backend_status: Vec<BackendStatus>,
    protocol_probes: Vec<(String, String)>,
    diagnostics_sender: Option<SyncSender<runtime::Request>>,
    runtime_sender: Option<SyncSender<runtime::Request>>,
    runtime_receiver: Option<mpsc::Receiver<runtime::Completion>>,
    diagnostics_running: bool,
    pending_control: usize,
    next_status_poll: Instant,
    pending_action: Option<PendingAction>,
    import_open: bool,
    import_path: String,
    import_preview: Option<(Config, wayexpand_core::EspansoImportReport)>,
    settings_open: bool,
    settings_tab: SettingsTab,
    settings_buffer: String,
    settings_undo_chord: String,
    settings_font_scale: FontScale,
    settings_error: Option<String>,
    dark_mode: bool,
    language: Language,
    strings: Strings,
    colorpack: ColorPack,
    /// Cached result of an explicit, user-triggered "Run once" command
    /// preview (`Ok` output or a `Err` message to display). `None` means no
    /// run has happened yet for the current draft. Cleared on selection
    /// change so a stale result from a different snippet is never shown.
    command_preview_result: Option<Result<String, String>>,
    command_preview_key: Option<u64>,
    /// Receiver for the currently running explicit command preview. The
    /// command itself runs off the UI thread because even a valid preview can
    /// wait for the configured command timeout.
    command_preview_receiver: Option<mpsc::Receiver<Result<String, String>>>,
    command_preview_cancel: Option<Arc<AtomicBool>>,
    /// Reload can change the persisted font scale outside the settings dialog.
    /// Apply that style on the next frame after the config has been replaced.
    theme_refresh_pending: bool,
    /// Cache of the last plain preview result. Stores (editor_revision, input,
    /// result) to avoid rebuilding the ExpansionEngine on every repaint. The
    /// revision is O(1), unlike hashing a potentially large draft.
    preview_cache: Option<(u64, String, String)>,
    /// Incremented on editor input that may change the draft or preview app.
    /// Preview cache validation therefore stays O(1) even for large snippets.
    preview_revision: u64,
    /// A background "Use current app" detection in progress. KWin setup runs
    /// off the UI thread and is bounded by per-call D-Bus timeouts plus a
    /// total script-readiness deadline; the subsequent focused-window wait is
    /// also bounded. The task remains present after UI cancellation until its
    /// worker reports completion, preventing retries from accumulating
    /// detached threads.
    app_detection: Option<AppDetectionTask>,
    /// Set by `execute_action(PendingAction::Close)` once the user has
    /// confirmed closing with an unsaved draft (or there was nothing to
    /// confirm). The original OS close request was already cancelled by
    /// then (see `ui()`), so this tells the next frame to issue a fresh
    /// one -- which will not be cancelled again since the draft is no
    /// longer dirty by that point.
    close_after_confirm: bool,
    /// Last title pushed to the compositor. The window title carries the
    /// configuration file name and an unsaved-changes marker, and
    /// `ViewportCommand::Title` is only sent when that text actually
    /// changes rather than on every frame.
    window_title: String,
}

/// The two halves of the settings window: display preferences that take
/// effect (and persist) the moment they are clicked, and configuration
/// values that are validated and written to the configuration file by an
/// explicit Save. Separating them keeps one dialog from silently mixing two
/// different commit models.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Appearance,
    Engine,
}

/// A bounded inverse operation. Snippet edits retain only the prior versions
/// of changed snippets; ordering is stored only for insert/delete/reorder
/// operations. Settings and policy sections are copied only when changed.
struct UndoEntry {
    prior_expansions: Vec<ExpansionConfig>,
    prior_order: Option<Vec<String>>,
    prior_hotkeys: Option<Vec<wayexpand_core::HotkeyConfig>>,
    prior_settings: Option<Settings>,
    prior_organization: Option<OrganizationPolicy>,
    estimated_bytes: usize,
}

impl UndoEntry {
    fn between(previous: &Config, current: &Config) -> Option<Self> {
        let current_by_id: std::collections::HashMap<_, _> = current
            .expansion
            .iter()
            .map(|expansion| (expansion.id.as_str(), expansion))
            .collect();
        let prior_expansions: Vec<_> = previous
            .expansion
            .iter()
            .filter(|expansion| current_by_id.get(expansion.id.as_str()) != Some(expansion))
            .cloned()
            .collect();
        let order_unchanged = previous
            .expansion
            .iter()
            .map(|expansion| expansion.id.as_str())
            .eq(current
                .expansion
                .iter()
                .map(|expansion| expansion.id.as_str()));
        let prior_order = (!order_unchanged).then(|| {
            previous
                .expansion
                .iter()
                .map(|expansion| expansion.id.clone())
                .collect()
        });
        let prior_hotkeys = (previous.hotkey != current.hotkey).then(|| previous.hotkey.clone());
        let prior_settings =
            (previous.settings != current.settings).then(|| previous.settings.clone());
        let prior_organization =
            (previous.organization != current.organization).then(|| previous.organization.clone());

        if prior_expansions.is_empty()
            && prior_order.is_none()
            && prior_hotkeys.is_none()
            && prior_settings.is_none()
            && prior_organization.is_none()
        {
            return None;
        }

        let mut entry = Self {
            prior_expansions,
            prior_order,
            prior_hotkeys,
            prior_settings,
            prior_organization,
            estimated_bytes: 0,
        };
        entry.estimated_bytes = entry.estimate_retained_bytes();
        Some(entry)
    }

    fn restore(&self, current: &Config) -> Result<Config, String> {
        let mut restored = current.clone();
        for previous in &self.prior_expansions {
            if let Some(expansion) = restored
                .expansion
                .iter_mut()
                .find(|expansion| expansion.id == previous.id)
            {
                *expansion = previous.clone();
            } else {
                restored.expansion.push(previous.clone());
            }
        }
        if let Some(order) = &self.prior_order {
            let by_id: std::collections::HashMap<_, _> = restored
                .expansion
                .into_iter()
                .map(|expansion| (expansion.id.clone(), expansion))
                .collect();
            restored.expansion = order
                .iter()
                .map(|id| {
                    by_id.get(id).cloned().ok_or_else(|| {
                        "undo history no longer matches the snippet library".to_owned()
                    })
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(hotkeys) = &self.prior_hotkeys {
            restored.hotkey.clone_from(hotkeys);
        }
        if let Some(settings) = &self.prior_settings {
            restored.settings.clone_from(settings);
        }
        if let Some(organization) = &self.prior_organization {
            restored.organization.clone_from(organization);
        }
        Ok(restored)
    }

    fn estimate_retained_bytes(&self) -> usize {
        fn serialized_estimate<T: serde::Serialize>(value: &T) -> usize {
            serde_json::to_vec(value)
                .map(|serialized| serialized.len())
                .unwrap_or(MAX_UNDO_BYTES.saturating_add(1))
        }

        let mut structural = std::mem::size_of::<Self>()
            .saturating_add(
                self.prior_expansions
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ExpansionConfig>()),
            )
            .saturating_add(self.prior_order.as_ref().map_or(0, |order| {
                order
                    .capacity()
                    .saturating_mul(std::mem::size_of::<String>())
            }))
            .saturating_add(self.prior_hotkeys.as_ref().map_or(0, |hotkeys| {
                hotkeys
                    .capacity()
                    .saturating_mul(std::mem::size_of::<wayexpand_core::HotkeyConfig>())
            }));
        let nested_string_count = self
            .prior_expansions
            .iter()
            .map(|expansion| {
                expansion.tags.len()
                    + expansion.app_filter.len()
                    + expansion
                        .command
                        .as_ref()
                        .map_or(0, |command| command.args.len() + command.pass_env.len())
            })
            .sum::<usize>()
            .saturating_add(self.prior_hotkeys.as_ref().map_or(0, |hotkeys| {
                hotkeys
                    .iter()
                    .map(|hotkey| hotkey.command.args.len() + hotkey.command.pass_env.len())
                    .sum()
            }))
            .saturating_add(self.prior_organization.as_ref().map_or(0, |policy| {
                policy.allowed_backends.len() + policy.allowed_packs.len()
            }));
        structural = structural
            .saturating_add(nested_string_count.saturating_mul(std::mem::size_of::<String>()));
        let payload = serialized_estimate(&self.prior_expansions)
            .saturating_add(self.prior_order.as_ref().map_or(0, serialized_estimate))
            .saturating_add(self.prior_hotkeys.as_ref().map_or(0, serialized_estimate))
            .saturating_add(self.prior_settings.as_ref().map_or(0, serialized_estimate))
            .saturating_add(
                self.prior_organization
                    .as_ref()
                    .map_or(0, serialized_estimate),
            );
        structural.saturating_add(payload)
    }
}

/// A detection worker cannot be forcefully cancelled while it is inside a
/// potentially blocking D-Bus call. Keep its receiver alive until the worker
/// reports completion so cancelling the UI action cannot allow another worker
/// to be started and leak an unbounded number of detached threads.
struct AppDetectionTask {
    receiver: mpsc::Receiver<AppDetection>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for GuiApp {
    fn drop(&mut self) {
        if let Some(cancelled) = self.command_preview_cancel.take() {
            cancelled.store(true, Ordering::Release);
        }
        if let Some(task) = self.app_detection.as_ref() {
            task.cancelled.store(true, Ordering::Release);
        }
    }
}

impl GuiApp {
    fn load(path: PathBuf) -> Result<Self> {
        let loaded = Config::ensure_user_config(&path)
            .map_err(|error| anyhow::anyhow!("configuration invalid: {}", error.safe_summary()))?;
        let config_document = persistence::read_config_document(loaded.source())?;
        let config_revision = loaded.revision.clone();
        let config = loaded.config;
        let search_index = library::SearchIndex::new(&config);
        let selected = (!config.expansion.is_empty()).then_some(0);
        let selected_id = selected.map(|index| config.expansion[index].id.clone());
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
            config_revision,
            pending_reload_revision: None,
            config_document,
            config,
            selected,
            selected_id,
            filter: String::new(),
            search_fields: library::SearchFields::default(),
            search_index,
            category_filter: None,
            preview_input,
            preview_app: String::new(),
            draft,
            new_draft: false,
            new_draft_origin: None,
            undo: Vec::new(),
            undo_bytes: 0,
            status: Status::info(strings.ready()),
            paused: false,
            daemon_connected: None,
            diagnostics_open: false,
            evdev_setup_open: false,
            evdev_setup_acknowledged: false,
            daemon_status: strings.not_checked().into(),
            daemon_capabilities: None,
            fleet_status: strings.not_checked().into(),
            backend_status: Vec::new(),
            protocol_probes: Vec::new(),
            diagnostics_sender: None,
            runtime_sender: None,
            runtime_receiver: None,
            diagnostics_running: false,
            pending_control: 0,
            next_status_poll: Instant::now() + Duration::from_secs(2),
            pending_action: None,
            import_open: false,
            import_path: String::new(),
            import_preview: None,
            settings_open: false,
            settings_tab: SettingsTab::Appearance,
            settings_buffer,
            settings_undo_chord,
            settings_font_scale,
            settings_error: None,
            dark_mode: prefs.dark_mode.unwrap_or(true),
            language: prefs.language,
            strings,
            colorpack: prefs.colorpack,
            command_preview_result: None,
            command_preview_key: None,
            command_preview_receiver: None,
            command_preview_cancel: None,
            theme_refresh_pending: false,
            preview_cache: None,
            preview_revision: 0,
            app_detection: None,
            close_after_confirm: false,
            window_title: String::new(),
        })
    }

    fn start_runtime(&mut self) -> Result<()> {
        let (control_sender, diagnostics_sender, receiver) =
            runtime::start().context("starting GUI background runtime")?;
        self.runtime_sender = Some(control_sender);
        self.diagnostics_sender = Some(diagnostics_sender);
        self.runtime_receiver = Some(receiver);
        self.refresh_diagnostics(false);
        Ok(())
    }

    fn refresh_diagnostics(&mut self, announce: bool) {
        if self.diagnostics_running {
            return;
        }
        let request = runtime::Request::Diagnostics {
            config_path: self.path.clone(),
            announce,
        };
        let Some(sender) = self.diagnostics_sender.as_ref() else {
            return;
        };
        match sender.try_send(request) {
            Ok(()) => {
                self.diagnostics_running = true;
                if announce {
                    self.status = Status::info(self.strings.diagnostics_running());
                }
            }
            Err(_) => {
                if announce {
                    self.status = Status::warning(self.strings.background_queue_full());
                }
            }
        }
    }

    fn poll_runtime(&mut self, ctx: &egui::Context) {
        let mut completions = Vec::new();
        let mut disconnected = false;
        if let Some(receiver) = self.runtime_receiver.as_ref() {
            loop {
                match receiver.try_recv() {
                    Ok(completion) => completions.push(completion),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.runtime_receiver = None;
            self.runtime_sender = None;
            self.diagnostics_sender = None;
            self.diagnostics_running = false;
            self.pending_control = 0;
            self.pending_reload_revision = None;
            self.daemon_reachable = Some(false);
            self.route_state = None;
            self.status = Status::error(self.strings.background_runtime_stopped());
        }
        for completion in completions {
            match completion {
                runtime::Completion::Diagnostics(snapshot) => {
                    self.backend_status = snapshot.backend_status;
                    self.fleet_status = snapshot.fleet_status;
                    self.protocol_probes = snapshot.protocol_probes;
                    self.daemon_status = snapshot.daemon_status;
                    self.daemon_capabilities = snapshot.daemon_capabilities;
                    self.daemon_reachable = snapshot.daemon_reachable;
                    self.route_state = snapshot.route_state;
                    if let Some(paused) = snapshot.paused {
                        self.paused = paused;
                    }
                    self.diagnostics_running = false;
                    self.next_status_poll = Instant::now() + Duration::from_secs(2);
                    if snapshot.announce {
                        self.status = Status::success(self.strings.status_diagnostics_refreshed());
                    }
                }
                runtime::Completion::Control { operation, result } => {
                    self.pending_control = self.pending_control.saturating_sub(1);
                    match operation {
                        runtime::Operation::Reload(previous_status) => match result {
                            Ok(_) => {
                                self.daemon_reachable = Some(true);
                                self.status = previous_status;
                            }
                            Err(error) => {
                                self.daemon_reachable = Some(false);
                                self.status = previous_status.with_caveat(
                                    self.strings.status_daemon_not_reloaded(&error.to_string()),
                                )
                            }
                        },
                        runtime::Operation::Pause { paused } => match result {
                            Ok(_) => {
                                self.daemon_reachable = Some(true);
                                self.paused = paused;
                                self.status = Status::success(if paused {
                                    self.strings.status_paused()
                                } else {
                                    self.strings.status_resumed()
                                });
                            }
                            Err(error) => {
                                self.daemon_reachable = Some(false);
                                self.status = Status::error(
                                    self.strings.status_control_unavailable(&error.to_string()),
                                )
                            }
                        },
                        runtime::Operation::Status => match result {
                            Ok(response) => {
                                self.daemon_capabilities =
                                    runtime::DaemonCapabilities::parse(&response);
                                self.daemon_status = response.trim().replace('\n', " · ");
                                self.daemon_reachable = Some(true);
                                self.route_state = runtime::parse_route_state(&response);
                                if let Some(paused) = runtime::parse_paused(&response) {
                                    self.paused = paused;
                                }
                            }
                            Err(error) => {
                                self.daemon_capabilities = None;
                                self.daemon_status = format!("Unavailable: {error}");
                                self.daemon_reachable = Some(false);
                                self.route_state = None;
                            }
                        },
                    }
                }
                runtime::Completion::ConfigReloaded(result) => {
                    self.pending_control = self.pending_control.saturating_sub(1);
                    let Some(expected_revision) = self.pending_reload_revision.take() else {
                        continue;
                    };
                    match *result {
                        Ok(snapshot) => {
                            if self.config_revision != expected_revision || self.draft_is_dirty() {
                                self.status = Status::warning(
                                    self.strings.status_reload_discarded_due_edits(),
                                );
                            } else {
                                self.apply_reload_snapshot(snapshot);
                            }
                        }
                        Err(error) => {
                            self.status = Status::error(self.strings.status_reload_failed(&error));
                        }
                    }
                }
            }
        }
        let now = Instant::now();
        if now >= self.next_status_poll {
            self.next_status_poll = now + Duration::from_secs(2);
            if self.pending_control == 0 && !self.diagnostics_running {
                if let Some(sender) = self.runtime_sender.as_ref() {
                    if sender
                        .try_send(runtime::Request::Control {
                            command: "status".into(),
                            operation: runtime::Operation::Status,
                        })
                        .is_ok()
                    {
                        self.pending_control += 1;
                    }
                }
            }
        }
        if self.diagnostics_running || self.pending_control > 0 {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else if self.runtime_receiver.is_some() {
            ctx.request_repaint_after(self.next_status_poll.saturating_duration_since(now));
        }
    }

    fn remember_undo(&mut self, previous: Config) {
        let Some(entry) = UndoEntry::between(&previous, &self.config) else {
            return;
        };
        if entry.estimated_bytes > MAX_UNDO_BYTES {
            self.undo.clear();
            self.undo_bytes = 0;
            return;
        }
        while self.undo.len() >= MAX_UNDO_HISTORY
            || self.undo_bytes.saturating_add(entry.estimated_bytes) > MAX_UNDO_BYTES
        {
            let Some(oldest) = self.undo.first() else {
                break;
            };
            self.undo_bytes = self.undo_bytes.saturating_sub(oldest.estimated_bytes);
            self.undo.remove(0);
        }
        self.undo_bytes = self.undo_bytes.saturating_add(entry.estimated_bytes);
        self.undo.push(entry);
    }

    /// Give the GUI a clear pre-save warning. The definitive check is repeated
    /// under the core store's writer lock during `save_config_candidate`, so
    /// this early check is only a UX optimization, not the concurrency guard.
    fn can_save_config(&mut self) -> bool {
        let current = Config::load_versioned(&self.path);
        if !current.is_ok_and(|loaded| loaded.revision == self.config_revision) {
            self.status = Status::warning(self.strings.status_config_changed_externally());
            return false;
        }
        true
    }

    fn save_config_candidate(&mut self, candidate: &Config) -> Result<(), String> {
        candidate.validate().map_err(|error| error.safe_summary())?;
        let replacement = toml_edit::ser::to_document(candidate)
            .map_err(|error| format!("could not serialize configuration: {error}"))?;
        let document =
            persistence::merge_config_document(self.config_document.clone(), replacement);
        let revision = Config::save_atomic_text_if_revision_matches(
            &self.path,
            &document.to_string(),
            &self.config_revision,
        )
        .map_err(|error| error.safe_summary())?;
        self.config_document = document;
        self.config_revision = revision;
        Ok(())
    }

    /// Reports a config change that was just saved to disk, then asks the
    /// running daemon to reload it. A failed reload request is appended to
    /// the status line rather than discarded -- and downgrades the tone from
    /// success to warning -- because otherwise the GUI reports success while
    /// the daemon keeps expanding the old config, and the user has no way to
    /// know the two have diverged.
    fn set_saved_status(&mut self, status: Status) {
        self.invalidate_preview();
        self.clear_command_preview();
        let Some(sender) = self.runtime_sender.as_ref() else {
            self.status = status;
            return;
        };
        match sender.try_send(runtime::Request::Control {
            command: "reload".into(),
            operation: runtime::Operation::Reload(status.clone()),
        }) {
            Ok(()) => {
                self.pending_control += 1;
                self.status = Status::info(self.strings.daemon_reloading());
            }
            Err(_) => {
                self.status = status.with_caveat(self.strings.background_queue_full());
            }
        }
    }

    /// Opens the settings window with the editable fields reset to what is
    /// currently persisted, so a previously abandoned edit never reappears.
    fn open_settings(&mut self) {
        self.settings_buffer = self.config.settings.max_buffer_chars.to_string();
        self.settings_undo_chord = self.config.settings.undo_chord.clone().unwrap_or_default();
        self.settings_font_scale = self.config.settings.font_scale;
        self.settings_error = None;
        self.settings_open = true;
    }

    fn set_dark_mode(&mut self, ctx: &egui::Context, dark: bool) {
        self.dark_mode = dark;
        ctx.set_theme(if dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        if save_gui_prefs(self.language, self.colorpack, self.dark_mode).is_err() {
            self.status = Status::warning(self.strings.status_appearance_save_failed());
        }
    }

    fn set_language(&mut self, language: Language) {
        self.language = language;
        self.strings.set_language(language);
        if save_gui_prefs(self.language, self.colorpack, self.dark_mode).is_err() {
            self.status = Status::warning(self.strings.status_appearance_save_failed());
        }
    }

    fn set_colorpack(&mut self, ctx: &egui::Context, pack: ColorPack) {
        self.colorpack = pack;
        theme::install_pack(ctx, pack, self.config.settings.font_scale);
        if save_gui_prefs(self.language, self.colorpack, self.dark_mode).is_err() {
            self.status = Status::warning(self.strings.status_appearance_save_failed());
        }
    }

    /// Applies a font scale immediately and persists it, so it behaves like
    /// the rest of the appearance settings rather than waiting on a Save
    /// button sitting under a different tab. Unlike a library edit this does
    /// not push an undo entry: the undo stack exists to recover snippet
    /// content, and filling it with display-preference steps would bury the
    /// change the user actually wants back.
    fn apply_font_scale(&mut self, ctx: &egui::Context, scale: FontScale) {
        if !self.can_save_config() {
            return;
        }
        self.settings_font_scale = scale;
        let mut candidate = self.config.clone();
        candidate.settings.font_scale = scale;
        // Apply the style first: the preference is visible even in the
        // unlikely case that persisting it fails, and the failure is
        // reported rather than silently producing a scale that resets on
        // the next launch.
        theme::install_pack(ctx, self.colorpack, scale);
        match self.save_config_candidate(&candidate) {
            Ok(()) => {
                self.config = candidate;
                self.status = Status::success(self.strings.status_font_size_saved());
            }
            Err(error) => {
                self.status = Status::error(self.strings.status_settings_save_failed(&error))
            }
        }
    }

    fn save_settings(&mut self, ctx: &egui::Context) {
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
        candidate.settings.font_scale = self.settings_font_scale;
        let undo_chord_input = self.settings_undo_chord.trim();
        candidate.settings.undo_chord =
            (!undo_chord_input.is_empty()).then(|| undo_chord_input.to_owned());
        if let Err(error) = candidate.validate() {
            let detail = error.safe_summary();
            self.status = Status::error(self.strings.status_settings_rejected(&detail));
            self.settings_error = Some(detail);
            return;
        }
        if !self.can_save_config() {
            return;
        }
        match self.save_config_candidate(&candidate) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.remember_undo(previous);
                self.settings_buffer = self.config.settings.max_buffer_chars.to_string();
                self.settings_undo_chord =
                    self.config.settings.undo_chord.clone().unwrap_or_default();
                self.settings_font_scale = self.config.settings.font_scale;
                self.settings_open = false;
                self.settings_error = None;
                theme::install_pack(ctx, self.colorpack, self.config.settings.font_scale);
                self.set_saved_status(Status::success(self.strings.status_settings_saved()));
            }
            Err(error) => {
                let detail = error;
                self.status = Status::error(self.strings.status_settings_save_failed(&detail));
                self.settings_error = Some(detail);
            }
        }
    }

    fn preview_import(&mut self) {
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

    fn apply_import(&mut self, replace_library: bool) {
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
        if !self.can_save_config() {
            self.import_preview = Some((imported, report));
            return;
        }
        match self.save_config_candidate(&candidate) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                self.set_selected_index((!self.config.expansion.is_empty()).then_some(0));
                self.draft = self
                    .selected
                    .map(|index| Draft::from_expansion(&self.config.expansion[index]));
                self.import_open = false;
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
                self.set_saved_status(Status::success(message));
            }
            Err(error) => {
                self.status = Status::error(self.strings.status_import_save_failed(&error));
                self.import_preview = Some((imported, report));
            }
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        self.search_index.visible_indices(
            &self.config,
            &self.filter,
            self.category_filter.as_deref(),
            self.search_fields,
        )
    }

    fn rebuild_search_index(&mut self) {
        self.search_index = library::SearchIndex::new(&self.config);
    }

    /// Distinct, sorted, non-empty categories currently in use — drives the
    /// sidebar filter chips and the editor's "pick existing" combo box.
    fn categories(&self) -> Vec<String> {
        self.search_index.categories().to_vec()
    }

    fn select(&mut self, index: usize) {
        self.cancel_app_detection();
        self.new_draft = false;
        self.new_draft_origin = None;
        self.set_selected_index(Some(index));
        self.draft = Some(Draft::from_expansion(&self.config.expansion[index]));
        self.preview_input = self.config.expansion[index].trigger.clone();
        self.pending_action = None;
        self.clear_command_preview();
    }

    fn cancel_app_detection(&mut self) {
        if let Some(task) = self.app_detection.as_mut() {
            task.cancelled.store(true, Ordering::Release);
        }
    }

    /// A cancelled worker may finish after the editor has disappeared (for
    /// example after deleting the last snippet). Reap its result here so the
    /// task does not pin the receiver or block a future detection forever.
    fn reap_app_detection_without_editor(&mut self, ctx: &egui::Context) {
        if self.selected_index().is_some() {
            return;
        }
        let Some(task) = self.app_detection.as_ref() else {
            return;
        };
        match task.receiver.try_recv() {
            Ok(_) | Err(mpsc::TryRecvError::Disconnected) => self.app_detection = None,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }

    /// The selected index, but only while it still addresses a snippet.
    /// Every action that indexes `config.expansion` goes through this, so a
    /// selection left behind by a reload or an external edit reports "no
    /// snippet selected" instead of panicking on an out-of-range index.
    fn selected_index(&self) -> Option<usize> {
        match self.selected_id.as_ref() {
            Some(id) => self
                .config
                .expansion
                .iter()
                .position(|entry| &entry.id == id),
            None => self
                .selected
                .filter(|index| *index < self.config.expansion.len()),
        }
    }

    fn set_selected_index(&mut self, index: Option<usize>) {
        self.selected = index.filter(|index| *index < self.config.expansion.len());
        self.selected_id = self
            .selected
            .map(|index| self.config.expansion[index].id.clone());
    }

    /// Whether the editor holds edits that are not yet in the configuration.
    ///
    /// Called several times per frame (toolbar badge, action bar, window
    /// title), so the comma-separated fields are compared in place instead of
    /// rebuilding a joined `String` -- and a `Vec` to join from -- on each
    /// call. The cheap scalar comparisons are ordered first so a draft that
    /// differs at all usually answers before touching a list at all.
    fn draft_is_dirty(&self) -> bool {
        if self.new_draft {
            return self.draft.is_some();
        }
        let (Some(index), Some(draft)) = (self.selected_index(), self.draft.as_ref()) else {
            return false;
        };
        let expansion = &self.config.expansion[index];
        draft.trigger != expansion.trigger
            || draft.description != expansion.description
            || draft.category != expansion.category
            || draft.replacement != expansion.replacement
            || draft.tags != expansion.tags
            || draft.app_filter != expansion.app_filter
            || draft.enabled != expansion.enabled
            || draft.match_mode != expansion.match_mode
            || draft.propagate_case != expansion.propagate_case
            || !draft.matches_command(expansion.command.as_ref())
    }

    fn request_action(&mut self, action: PendingAction) {
        if matches!(&action, PendingAction::Select(index) if self.selected_index() == Some(*index))
        {
            return;
        }
        self.cancel_app_detection();
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
            PendingAction::Close => self.close_after_confirm = true,
        }
    }

    fn discard_pending(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        if self.new_draft {
            self.abandon_new_draft();
        }
        self.execute_action(action);
    }

    fn abandon_new_draft(&mut self) {
        if !self.new_draft {
            return;
        }
        let origin_id = self.new_draft_origin.take();
        let origin = origin_id
            .as_ref()
            .and_then(|id| self.config.expansion.iter().position(|item| &item.id == id))
            .or_else(|| (!self.config.expansion.is_empty()).then_some(0));
        self.new_draft = false;
        self.set_selected_index(origin);
        self.draft = origin.map(|index| Draft::from_expansion(&self.config.expansion[index]));
        self.preview_input = origin
            .map(|index| self.config.expansion[index].trigger.clone())
            .unwrap_or_default();
        self.clear_command_preview();
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
        if self.pending_reload_revision.is_some() {
            return;
        }
        let Some(sender) = self.runtime_sender.as_ref() else {
            self.status = Status::error(self.strings.background_runtime_stopped());
            return;
        };
        match sender.try_send(runtime::Request::ReloadConfig {
            path: self.path.clone(),
        }) {
            Ok(()) => {
                self.pending_reload_revision = Some(self.config_revision.clone());
                self.pending_control += 1;
                self.status = Status::info(self.strings.config_reload_running());
            }
            Err(_) => self.status = Status::warning(self.strings.background_queue_full()),
        }
    }

    fn apply_reload_snapshot(&mut self, snapshot: runtime::ReloadSnapshot) {
        let new_config = snapshot.config;
        let new_selected = self
            .selected_id
            .as_ref()
            .and_then(|id| {
                new_config
                    .expansion
                    .iter()
                    .position(|entry| &entry.id == id)
            })
            .or_else(|| (!new_config.expansion.is_empty()).then_some(0));
        let new_draft =
            new_selected.map(|index| Draft::from_expansion(&new_config.expansion[index]));
        let new_preview_input = new_selected
            .map(|index| new_config.expansion[index].trigger.clone())
            .unwrap_or_default();

        self.config = new_config;
        self.new_draft = false;
        self.new_draft_origin = None;
        self.search_index = snapshot.search_index;
        self.config_document = snapshot.document;
        self.config_revision = snapshot.revision;
        self.set_selected_index(new_selected);
        self.draft = new_draft;
        self.preview_input = new_preview_input;
        self.undo.clear();
        self.undo_bytes = 0;
        self.invalidate_preview();
        self.clear_command_preview();
        self.theme_refresh_pending = true;
        self.status = Status::success(self.strings.status_config_reloaded());
    }

    fn save_selected(&mut self) {
        let Some(draft) = self.draft.as_ref().cloned() else {
            self.status = Status::warning(self.strings.no_selection());
            return;
        };
        let is_new = self.new_draft;
        let Some(index) = self
            .selected_index()
            .or_else(|| is_new.then_some(self.config.expansion.len()))
        else {
            self.status = Status::warning(self.strings.no_selection());
            return;
        };
        let mut candidate = self.config.clone();
        let command = match draft.command_config() {
            Ok(command) => command,
            Err(error) => {
                self.status =
                    Status::error(self.strings.status_command_invalid(&error.to_string()));
                return;
            }
        };
        if is_new {
            if draft.replacement.is_empty() {
                self.status = Status::error(
                    self.strings
                        .status_save_rejected(self.strings.new_snippet_replacement_required()),
                );
                return;
            }
            candidate.expansion.push(ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: draft.trigger.clone(),
                replacement: draft.replacement.clone(),
                description: draft.description.clone(),
                tags: draft.tags.clone(),
                category: draft.category.clone(),
                app_filter: draft.app_filter.clone(),
                match_mode: draft.match_mode,
                command,
                enabled: draft.enabled,
                propagate_case: draft.propagate_case,
            });
        } else {
            candidate.expansion[index].trigger = draft.trigger.clone();
            candidate.expansion[index].description = draft.description.clone();
            candidate.expansion[index].tags = draft.tags.clone();
            candidate.expansion[index].category = draft.category.clone();
            candidate.expansion[index].app_filter = draft.app_filter.clone();
            candidate.expansion[index].replacement = draft.replacement.clone();
            candidate.expansion[index].enabled = draft.enabled;
            candidate.expansion[index].match_mode = draft.match_mode;
            candidate.expansion[index].propagate_case = draft.propagate_case;
            candidate.expansion[index].command = command;
        }
        if let Err(error) = candidate.validate() {
            self.status = Status::error(self.strings.status_save_rejected(&error.safe_summary()));
            return;
        }
        if !self.can_save_config() {
            return;
        }
        match self.save_config_candidate(&candidate) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                if is_new {
                    self.new_draft = false;
                    self.new_draft_origin = None;
                    self.set_selected_index(Some(index));
                }
                self.draft = self
                    .selected
                    .map(|selected| Draft::from_expansion(&self.config.expansion[selected]));
                self.preview_input = self
                    .selected
                    .map(|selected| self.config.expansion[selected].trigger.clone())
                    .unwrap_or_default();
                self.clear_command_preview();
                self.set_saved_status(Status::success(self.strings.status_snippet_saved()));
            }
            Err(error) => self.status = Status::error(self.strings.status_save_failed(&error)),
        }
    }

    fn undo(&mut self) {
        let Some(entry) = self.undo.last() else {
            self.status = Status::warning(self.strings.status_nothing_to_undo());
            return;
        };
        let restored = match entry.restore(&self.config) {
            Ok(config) => config,
            Err(error) => {
                self.status = Status::error(self.strings.status_undo_save_failed(&error));
                return;
            }
        };
        // Save the restored config to disk *before* committing it to GUI
        // state or popping it off the undo stack. Doing it in the opposite
        // order (as before) meant a failed save still left the undo entry
        // consumed and the in-memory config changed, with disk untouched --
        // GUI, daemon, and disk would all disagree about what the config is.
        if !self.can_save_config() {
            return;
        }
        if let Err(error) = self.save_config_candidate(&restored) {
            self.status = Status::error(self.strings.status_undo_save_failed(&error));
            return;
        }
        let entry = self.undo.pop().expect("checked non-empty above");
        self.undo_bytes = self.undo_bytes.saturating_sub(entry.estimated_bytes);
        self.config = restored;
        self.rebuild_search_index();
        let restored_selection = self
            .selected_id
            .as_ref()
            .and_then(|id| {
                self.config
                    .expansion
                    .iter()
                    .position(|entry| &entry.id == id)
            })
            .or_else(|| {
                self.selected
                    .filter(|index| *index < self.config.expansion.len())
            });
        self.set_selected_index(restored_selection);
        self.draft = self
            .selected
            .map(|index| Draft::from_expansion(&self.config.expansion[index]));
        self.preview_input = self
            .selected
            .map(|index| self.config.expansion[index].trigger.clone())
            .unwrap_or_default();
        self.clear_command_preview();
        self.set_saved_status(Status::success(self.strings.status_undone()));
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
        let draft_expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
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
        };
        self.new_draft_origin = self.selected_id.clone();
        self.new_draft = true;
        self.set_selected_index(None);
        self.draft = Some(Draft::from_expansion(&draft_expansion));
        self.preview_input = draft_expansion.trigger;
        self.clear_command_preview();
        self.status = Status::info(self.strings.new_snippet_draft());
    }

    fn create_test_snippet(&mut self) {
        if !self.config.expansion.is_empty() {
            self.status = Status::warning(self.strings.no_selection());
            return;
        }
        let mut candidate = self.config.clone();
        candidate.expansion.push(ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":wayexpand-test".into(),
            replacement: "WayExpand is working!".into(),
            description: self.strings.onboarding_sample_description().into(),
            tags: vec!["tutorial".into()],
            category: self.strings.onboarding_sample_category().into(),
            app_filter: Vec::new(),
            match_mode: MatchMode::WordBoundary,
            command: None,
            enabled: true,
            propagate_case: false,
        });
        if !self.can_save_config() {
            return;
        }
        match self.save_config_candidate(&candidate) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                self.select(0);
                self.set_saved_status(Status::success(self.strings.status_created()));
            }
            Err(error) => self.status = Status::error(self.strings.status_create_failed(&error)),
        }
    }

    fn duplicate_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            self.status = Status::warning(self.strings.no_selection());
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
        duplicate.id = ExpansionConfig::new_id();
        if !duplicate.description.is_empty() {
            duplicate.description.push_str(" (copy)");
        }
        let mut candidate = self.config.clone();
        candidate.expansion.push(duplicate);
        if !self.can_save_config() {
            return;
        }
        match self.save_config_candidate(&candidate) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                self.select(self.config.expansion.len() - 1);
                self.set_saved_status(Status::success(self.strings.status_duplicated()));
            }
            Err(error) => self.status = Status::error(self.strings.status_duplicate_failed(&error)),
        }
    }

    fn perform_delete_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            self.status = Status::warning(self.strings.no_selection());
            return;
        };
        let mut candidate = self.config.clone();
        let trigger = candidate.expansion[index].trigger.clone();
        candidate.expansion.remove(index);
        if !self.can_save_config() {
            return;
        }
        match self.save_config_candidate(&candidate) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                let next_selection = (!self.config.expansion.is_empty())
                    .then_some(index.min(self.config.expansion.len() - 1));
                self.set_selected_index(next_selection);
                self.draft = self
                    .selected
                    .map(|selected| Draft::from_expansion(&self.config.expansion[selected]));
                self.clear_command_preview();
                self.set_saved_status(Status::success(self.strings.status_deleted(&trigger)));
            }
            Err(error) => self.status = Status::error(self.strings.status_delete_failed(&error)),
        }
    }

    /// Flips a snippet's enabled flag directly from the sidebar dot and
    /// saves immediately, independent of selection or any in-progress
    /// unsaved draft. If the toggled row is the one currently being edited,
    /// only its `enabled` field is synced so other unsaved edits survive.
    fn toggle_enabled(&mut self, index: usize) {
        if index >= self.config.expansion.len() {
            self.status = Status::warning(self.strings.no_selection());
            return;
        }
        let mut candidate = self.config.clone();
        candidate.expansion[index].enabled = !candidate.expansion[index].enabled;
        let now_enabled = candidate.expansion[index].enabled;
        let trigger = candidate.expansion[index].trigger.clone();
        if !self.can_save_config() {
            return;
        }
        match self.save_config_candidate(&candidate) {
            Ok(()) => {
                let previous = std::mem::replace(&mut self.config, candidate);
                self.rebuild_search_index();
                self.remember_undo(previous);
                if self.selected == Some(index) {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.enabled = now_enabled;
                    }
                }
                let message = if now_enabled {
                    self.strings.status_snippet_enabled(&trigger)
                } else {
                    self.strings.status_snippet_disabled(&trigger)
                };
                self.set_saved_status(Status::success(message));
            }
            Err(error) => self.status = Status::error(self.strings.status_toggle_failed(&error)),
        }
    }

    /// Renders a live preview for a plain (non-command) draft. Must never be
    /// called for a command-backed draft: the engine could spawn the
    /// configured program continuously while the editor is simply open --
    /// including any side-effecting script the user has not even saved yet.
    /// Command previews are explicit and user-triggered instead; see
    /// `run_command_preview`.
    fn preview(&mut self) -> String {
        let source = if self.new_draft {
            if self.draft.is_none() {
                return self.strings.no_selection().into();
            }
            None
        } else {
            let Some(index) = self.selected_index() else {
                return self.strings.no_selection().into();
            };
            Some(&self.config.expansion[index])
        };
        if let Some((cached_revision, cached_input, cached_result)) = &self.preview_cache {
            if *cached_revision == self.preview_revision && cached_input == &self.preview_input {
                return cached_result.clone();
            }
        }
        let result = preview::render(
            source,
            &self.config.settings,
            &self.config.organization,
            self.draft.as_ref(),
            &self.preview_input,
            &self.preview_app,
        );
        self.preview_cache = Some((
            self.preview_revision,
            self.preview_input.clone(),
            result.clone(),
        ));
        result
    }

    fn invalidate_preview(&mut self) {
        self.preview_revision = self.preview_revision.wrapping_add(1);
        self.preview_cache = None;
    }

    /// Runs the draft's configured command exactly once, on explicit user
    /// request (a button click), and caches the result for display. This is
    /// the only place a command-backed draft's program should ever run
    /// before it is saved.
    fn run_command_preview(&mut self) {
        self.clear_command_preview();
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        self.command_preview_key = Some(self.preview_revision);
        let command = match draft.command_config() {
            Ok(Some(command)) => command,
            Ok(None) => {
                self.command_preview_result =
                    Some(Err(self.strings.status_enable_command_first().into()));
                return;
            }
            Err(error) => {
                self.command_preview_result =
                    Some(Err(self.strings.status_command_invalid(&error.to_string())));
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancelled);
        self.command_preview_result = None;
        self.command_preview_receiver = Some(receiver);
        self.command_preview_cancel = Some(cancelled);
        // `Strings` is a plain language tag, so the worker can phrase its own
        // failure in the user's language without borrowing the app.
        let strings = Strings::new(self.language);
        thread::spawn(move || {
            let result = wayexpand_core::run_command_cancellable(&command, &worker_cancel)
                .map_err(|error| strings.status_command_failed(&error.to_string()));
            let _ = sender.send(result);
        });
    }

    fn poll_command_preview(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.command_preview_receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.command_preview_receiver = None;
                self.command_preview_cancel = None;
                self.command_preview_result = Some(result);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.command_preview_receiver = None;
                self.command_preview_cancel = None;
                self.command_preview_result =
                    Some(Err(self.strings.status_command_preview_failed().into()));
            }
        }
    }

    fn clear_command_preview(&mut self) {
        if let Some(cancel) = self.command_preview_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.command_preview_result = None;
        self.command_preview_key = None;
        self.command_preview_receiver = None;
    }

    fn toggle_pause(&mut self) {
        if self.pending_control > 0 {
            return;
        }
        let paused = !self.paused;
        let command = if paused { "pause" } else { "resume" };
        let Some(sender) = self.runtime_sender.as_ref() else {
            self.status = Status::error(self.strings.background_runtime_stopped());
            return;
        };
        match sender.try_send(runtime::Request::Control {
            command: command.into(),
            operation: runtime::Operation::Pause { paused },
        }) {
            Ok(()) => {
                self.pending_control += 1;
                self.status = Status::info(self.strings.daemon_control_running());
            }
            Err(_) => {
                self.status = Status::warning(self.strings.background_queue_full());
            }
        }
    }

    fn any_dialog_open(&self) -> bool {
        self.diagnostics_open || self.import_open || self.settings_open || self.evdev_setup_open
    }

    /// Escape closes one dialog at a time, most recently opened first. The
    /// import dialog keeps its preview open on the first Escape so a loaded
    /// library is not discarded by a keystroke meant to dismiss something
    /// else.
    fn close_topmost_dialog(&mut self) {
        if self.evdev_setup_open {
            self.evdev_setup_open = false;
        } else if self.settings_open {
            self.settings_open = false;
        } else if self.import_open {
            if self.import_preview.is_some() {
                self.import_preview = None;
            } else {
                self.import_open = false;
            }
        } else if self.diagnostics_open {
            self.diagnostics_open = false;
        }
    }

    fn render_toolbar(&mut self, root: &mut egui::Ui, palette: &Palette) {
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
            .show(root, |ui| {
                // One row when there is room: identity on the left, the
                // controls people reach for on the right. Narrow windows fall
                // back to a second row for search instead of overlapping.
                let single_row = ui.available_width() >= 980.0;
                ui.horizontal(|ui| {
                    self.render_toolbar_brand(ui, palette);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.render_toolbar_controls(ui, palette);
                        if single_row {
                            ui.add_space(4.0);
                            self.render_search(ui);
                        }
                    });
                });
                if !single_row {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_search(ui);
                        });
                    });
                }
            });
    }

    fn render_toolbar_brand(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.label(RichText::new("⚡").size(20.0).color(palette.accent));
        ui.label(RichText::new("WayExpand").heading().strong());
        ui.label(RichText::new(self.strings.title()).color(palette.muted));
        ui.add_space(8.0);
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
    }

    /// Daemon health and the actions menu, laid out right to left.
    fn render_toolbar_controls(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let ctx = ui.ctx().clone();
        let (daemon_label, daemon_color) = match self.daemon_reachable {
            Some(true) => (self.strings.daemon_running_status(), palette.success),
            Some(false) => (self.strings.daemon_unreachable_status(), palette.danger),
            None => (self.strings.daemon_unknown_status(), palette.muted),
        };
        let (route_label, route_color) = match self.route_state {
            Some(runtime::RouteState::Connected) if self.paused => {
                (self.strings.route_paused_status(), palette.warning)
            }
            Some(runtime::RouteState::Connected) => {
                (self.strings.route_connected_status(), palette.success)
            }
            Some(runtime::RouteState::Reconnecting) => {
                (self.strings.route_reconnecting_status(), palette.warning)
            }
            Some(runtime::RouteState::Starting) => {
                (self.strings.route_starting_status(), palette.warning)
            }
            Some(runtime::RouteState::PermissionRequired) => {
                (self.strings.route_permission_required_status(), palette.warning)
            }
            Some(runtime::RouteState::PortalRevoked) => {
                (self.strings.route_portal_revoked_status(), palette.warning)
            }
            Some(runtime::RouteState::Unsupported) => {
                (self.strings.route_unsupported_status(), palette.muted)
            }
            Some(runtime::RouteState::Degraded) => {
                (self.strings.route_degraded_status(), palette.warning)
            }
            Some(runtime::RouteState::Failed) => {
                (self.strings.route_failed_status(), palette.danger)
            }
            Some(runtime::RouteState::Stopped) => {
                (self.strings.route_stopped_status(), palette.muted)
            }
            None => (self.strings.route_unknown_status(), palette.muted),
        };
        let more_actions = self.strings.more_actions();
        let actions_response = ui
            .menu_button(more_actions, |ui| {
                if ui.button(self.strings.reload()).clicked() {
                    self.request_action(PendingAction::Reload);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.pending_control == 0,
                        egui::Button::new(if self.paused {
                            self.strings.resume()
                        } else {
                            self.strings.pause()
                        }),
                    )
                    .clicked()
                {
                    self.toggle_pause();
                    ui.close();
                }
                if ui.button(self.strings.diagnostics()).clicked() {
                    self.diagnostics_open = true;
                    self.refresh_diagnostics(true);
                    ui.close();
                }
                if ui.button(self.strings.import_espanso()).clicked() {
                    self.import_open = true;
                    self.import_preview = None;
                    ui.close();
                }
                ui.separator();
                if ui.button(self.strings.settings()).clicked() {
                    self.open_settings();
                    ui.close();
                }
                if ui
                    .button(if self.dark_mode {
                        self.strings.theme_light()
                    } else {
                        self.strings.theme_dark()
                    })
                    .clicked()
                {
                    self.set_dark_mode(&ctx, !self.dark_mode);
                    ui.close();
                }
            })
            .response;
        actions_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, more_actions)
        });
        actions_response.on_hover_text(more_actions);
        if ui
            .add(
                egui::Button::new(RichText::new(route_label).color(route_color))
                    .fill(theme::tint(route_color, 38))
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(egui::CornerRadius::same(255))
                    .min_size(egui::vec2(0.0, 30.0)),
            )
            .on_hover_text(self.daemon_status.clone())
            .clicked()
        {
            self.diagnostics_open = true;
            self.refresh_diagnostics(true);
        }
        // The first-run screen has its own step for this.
        theme::pill(
            ui,
            daemon_label,
            daemon_color,
            theme::tint(daemon_color, 38),
        );
        if self.daemon_reachable == Some(false) && !self.config.expansion.is_empty() {
            self.turn_on_button(ui, palette);
        }
    }

    /// The search field and its field-scope menu, laid out right to left.
    fn render_search(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.strings.search_fields(), |ui| {
            ui.checkbox(
                &mut self.search_fields.triggers,
                self.strings.search_triggers(),
            );
            ui.checkbox(
                &mut self.search_fields.descriptions,
                self.strings.search_descriptions(),
            );
            ui.checkbox(&mut self.search_fields.tags, self.strings.search_tags());
            ui.checkbox(
                &mut self.search_fields.replacements,
                self.strings.search_replacements(),
            );
        });
        if !self.filter.is_empty()
            && ui
                .small_button("×")
                .on_hover_text(self.strings.clear_filters())
                .clicked()
        {
            self.filter.clear();
        }
        ui.add(
            TextEdit::singleline(&mut self.filter)
                .id(egui::Id::new(SEARCH_FIELD_SALT))
                .hint_text(self.strings.search_placeholder())
                .margin(egui::Margin::symmetric(10, 7))
                .desired_width(280.0),
        )
        .on_hover_text(self.strings.search_tooltip());
    }

    fn render_diagnostics(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.diagnostics_open {
            let mut open = self.diagnostics_open;
            egui::Window::new(self.strings.diagnostics_title())
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .min_width(340.0)
                .max_height(560.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    theme::section_header(ui, "", self.strings.runtime_health());
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(self.strings.onboarding_certification_note())
                            .color(palette.warning)
                            .small(),
                    );
                    ui.label(
                        RichText::new(self.strings.daemon())
                            .color(palette.muted)
                            .small(),
                    );
                    theme::card(ui, palette, |ui| {
                        ui.label(RichText::new(&self.daemon_status).monospace());
                    });
                    if self.daemon_reachable == Some(true) {
                        let capabilities = self.daemon_capabilities.unwrap_or_default();
                        theme::section_header(ui, "", self.strings.configuration_health());
                        let configuration_ok = !self.fleet_status.starts_with("invalid:");
                        ui.horizontal(|ui| {
                            ui.label(self.strings.configuration_file());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.colored_label(
                                        if configuration_ok {
                                            palette.success
                                        } else {
                                            palette.danger
                                        },
                                        if configuration_ok {
                                            self.strings.configuration_healthy()
                                        } else {
                                            self.strings.configuration_invalid()
                                        },
                                    );
                                },
                            );
                        });
                        theme::section_header(ui, "", self.strings.safety());
                        self.render_capability_row(
                            ui,
                            palette,
                            "sensitive_focus",
                            capabilities.capture_sensitive_focus,
                        );
                        self.render_capability_row(
                            ui,
                            palette,
                            "atomic_replace",
                            capabilities.inject_atomic_replace,
                        );
                        ui.add_space(8.0);
                        theme::section_header(ui, "", self.strings.capture_guarantees());
                        for (key, value) in [
                            ("sensitive_focus", capabilities.capture_sensitive_focus),
                            ("exclusive", capabilities.capture_exclusive),
                            (
                                "reliable_key_state",
                                capabilities.capture_reliable_key_state,
                            ),
                            ("capture_passthrough", capabilities.capture_key_passthrough),
                            ("composition", capabilities.capture_composition_aware),
                        ] {
                            self.render_capability_row(ui, palette, key, value);
                        }
                        theme::section_header(ui, "", self.strings.application_context());
                        self.render_capability_row(
                            ui,
                            palette,
                            "window_tracker",
                            capabilities.window_tracker_connected,
                        );
                        theme::section_header(ui, "", self.strings.injection_guarantees());
                        for (key, value) in [
                            ("atomic_replace", capabilities.inject_atomic_replace),
                            ("unicode", capabilities.inject_full_unicode),
                            ("cursor", capabilities.inject_cursor_reposition),
                            ("injection_passthrough", capabilities.inject_key_passthrough),
                        ] {
                            self.render_capability_row(ui, palette, key, value);
                        }
                    }
                    ui.label(
                        RichText::new(format!(
                            "{}: {}",
                            self.strings.fleet_layers(),
                            self.fleet_status
                        ))
                        .small()
                        .color(palette.muted),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        theme::section_header(ui, "", self.strings.backends());
                        if ui
                            .add_enabled(
                                !self.diagnostics_running,
                                egui::Button::new(self.strings.refresh()),
                            )
                            .clicked()
                        {
                            self.refresh_diagnostics(true);
                        }
                        if self.diagnostics_running {
                            ui.spinner();
                            ui.label(self.strings.diagnostics_running());
                        }
                    });
                    ui.add_space(4.0);
                    for status in &self.backend_status {
                        let color = match status.state {
                            BackendState::Available => palette.success,
                            BackendState::RequiresPermission => palette.warning,
                            BackendState::Implemented => palette.warning,
                            BackendState::Unavailable | BackendState::NotImplemented => {
                                palette.muted
                            }
                        };
                        ui.horizontal(|ui| {
                            theme::pill(
                                ui,
                                self.strings.backend_state(status.state),
                                color,
                                theme::tint(color, 32),
                            );
                            ui.label(
                                RichText::new(self.strings.backend_label(status.kind)).strong(),
                            );
                        });
                        ui.label(RichText::new(&status.detail).small().color(palette.muted));
                        // The exact triple `wayexpand doctor` reports, kept
                        // verbatim so the GUI mirrors the documented
                        // diagnostic vocabulary instead of paraphrasing it.
                        ui.collapsing(self.strings.technical_details(), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "implementation={} · availability={} · permission={}",
                                    status.implementation(),
                                    status.availability(),
                                    status.permission()
                                ))
                                .monospace()
                                .small()
                                .color(palette.muted),
                            );
                        });
                        ui.add_space(6.0);
                    }
                    ui.separator();
                    theme::section_header(ui, "", self.strings.protocol_probes());
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
    }

    fn render_capability_row(
        &self,
        ui: &mut egui::Ui,
        palette: &Palette,
        key: &str,
        value: Option<bool>,
    ) {
        let (label, color) = match value {
            Some(true) => (self.strings.capability_available(), palette.success),
            Some(false) => (self.strings.capability_unavailable(), palette.warning),
            None => (self.strings.capability_unknown(), palette.muted),
        };
        ui.horizontal(|ui| {
            ui.label(self.strings.capability_label(key));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(color, label);
            });
        });
    }

    fn render_import_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
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

    fn render_settings_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        egui::Window::new(self.strings.settings_title())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(420.0)
            .max_height(560.0)
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
                            self.config.settings.font_scale.multiplier(),
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
    fn render_appearance_settings(&mut self, ui: &mut egui::Ui, palette: &Palette) {
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
                    self.config.settings.font_scale.multiplier(),
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
            ] {
                if theme::chip_scaled(
                    ui,
                    palette,
                    label,
                    self.language == language,
                    self.config.settings.font_scale.multiplier(),
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
                    self.config.settings.font_scale.multiplier(),
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
                self.config.settings.font_scale.multiplier(),
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
    fn render_engine_settings(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.label(
            RichText::new(self.strings.engine_note())
                .small()
                .color(palette.muted),
        );
        ui.add_space(10.0);

        ui.label(self.strings.buffer_limit());
        ui.add(TextEdit::singleline(&mut self.settings_buffer).desired_width(120.0));
        ui.label(
            RichText::new(self.strings.buffer_limit_help())
                .small()
                .color(palette.muted),
        );

        ui.add_space(10.0);
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

    fn render_snippet_list(&mut self, root: &mut egui::Ui, palette: &Palette) {
        // The first-run editor is already a focused setup surface with one
        // explicit test-snippet action. Avoid a mostly empty library panel
        // with a competing New button until there is a saved item to browse.
        if self.config.expansion.is_empty() && !self.new_draft {
            return;
        }
        egui::Panel::left("snippets")
            .resizable(true)
            .default_size(340.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(14, 14)),
            )
            .show(root, |ui| {
                ui.label(
                    RichText::new(if self.filter.is_empty() {
                        self.strings.your_library()
                    } else {
                        self.strings.filtered_snippets()
                    })
                    .size(15.0)
                    .strong(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, palette, self.strings.new_button())
                        .on_hover_text(self.strings.new_tooltip())
                        .clicked()
                    {
                        self.request_action(PendingAction::New);
                    }
                    if theme::secondary_button(ui, palette, self.strings.duplicate()).clicked() {
                        self.request_action(PendingAction::Duplicate);
                    }
                    if theme::secondary_button(
                        ui,
                        palette,
                        &self.strings.undo_button(self.undo.len()),
                    )
                    .clicked()
                    {
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
                            palette,
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
                            if theme::chip_scaled(ui, palette, category, selected, font_scale)
                                .clicked()
                            {
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
                // Filtering walks the whole library, so it happens once per
                // frame and the same result answers both the list and the
                // empty-state check below.
                let visible_indices = self.visible_indices();
                let nothing_visible = visible_indices.is_empty();
                ScrollArea::vertical().show(ui, |ui| {
                    for index in visible_indices {
                        let expansion = &self.config.expansion[index];
                        let detail = if expansion.command.is_some() {
                            self.strings.command_backed_summary().to_owned()
                        } else {
                            expansion.description.clone()
                        };
                        let response = theme::snippet_row_scaled(
                            ui,
                            palette,
                            theme::SnippetRow {
                                selected: self.selected_index() == Some(index),
                                enabled: expansion.enabled,
                                command_backed: expansion.command.is_some(),
                                trigger: &expansion.trigger,
                                detail: &detail,
                                category: &expansion.category,
                                detail_placeholder: self.strings.no_description(),
                                toggle_hint: if expansion.enabled {
                                    self.strings.click_to_disable()
                                } else {
                                    self.strings.click_to_enable()
                                },
                            },
                            font_scale,
                        );
                        if response.toggle.clicked() {
                            self.toggle_enabled(index);
                        } else if response.row.clicked() {
                            self.request_action(PendingAction::Select(index));
                        }
                        ui.add_space(3.0);
                    }
                    if self.config.expansion.is_empty() {
                        ui.add_space(16.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("📭").size(28.0));
                            ui.label(
                                RichText::new(self.strings.no_snippets()).color(palette.muted),
                            );
                            ui.add_space(6.0);
                        });
                    } else if nothing_visible {
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
                                (true, None) => self.strings.no_snippets_match_filter().to_owned(),
                            };
                            ui.label(RichText::new(reason).color(palette.muted));
                            ui.add_space(6.0);
                            if theme::secondary_button(ui, palette, self.strings.clear_filters())
                                .clicked()
                            {
                                self.filter.clear();
                                self.category_filter = None;
                            }
                        });
                    }
                });
            });
    }

    fn render_editor(&mut self, root: &mut egui::Ui, palette: &Palette) {
        // egui sends text/key/paste/click events only on frames where user
        // input can mutate an editor control. Advancing this scalar revision
        // avoids hashing the full draft (which may contain megabytes of text)
        // on every otherwise-idle repaint. Pointer movement is intentionally
        // excluded so moving the mouse does not invalidate the preview.
        let editor_input = root.ctx().input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                        | egui::Event::Key { pressed: true, .. }
                        | egui::Event::PointerButton { pressed: true, .. }
                )
            })
        });
        if editor_input {
            self.invalidate_preview();
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.background)
                    .inner_margin(egui::Margin::symmetric(22, 18)),
            )
            .show(root, |ui| {
                if self.selected_index().is_none()
                    && !self.new_draft
                    && !self.config.expansion.is_empty()
                {
                    ui.centered_and_justified(|ui| {
                        ui.label(self.strings.select_snippet_prompt());
                    });
                    return;
                }
                let Some(index) = self
                    .selected_index()
                    .or_else(|| self.new_draft.then_some(self.config.expansion.len()))
                else {
                    let desktop = env::var("XDG_CURRENT_DESKTOP")
                        .or_else(|_| env::var("XDG_SESSION_DESKTOP"))
                        .unwrap_or_else(|_| "Linux desktop".into());
                    let is_wayland = env::var_os("WAYLAND_DISPLAY").is_some();
                    let app_context_available = self.backend_status.iter().any(|status| {
                        status.kind == wayexpand_core::BackendKind::WindowTracker
                            && status.state == BackendState::Available
                    });
                    let best_probe = |matches: fn(wayexpand_core::BackendKind) -> bool| {
                        self.backend_status
                            .iter()
                            .filter(|status| matches(status.kind))
                            .map(|status| status.state)
                            .max_by_key(|state| match state {
                                BackendState::Available => 4,
                                BackendState::RequiresPermission => 3,
                                BackendState::Implemented => 2,
                                BackendState::Unavailable => 1,
                                BackendState::NotImplemented => 0,
                            })
                            .unwrap_or(BackendState::NotImplemented)
                    };
                    let keyboard_probe = best_probe(|kind| {
                        matches!(
                            kind,
                            wayexpand_core::BackendKind::InputMethodV2
                                | wayexpand_core::BackendKind::Evdev
                        )
                    });
                    let injection_probe = best_probe(|kind| {
                        matches!(
                            kind,
                            wayexpand_core::BackendKind::Libei
                                | wayexpand_core::BackendKind::WlrootsVirtualKeyboard
                                | wayexpand_core::BackendKind::Uinput
                        )
                    });
                    ui.vertical_centered(|ui| {
                        ui.add_space(22.0);
                        ui.label(RichText::new("⚡").size(32.0).color(palette.accent));
                        ui.add_space(4.0);
                        ui.heading(self.strings.welcome_title());
                        ui.label(RichText::new(self.strings.welcome_intro()).color(palette.muted));
                        ui.add_space(20.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width().min(620.0), ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                theme::card(ui, palette, |ui| {
                                    ui.label(
                                        RichText::new(self.strings.onboarding_desktop_step())
                                            .strong(),
                                    );
                                    ui.label(self.strings.onboarding_desktop(&desktop, is_wayland));
                                    ui.add_space(10.0);
                                    ui.label(
                                        RichText::new(self.strings.onboarding_support_title())
                                            .strong(),
                                    );
                                    ui.label(
                                        RichText::new(self.strings.onboarding_probe_caveat())
                                            .color(palette.muted),
                                    );
                                    ui.horizontal(|ui| {
                                        ui.label(self.strings.onboarding_keyboard_label());
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    self.strings
                                                        .onboarding_backend_state(keyboard_probe),
                                                );
                                            },
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label(self.strings.onboarding_injection_label());
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    self.strings
                                                        .onboarding_backend_state(injection_probe),
                                                );
                                            },
                                        );
                                    });
                                    ui.label(
                                        RichText::new(
                                            self.strings
                                                .onboarding_detection(app_context_available),
                                        )
                                        .color(palette.muted),
                                    );
                                    ui.label(
                                        RichText::new(self.strings.onboarding_app_caveat())
                                            .small()
                                            .color(palette.muted),
                                    );
                                    ui.add_space(10.0);
                                    ui.label(
                                        RichText::new(self.strings.onboarding_safety_title())
                                            .strong(),
                                    );
                                    ui.label(self.strings.onboarding_try_text());
                                    ui.add_space(8.0);
                                    if theme::primary_button(
                                        ui,
                                        palette,
                                        self.strings.create_test_snippet(),
                                    )
                                    .clicked()
                                    {
                                        self.create_test_snippet();
                                    }
                                    if ui.link(self.strings.onboarding_evdev_setup()).clicked() {
                                        self.evdev_setup_acknowledged = false;
                                        self.evdev_setup_open = true;
                                    }
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(self.strings.onboarding_certification_note())
                                            .small()
                                            .color(palette.warning),
                                    );
                                });
                            },
                        );
                    });
                    return;
                };
                if index >= self.config.expansion.len() && !self.new_draft {
                    self.set_selected_index(None);
                    self.draft = None;
                    ui.label(self.strings.selection_stale());
                    return;
                }
                if self.draft.is_none() {
                    self.draft = self.config.expansion.get(index).map(Draft::from_expansion);
                }
                let command_backed = self
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.command_enabled);
                let mut detect_app_clicked = false;
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt("editor_scroll")
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .fill(palette.surface)
                            .stroke(egui::Stroke::new(1.0, palette.border))
                            .corner_radius(egui::CornerRadius::same(10))
                            .inner_margin(egui::Margin::same(18))
                            .show(ui, |ui| {
                                theme::section_header(ui, "", self.strings.snippet_details());
                                ui.add_space(6.0);
                                let categories = self.categories();
                                let duplicate_trigger = self.draft.as_ref().is_some_and(|draft| {
                                    !draft.trigger.is_empty()
                                        && self.config.expansion.iter().enumerate().any(
                                            |(other_index, other)| {
                                                other_index != index
                                                    && other.trigger == draft.trigger
                                            },
                                        )
                                });
                                let strings = &self.strings;
                                let app_detecting = self
                                    .app_detection
                                    .as_ref()
                                    .is_some_and(|task| !task.cancelled.load(Ordering::Acquire));
                                let app_detection_busy = self.app_detection.is_some();
                                let mut cancel_detection = false;
                                let Some(draft) = self.draft.as_mut() else {
                                    ui.label(strings.draft_unavailable());
                                    return;
                                };
                                // A two-column grid rather than a stack of
                                // `horizontal` rows: with free-form rows every
                                // field started at a different x depending on
                                // how long its label happened to be in the
                                // selected language, which read as a form
                                // nobody had laid out.
                                egui::Grid::new("snippet_details_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 8.0])
                                    .show(ui, |ui| {
                                        ui.label(strings.trigger());
                                        ui.add(
                                            TextEdit::singleline(&mut draft.trigger)
                                                .hint_text(strings.trigger_hint())
                                                .font(egui::TextStyle::Monospace)
                                                .desired_width(f32::INFINITY),
                                        );
                                        ui.end_row();

                                        ui.label("");
                                        if duplicate_trigger {
                                            ui.colored_label(
                                                palette.danger,
                                                strings.duplicate_trigger(),
                                            );
                                        } else {
                                            ui.label(
                                                RichText::new(strings.trigger_tip())
                                                    .small()
                                                    .color(palette.muted),
                                            );
                                        }
                                        ui.end_row();

                                        ui.label(strings.description());
                                        ui.add(
                                            TextEdit::singleline(&mut draft.description)
                                                .desired_width(f32::INFINITY),
                                        );
                                        ui.end_row();

                                        ui.label(strings.tags());
                                        let mut remove_tag = None;
                                        ui.horizontal_wrapped(|ui| {
                                            for (index, tag) in draft.tags.iter_mut().enumerate() {
                                                ui.horizontal(|ui| {
                                                    ui.add(
                                                        TextEdit::multiline(tag)
                                                            .desired_rows(1)
                                                            .desired_width(120.0),
                                                    );
                                                    if ui.small_button("×").clicked() {
                                                        remove_tag = Some(index);
                                                    }
                                                });
                                            }
                                            if ui.small_button("+ Add tag").clicked() {
                                                draft.tags.push(String::new());
                                            }
                                        });
                                        if let Some(index) = remove_tag {
                                            draft.tags.remove(index);
                                        }
                                        ui.end_row();

                                        ui.label(strings.category());
                                        ui.horizontal(|ui| {
                                            let picker_width =
                                                if categories.is_empty() { 0.0 } else { 150.0 };
                                            ui.add(
                                                TextEdit::singleline(&mut draft.category)
                                                    .hint_text(strings.category_hint())
                                                    .desired_width(
                                                        (ui.available_width() - picker_width)
                                                            .max(120.0),
                                                    ),
                                            );
                                            if !categories.is_empty() {
                                                egui::ComboBox::from_id_salt("category_picker")
                                                    .selected_text(strings.existing())
                                                    .width(110.0)
                                                    .show_ui(ui, |ui| {
                                                        for category in &categories {
                                                            if ui
                                                                .selectable_label(
                                                                    draft.category == *category,
                                                                    category,
                                                                )
                                                                .clicked()
                                                            {
                                                                draft.category = category.clone();
                                                            }
                                                        }
                                                    });
                                            }
                                        });
                                        ui.end_row();

                                        ui.label(strings.app_filter());
                                        let mut remove_filter = None;
                                        ui.vertical(|ui| {
                                            for (index, filter) in
                                                draft.app_filter.iter_mut().enumerate()
                                            {
                                                ui.horizontal(|ui| {
                                                    ui.add(
                                                        TextEdit::multiline(filter)
                                                            .desired_rows(1)
                                                            .desired_width(220.0),
                                                    );
                                                    if ui.small_button("×").clicked() {
                                                        remove_filter = Some(index);
                                                    }
                                                });
                                            }
                                            ui.horizontal(|ui| {
                                                if ui.small_button("+ Add app").clicked() {
                                                    draft.app_filter.push(String::new());
                                                }
                                                if app_detecting {
                                                    ui.spinner();
                                                    ui.label(strings.detecting_app())
                                                        .on_hover_text(
                                                            strings.detect_app_tooltip(),
                                                        );
                                                    if ui.small_button(strings.cancel()).clicked() {
                                                        // The spawned thread is not joined/cancelled --
                                                        // it may itself be stuck in a hung D-Bus call --
                                                        // just stop waiting on it and discard whatever
                                                        // it eventually sends.
                                                        cancel_detection = true;
                                                    }
                                                } else if app_detection_busy {
                                                    ui.spinner();
                                                    ui.label(strings.stopping_app_detection())
                                                        .on_hover_text(
                                                            strings.detect_app_tooltip(),
                                                        );
                                                } else if ui
                                                    .button(strings.detect_app())
                                                    .on_hover_text(strings.detect_app_tooltip())
                                                    .clicked()
                                                {
                                                    detect_app_clicked = true;
                                                }
                                            });
                                        });
                                        if let Some(index) = remove_filter {
                                            draft.app_filter.remove(index);
                                        }
                                        ui.end_row();

                                        ui.label("");
                                        ui.label(
                                            RichText::new(if draft.app_filter.is_empty() {
                                                strings.app_filter_help()
                                            } else {
                                                strings.window_tracking_warning()
                                            })
                                            .small()
                                            .color(palette.muted),
                                        );
                                        ui.end_row();

                                        ui.label(strings.matching());
                                        ui.horizontal_wrapped(|ui| {
                                            ui.checkbox(&mut draft.enabled, strings.enabled());
                                            ui.separator();
                                            ui.radio_value(
                                                &mut draft.match_mode,
                                                MatchMode::Immediate,
                                                strings.immediate(),
                                            );
                                            ui.radio_value(
                                                &mut draft.match_mode,
                                                MatchMode::WordBoundary,
                                                strings.word_boundary(),
                                            );
                                            ui.separator();
                                            ui.checkbox(
                                                &mut draft.propagate_case,
                                                strings.propagate_case(),
                                            )
                                            .on_hover_text(strings.propagate_case_tooltip());
                                        });
                                        ui.end_row();
                                    });
                                if cancel_detection {
                                    if let Some(task) = self.app_detection.as_mut() {
                                        task.cancelled.store(true, Ordering::Release);
                                    }
                                }
                                let Some(draft) = self.draft.as_mut() else {
                                    return;
                                };
                                ui.add_space(8.0);
                                ui.label(self.strings.replacement());
                                ui.add(
                                    TextEdit::multiline(&mut draft.replacement)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_rows(9)
                                        .desired_width(f32::INFINITY),
                                );
                            });
                        if detect_app_clicked && self.app_detection.is_none() {
                            // Run entirely off the UI thread. Tracker setup has
                            // bounded D-Bus calls and a total readiness deadline,
                            // but a slow session bus can still consume several
                            // seconds. The receiver is polled below on every frame.
                            use wayexpand_backend_kwin_window::KwinWindowTracker;
                            use wayexpand_core::WindowTracker;
                            let (sender, receiver) = mpsc::channel();
                            let cancelled = Arc::new(AtomicBool::new(false));
                            let worker_cancel = Arc::clone(&cancelled);
                            self.app_detection = Some(AppDetectionTask {
                                receiver,
                                cancelled,
                            });
                            thread::spawn(move || {
                                let detection = match KwinWindowTracker::new_cancellable(Some(
                                    &worker_cancel,
                                )) {
                                    Ok(mut tracker) => {
                                        if worker_cancel.load(Ordering::Acquire) {
                                            let _ = sender.send(AppDetection::Unavailable);
                                            return;
                                        }
                                        match tracker
                                            .next_window_timeout(std::time::Duration::from_secs(5))
                                        {
                                            Ok(Some(Some(window))) => AppDetection::Found(window),
                                            Ok(Some(None)) => AppDetection::NoWindow,
                                            Ok(None) | Err(_) => AppDetection::Unavailable,
                                        }
                                    }
                                    Err(_) => AppDetection::Unavailable,
                                };
                                // The GUI may have given up waiting (Cancel, or the
                                // window closed) by the time this send happens; that is
                                // not an error, there is simply nothing left to notify.
                                let _ = sender.send(detection);
                            });
                        }
                        let detection_cancelled = self
                            .app_detection
                            .as_ref()
                            .is_some_and(|task| task.cancelled.load(Ordering::Acquire));
                        let detection_result = self
                            .app_detection
                            .as_ref()
                            .map(|task| task.receiver.try_recv());
                        if let Some(detection_result) = detection_result {
                            match detection_result {
                                Ok(AppDetection::Found(window)) => {
                                    self.app_detection = None;
                                    if !detection_cancelled {
                                        let value =
                                            window.app_id.or(window.title).unwrap_or_default();
                                        if value.is_empty() {
                                            self.status = Status::warning(
                                                self.strings.status_window_unidentified(),
                                            );
                                        } else {
                                            if let Some(draft) = self.draft.as_mut() {
                                                if !draft.app_filter.contains(&value) {
                                                    draft.app_filter.push(value.clone());
                                                    self.invalidate_preview();
                                                }
                                            }
                                            self.status = Status::success(
                                                self.strings.status_app_filter_added(&value),
                                            );
                                        }
                                    }
                                }
                                Ok(AppDetection::NoWindow) => {
                                    self.app_detection = None;
                                    if !detection_cancelled {
                                        self.status = Status::warning(
                                            self.strings.status_no_focused_window(),
                                        );
                                    }
                                }
                                Ok(AppDetection::Unavailable) => {
                                    self.app_detection = None;
                                    if !detection_cancelled {
                                        self.status = Status::warning(
                                            self.strings.status_detection_unavailable(),
                                        );
                                    }
                                }
                                Err(mpsc::TryRecvError::Empty) => {
                                    // Still waiting: request another repaint soon so
                                    // this gets polled promptly instead of only on the
                                    // next user-driven event, without busy-looping.
                                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                                }
                                Err(mpsc::TryRecvError::Disconnected) => {
                                    // The sender was dropped without sending, which
                                    // should not happen (the spawned thread always
                                    // sends before exiting) -- treat it the same as an
                                    // explicit Unavailable rather than waiting forever.
                                    self.app_detection = None;
                                    if !detection_cancelled {
                                        self.status =
                                            Status::error(self.strings.status_detection_failed());
                                    }
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
                        ui.collapsing(self.strings.template_variables(), |ui| {
                            ui.label(
                                RichText::new(self.strings.template_help())
                                    .small()
                                    .color(palette.muted),
                            );
                            ui.horizontal_wrapped(|ui| {
                                for (variable, description) in TEMPLATE_VARIABLES {
                                    if theme::secondary_button(ui, palette, variable)
                                        .on_hover_text(*description)
                                        .clicked()
                                    {
                                        if let Some(draft) = self.draft.as_mut() {
                                            draft.replacement.push_str(variable);
                                        }
                                    }
                                }
                            });
                        });
                        ui.horizontal(|ui| {
                            ui.label(self.strings.preview_app());
                            ui.add(
                                TextEdit::singleline(&mut self.preview_app)
                                    .hint_text(self.strings.preview_app_hint())
                                    .desired_width(300.0),
                            );
                        });
                        ui.add_space(4.0);
                        ui.collapsing(self.strings.dynamic_command(), |ui| {
                            let Some(draft) = self.draft.as_mut() else {
                                ui.label(self.strings.draft_unavailable());
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
                                        TextEdit::singleline(&mut draft.command_timeout_ms)
                                            .desired_width(90.0),
                                    );
                                    ui.label(self.strings.cache_ms());
                                    ui.add(
                                        TextEdit::singleline(&mut draft.command_cache_ms)
                                            .desired_width(90.0),
                                    );
                                });
                                ui.label(self.strings.arguments());
                                let mut remove_arg = None;
                                let mut move_arg = None;
                                for index in 0..draft.command_args.len() {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            TextEdit::multiline(&mut draft.command_args[index])
                                                .desired_rows(1)
                                                .desired_width(ui.available_width() - 108.0),
                                        );
                                        if ui.small_button("↑").on_hover_text("Move up").clicked()
                                            && index > 0
                                        {
                                            move_arg = Some((index, index - 1));
                                        }
                                        if ui.small_button("↓").on_hover_text("Move down").clicked()
                                            && index + 1 < draft.command_args.len()
                                        {
                                            move_arg = Some((index, index + 1));
                                        }
                                        if ui
                                            .small_button("×")
                                            .on_hover_text("Remove argument")
                                            .clicked()
                                        {
                                            remove_arg = Some(index);
                                        }
                                    });
                                }
                                if let Some(index) = remove_arg {
                                    draft.command_args.remove(index);
                                }
                                if let Some((from, to)) = move_arg {
                                    draft.command_args.swap(from, to);
                                }
                                if ui.small_button("+ Add argument").clicked() {
                                    draft.command_args.push(String::new());
                                }
                                ui.horizontal(|ui| {
                                    ui.label("Environment");
                                    egui::ComboBox::from_id_salt("command_environment")
                                        .selected_text(match draft.command_environment {
                                            wayexpand_core::CommandEnvironment::Minimal => {
                                                "Minimal"
                                            }
                                            wayexpand_core::CommandEnvironment::Inherit => {
                                                "Inherit"
                                            }
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut draft.command_environment,
                                                wayexpand_core::CommandEnvironment::Minimal,
                                                "Minimal",
                                            );
                                            ui.selectable_value(
                                                &mut draft.command_environment,
                                                wayexpand_core::CommandEnvironment::Inherit,
                                                "Inherit",
                                            );
                                        });
                                });
                                ui.label("Pass environment variables");
                                let mut remove_env = None;
                                for index in 0..draft.command_pass_env.len() {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            TextEdit::multiline(&mut draft.command_pass_env[index])
                                                .desired_rows(1)
                                                .desired_width(ui.available_width() - 38.0),
                                        );
                                        if ui
                                            .small_button("×")
                                            .on_hover_text("Remove environment variable")
                                            .clicked()
                                        {
                                            remove_env = Some(index);
                                        }
                                    });
                                }
                                if let Some(index) = remove_env {
                                    draft.command_pass_env.remove(index);
                                }
                                if ui.small_button("+ Add environment variable").clicked() {
                                    draft.command_pass_env.push(String::new());
                                }
                            });
                        });
                        // Save and Delete are a pinned action bar under this
                        // scroll area (`render_editor_actions`): with a long
                        // replacement open they used to scroll out of reach,
                        // so the primary action of the window depended on
                        // where the user happened to be scrolled to.
                        ui.add_space(14.0);
                        theme::section_header(ui, "", self.strings.preview());
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(self.strings.input());
                            ui.add(
                                TextEdit::singleline(&mut self.preview_input)
                                    .hint_text(self.strings.input_hint())
                                    .desired_width(420.0),
                            );
                            if theme::secondary_button(ui, palette, self.strings.use_trigger())
                                .clicked()
                            {
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
                                    ui.label(self.strings.command_preview_help());
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        let preview_is_current =
                                            self.command_preview_key == Some(self.preview_revision);
                                        if ui
                                            .add_enabled(
                                                self.command_preview_receiver.is_none(),
                                                egui::Button::new(
                                                    if self.command_preview_receiver.is_some() {
                                                        self.strings.running()
                                                    } else {
                                                        self.strings.run_once()
                                                    },
                                                ),
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
                                                let shown: String = if output.chars().count() > 200
                                                {
                                                    output.chars().take(199).collect::<String>()
                                                        + "…"
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
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::TOP),
                                            |ui| {
                                                if ui
                                                    .small_button(self.strings.copy())
                                                    .on_hover_text(self.strings.copy_tooltip())
                                                    .clicked()
                                                {
                                                    ui.ctx().copy_text(preview_text.clone());
                                                    self.status = Status::success(
                                                        self.strings.status_preview_copied(),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                });
                        }
                        // The status line is a persistent bottom panel
                        // (`render_status_bar`) rather than the tail of this
                        // scroll area: a message about a failed save was
                        // previously only visible after scrolling down to it.
                        ui.add_space(4.0);
                    });
            });
    }

    /// The editor's pinned action bar. It is a panel rather than the last
    /// row of the editor's scroll area so the primary action stays on screen
    /// however far the snippet's replacement text scrolls.
    fn render_editor_actions(&mut self, root: &mut egui::Ui, palette: &Palette) {
        if self.selected_index().is_none() && !self.new_draft {
            return;
        }
        egui::Panel::bottom("editor_actions")
            // Without the rule the editor's content scrolls flush against the
            // buttons and the bar stops reading as a fixed surface.
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(22, 10))
                    .stroke(egui::Stroke::NONE),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, palette, self.strings.save_changes())
                        .on_hover_text(self.strings.save_tooltip())
                        .clicked()
                    {
                        self.save_selected();
                    }
                    if self.new_draft {
                        if theme::secondary_button(ui, palette, self.strings.cancel()).clicked() {
                            self.abandon_new_draft();
                            self.status = Status::info(self.strings.ready());
                        }
                    } else if theme::danger_button(ui, palette, self.strings.delete()).clicked() {
                        self.request_action(PendingAction::Delete);
                    }
                    if self.draft_is_dirty() {
                        theme::pill(
                            ui,
                            self.strings.unsaved_changes(),
                            palette.warning,
                            theme::tint(palette.warning, 38),
                        );
                    }
                });
            });
    }

    /// The always-visible status line: the outcome of the last action on
    /// the left, and which configuration file this window is editing on the
    /// right, so a second instance opened on a different file is never
    /// mistaken for the first.
    fn render_status_bar(&mut self, root: &mut egui::Ui, palette: &Palette) {
        let (text_color, accent) = self.status.colors(palette);
        egui::Panel::bottom("status")
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .fill(self.status.background(palette))
                    .inner_margin(egui::Margin::symmetric(18, 8))
                    .stroke(egui::Stroke::NONE),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let (bar, _) = ui.allocate_exact_size(
                        egui::Vec2::new(3.0, ui.text_style_height(&egui::TextStyle::Body)),
                        egui::Sense::hover(),
                    );
                    if let Some(accent) = accent {
                        ui.painter()
                            .rect_filled(bar, egui::CornerRadius::same(2), accent);
                    }
                    ui.label(RichText::new(self.status.text()).color(text_color));
                    if self.diagnostics_running || self.pending_control > 0 {
                        ui.spinner();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(self.path.display().to_string())
                                .monospace()
                                .small()
                                .color(palette.muted),
                        )
                        .on_hover_text(self.strings.configuration_file());
                    });
                });
            });
    }

    /// Keeps the window title in step with which file is open and whether it
    /// has unsaved edits, the way any other editor does, instead of showing
    /// a constant application name.
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());
        let title = if self.draft_is_dirty() {
            format!("• {name} — WayExpand")
        } else {
            format!("{name} — WayExpand")
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn render_pending_action(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.pending_action.is_some() {
            egui::Window::new(self.strings.unsaved_title())
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let action = match self.pending_action {
                        Some(PendingAction::Select(_)) => self.strings.unsaved_switching(),
                        Some(PendingAction::New) => self.strings.unsaved_creating(),
                        Some(PendingAction::Duplicate) => self.strings.unsaved_duplicating(),
                        Some(PendingAction::Delete) => self.strings.unsaved_deleting(),
                        Some(PendingAction::Reload) => self.strings.unsaved_reloading(),
                        Some(PendingAction::Undo) => self.strings.unsaved_undoing(),
                        Some(PendingAction::Close) => self.strings.unsaved_closing(),
                        None => "continuing",
                    };
                    if self.draft_is_dirty() {
                        ui.label(self.strings.save_before(action));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::primary_button(ui, palette, self.strings.save_continue())
                                .clicked()
                            {
                                self.save_and_execute_pending();
                            }
                            if theme::secondary_button(ui, palette, self.strings.discard())
                                .clicked()
                            {
                                self.discard_pending();
                            }
                            if theme::secondary_button(ui, palette, self.strings.cancel()).clicked()
                            {
                                self.pending_action = None;
                            }
                        });
                    } else if matches!(self.pending_action, Some(PendingAction::Delete)) {
                        ui.label(self.strings.delete_confirm());
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::danger_button(ui, palette, self.strings.delete_button())
                                .clicked()
                            {
                                self.pending_action = None;
                                self.execute_action(PendingAction::Delete);
                            }
                            if theme::secondary_button(ui, palette, self.strings.cancel()).clicked()
                            {
                                self.pending_action = None;
                            }
                        });
                    }
                });
        }
    }

    fn render_evdev_setup(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.evdev_setup_open {
            return;
        }
        egui::Window::new(self.strings.evdev_setup_title())
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(self.strings.evdev_setup_warning()).color(palette.warning));
                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.evdev_setup_acknowledged,
                    self.strings.evdev_setup_acknowledge(),
                );
                ui.add_space(8.0);
                ui.label(self.strings.evdev_setup_steps());
                if self.evdev_setup_acknowledged {
                    ui.horizontal(|ui| {
                        ui.monospace("wayexpand setup --mode maximum");
                        if ui.button(self.strings.copy()).clicked() {
                            ui.ctx().copy_text("wayexpand setup --mode maximum".into());
                            self.status = Status::info(self.strings.status_preview_copied());
                        }
                    });
                    ui.label(
                        RichText::new("This command is not run by the GUI.")
                            .small()
                            .color(palette.muted),
                    );
                }
                ui.add_space(8.0);
                if ui.button(self.strings.close()).clicked() {
                    self.evdev_setup_open = false;
                }
            });
    }
}

impl eframe::App for GuiApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        self.poll_runtime(ctx);
        self.poll_command_preview(ctx);
        self.reap_app_detection_without_editor(ctx);
        if self.theme_refresh_pending {
            theme::install_pack(ctx, self.colorpack, self.config.settings.font_scale);
            self.theme_refresh_pending = false;
        }
        let palette = Palette::for_pack(self.colorpack, self.dark_mode);
        // Any open dialog swallows the editor accelerators, so Ctrl+S in the
        // settings window does not silently save the snippet behind it.
        let modal_open = self.any_dialog_open() || self.pending_action.is_some();
        let (want_save, want_new, want_search, want_escape, close_requested) = ctx.input(|input| {
            (
                !modal_open && input.modifiers.command && input.key_pressed(egui::Key::S),
                !modal_open && input.modifiers.command && input.key_pressed(egui::Key::N),
                !modal_open && input.modifiers.command && input.key_pressed(egui::Key::F),
                input.key_pressed(egui::Key::Escape),
                input.viewport().close_requested(),
            )
        });
        if close_requested && self.draft_is_dirty() {
            // The user clicked the window's close button (or an OS-level
            // quit) with an unsaved draft open. Every other action that can
            // discard a draft (Select, Delete, Reload, Undo) already
            // confirms first; closing the whole app was the one silent
            // exit left. Cancel this close and route it through the same
            // Save/Discard/Cancel dialog; if confirmed, close_after_confirm
            // (below) re-issues the close next frame, by which point the
            // draft is no longer dirty so it goes through uncancelled.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_action(PendingAction::Close);
        }
        if self.close_after_confirm {
            self.close_after_confirm = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if want_save && self.selected.is_some() {
            self.save_selected();
        }
        if want_new {
            self.request_action(PendingAction::New);
        }
        if want_search {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(SEARCH_FIELD_SALT)));
        }
        if want_escape {
            self.close_topmost_dialog();
        }
        self.sync_window_title(ctx);
        self.render_toolbar(root, &palette);
        self.render_diagnostics(ctx, &palette);
        self.render_import_dialog(ctx, &palette);
        self.render_settings_dialog(ctx, &palette);
        // Panel order decides how the remaining space is carved up: the
        // status line spans the full width, the sidebar then claims the left
        // edge, and the editor's action bar sits above the status line but
        // only across the editor itself.
        self.render_status_bar(root, &palette);
        self.render_snippet_list(root, &palette);
        self.render_editor_actions(root, &palette);
        self.render_editor(root, &palette);
        self.render_pending_action(ctx, &palette);
        self.render_evdev_setup(ctx, &palette);
    }
}

fn expand_user_path(value: &str) -> PathBuf {
    expand_user_path_with_home(value, env::var_os("HOME").as_deref())
}

fn expand_user_path_with_home(value: &str, home: Option<&std::ffi::OsStr>) -> PathBuf {
    let Some(home) = home else {
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
    app.start_runtime()?;
    let saved_dark_mode = load_gui_prefs().dark_mode;
    let colorpack = app.colorpack;
    let icon = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/icon/hicolor/256x256/apps/wayexpand.png"
    ))
    .expect("bundled app icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([640.0, 460.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "WayExpand",
        options,
        Box::new(move |creation_context| {
            // Keep egui's bundled fonts first, then add only validated system
            // fallbacks so arrows, status symbols, and CJK snippet text do not
            // render as missing-glyph boxes on minimal desktop installations.
            fonts::install(&creation_context.egui_ctx);
            theme::install_pack(
                &creation_context.egui_ctx,
                colorpack,
                app.config.settings.font_scale,
            );
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
    use crate::status::StatusTone;
    use std::os::unix::fs::PermissionsExt;

    fn run_gui_test_frame(
        ctx: &egui::Context,
        input: egui::RawInput,
        draw: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(input, draw);
        // Headless tests inspect semantics/layout, not renderer texture uploads.
        // epaint 0.36 asserts if its unapplied font-atlas delta is dropped.
        output.textures_delta.clear();
        output
    }

    fn import_expansion(trigger: &str, replacement: &str) -> wayexpand_core::ExpansionConfig {
        wayexpand_core::ExpansionConfig {
            id: wayexpand_core::ExpansionConfig::new_id(),
            trigger: trigger.into(),
            replacement: replacement.into(),
            description: String::new(),
            tags: vec!["imported".into()],
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        }
    }

    #[test]
    fn import_merge_deduplicates_and_keeps_existing_trigger_conflicts() {
        let current = Config {
            expansion: vec![
                import_expansion(":same", "identical"),
                import_expansion(":conflict", "keep this"),
                wayexpand_core::ExpansionConfig {
                    trigger: ":hello".into(),
                    replacement: "case variant wins".into(),
                    propagate_case: true,
                    ..import_expansion(":hello", "case variant wins")
                },
            ],
            hotkey: Vec::new(),
            settings: wayexpand_core::Settings {
                max_buffer_chars: 2048,
                ..Default::default()
            },
            organization: wayexpand_core::OrganizationPolicy::default(),
        };
        let imported = Config {
            expansion: vec![
                import_expansion(":same", "identical"),
                import_expansion(":conflict", "do not replace"),
                import_expansion(":HELLO", "must not collide"),
                import_expansion(":new", "append this"),
            ],
            hotkey: Vec::new(),
            settings: wayexpand_core::Settings::default(),
            organization: wayexpand_core::OrganizationPolicy::default(),
        };

        let (merged, stats) = import::merge_imported_expansions(&current, &imported);
        assert_eq!(stats.added, 1);
        assert_eq!(stats.identical_duplicates, 1);
        assert_eq!(stats.conflicts_kept, 2);
        assert_eq!(merged.expansion.len(), 4);
        assert_eq!(merged.expansion[1].replacement, "keep this");
        assert_eq!(merged.expansion[2].replacement, "case variant wins");
        assert_eq!(merged.settings.max_buffer_chars, 2048);
    }

    fn draft() -> Draft {
        Draft {
            trigger: ":cmd".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            replacement: "fallback".into(),
            enabled: true,
            match_mode: MatchMode::Immediate,
            propagate_case: false,
            command_enabled: true,
            command_program: "uname".into(),
            command_args: vec!["-s".into(), "-r".into()],
            command_timeout_ms: "500".into(),
            command_cache_ms: "1000".into(),
            command_environment: wayexpand_core::CommandEnvironment::default(),
            command_pass_env: Vec::new(),
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
    fn command_editor_preserves_advanced_environment_settings() {
        let mut source = wayexpand_core::ExpansionConfig {
            id: wayexpand_core::ExpansionConfig::new_id(),
            trigger: ":foo".into(),
            replacement: String::new(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        };
        source.command = Some(wayexpand_core::CommandConfig {
            program: "/usr/bin/foo ".into(),
            args: vec![String::new(), " foo ".into(), "hello\nworld".into()],
            timeout_ms: 900,
            cache_ms: 0,
            environment: wayexpand_core::CommandEnvironment::Inherit,
            pass_env: vec![" DISPLAY ".into(), "TEAM\nID".into()],
        });
        let mut form = Draft::from_expansion(&source);
        assert!(form.matches_command(source.command.as_ref()));
        assert_eq!(form.command_args, source.command.as_ref().unwrap().args);
        assert_eq!(
            form.command_pass_env,
            source.command.as_ref().unwrap().pass_env
        );
        form.description = "Edited description".into();
        assert!(form.matches_command(source.command.as_ref()));
        assert_eq!(form.command_config().unwrap(), source.command);
        form.command_program.clear();
        assert!(!form.matches_command(source.command.as_ref()));
        assert!(form.command_config().is_err());
    }

    #[test]
    fn new_snippet_stays_out_of_config_until_a_nonempty_replacement_is_saved() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-new-draft-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        let original_file = fs::read(&path).unwrap();

        app.create_new_snippet();

        assert!(app.new_draft);
        assert!(app.draft_is_dirty());
        assert!(app.config.expansion.is_empty());
        assert_eq!(fs::read(&path).unwrap(), original_file);
        assert!(app.draft.as_ref().unwrap().replacement.is_empty());
        assert!(app.draft.as_ref().unwrap().enabled);

        app.save_selected();
        assert!(app.new_draft);
        assert!(app.config.expansion.is_empty());
        assert_eq!(fs::read(&path).unwrap(), original_file);

        app.draft.as_mut().unwrap().replacement = "Finished snippet".into();
        assert_eq!(app.preview(), "Finished snippet");
        app.save_selected();

        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.expansion.len(), 1);
        assert_eq!(saved.expansion[0].trigger, ":new");
        assert_eq!(saved.expansion[0].replacement, "Finished snippet");
        assert!(saved.expansion[0].enabled);
        assert!(!app.new_draft);
        assert!(!app.draft_is_dirty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn discarding_a_new_snippet_restores_the_previous_selection_without_writing() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-discard-new-draft-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let config = Config {
            expansion: vec![import_expansion(":existing", "Existing text")],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        let original_file = fs::read(&path).unwrap();

        app.create_new_snippet();
        app.draft.as_mut().unwrap().replacement = "discard me".into();
        app.request_action(PendingAction::Select(0));
        assert_eq!(app.pending_action, Some(PendingAction::Select(0)));
        app.discard_pending();

        assert!(!app.new_draft);
        assert_eq!(app.selected_index(), Some(0));
        assert_eq!(app.draft.as_ref().unwrap().trigger, ":existing");
        assert_eq!(app.config.expansion.len(), 1);
        assert_eq!(fs::read(&path).unwrap(), original_file);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalid_command_draft_is_not_mistaken_for_clean_none() {
        let source = wayexpand_core::ExpansionConfig {
            id: wayexpand_core::ExpansionConfig::new_id(),
            trigger: ":foo".into(),
            replacement: String::new(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        };
        let mut form = Draft::from_expansion(&source);
        form.command_enabled = true;
        assert!(!form.matches_command(None));
        assert!(form.command_config().is_err());
    }

    #[test]
    fn editor_draft_preserves_tag_and_app_filter_tokens_verbatim() {
        let source = wayexpand_core::ExpansionConfig {
            id: wayexpand_core::ExpansionConfig::new_id(),
            trigger: ":foo".into(),
            replacement: String::new(),
            description: String::new(),
            tags: vec!["customer, west".into(), " email ".into()],
            category: String::new(),
            app_filter: vec!["org.example, beta".into(), "browser".into()],
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        };
        let form = Draft::from_expansion(&source);
        assert_eq!(form.tags, source.tags);
        assert_eq!(form.app_filter, source.app_filter);
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
    fn cancelling_app_detection_keeps_the_worker_until_it_finishes() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-detection-{}.toml",
            std::process::id()
        ));
        let mut app = GuiApp::load(path.clone()).unwrap();
        let (_sender, receiver) = mpsc::channel();
        app.app_detection = Some(AppDetectionTask {
            receiver,
            cancelled: Arc::new(AtomicBool::new(false)),
        });

        app.cancel_app_detection();

        assert!(app
            .app_detection
            .as_ref()
            .is_some_and(|task| task.cancelled.load(Ordering::Acquire)));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_configuration_is_initialized_without_replacing_existing_files() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-onboarding-run-{}.toml",
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
                    id: ExpansionConfig::new_id(),
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
                    id: ExpansionConfig::new_id(),
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
            organization: OrganizationPolicy::default(),
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
    fn gui_refuses_to_overwrite_an_external_config_change() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-external-change-{}.toml",
            std::process::id()
        ));
        let config = Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
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
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        let _ = fs::remove_file(&path);
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        app.draft.as_mut().unwrap().replacement = "from gui".into();

        let mut external = config.clone();
        external.expansion[0].replacement = "from external editor".into();
        external.save_atomic(&path).unwrap();

        app.save_selected();

        assert_eq!(
            Config::load(&path).unwrap().expansion[0].replacement,
            "from external editor"
        );
        assert_eq!(app.status.tone_for_test(), status::StatusTone::Warning);
        assert!(app.status.text().contains("changed outside WayExpand"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_reload_leaves_the_loaded_editor_snapshot_untouched() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-failed-reload-{}.toml",
            std::process::id()
        ));
        let config = Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":before".into(),
                replacement: "original text".into(),
                description: "original description".into(),
                tags: vec!["preserve".into()],
                category: "original category".into(),
                app_filter: vec!["org.example.Editor".into()],
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        let _ = fs::remove_file(&path);
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        let original_config = app.config.clone();
        let original_document = app.config_document.to_string();
        let original_revision = app.config_revision.clone();
        let original_selected_id = app.selected_id.clone();
        let original_draft = app.draft.as_ref().unwrap().clone();
        fs::write(&path, "[[expansion]\nthis is not valid TOML").unwrap();

        let (sender, diagnostics_sender, receiver) = runtime::start().unwrap();
        app.runtime_sender = Some(sender);
        app.diagnostics_sender = Some(diagnostics_sender);
        app.runtime_receiver = Some(receiver);
        app.perform_reload();
        assert!(app.pending_reload_revision.is_some());
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.pending_reload_revision.is_some() {
            app.poll_runtime(&ctx);
            assert!(
                Instant::now() < deadline,
                "background reload did not finish"
            );
            thread::sleep(Duration::from_millis(5));
        }

        assert_eq!(
            format!("{:?}", app.config),
            format!("{:?}", original_config)
        );
        assert_eq!(app.config_document.to_string(), original_document);
        assert_eq!(app.config_revision, original_revision);
        assert_eq!(app.selected_id, original_selected_id);
        let draft = app.draft.as_ref().unwrap();
        assert_eq!(draft.trigger, original_draft.trigger);
        assert_eq!(draft.description, original_draft.description);
        assert_eq!(draft.replacement, original_draft.replacement);
        assert_eq!(draft.tags, original_draft.tags);
        assert_eq!(draft.app_filter, original_draft.app_filter);
        assert_eq!(app.preview_input, original_draft.trigger);
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.status.tone_for_test(), status::StatusTone::Error);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reload_completion_cannot_overwrite_edits_made_while_loading() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-reload-race-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let config = Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":before".into(),
                replacement: "original".into(),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        let original_revision = app.config_revision.clone();

        let mut external_config = config;
        external_config.expansion[0].replacement = "external version".into();
        external_config.save_atomic(&path).unwrap();
        let loaded = Config::load_versioned(&path).unwrap();
        let snapshot = runtime::ReloadSnapshot {
            document: persistence::read_config_document(loaded.source()).unwrap(),
            search_index: library::SearchIndex::new(&loaded.config),
            revision: loaded.revision,
            config: loaded.config,
        };
        app.draft.as_mut().unwrap().replacement = "local unsaved edit".into();

        let (completion_sender, completion_receiver) = mpsc::channel();
        app.runtime_receiver = Some(completion_receiver);
        app.pending_reload_revision = Some(original_revision.clone());
        app.pending_control = 1;
        completion_sender
            .send(runtime::Completion::ConfigReloaded(Box::new(Ok(snapshot))))
            .unwrap();
        app.poll_runtime(&egui::Context::default());

        assert_eq!(app.config_revision, original_revision);
        assert_eq!(app.config.expansion[0].replacement, "original");
        assert_eq!(
            app.draft.as_ref().unwrap().replacement,
            "local unsaved edit"
        );
        assert_eq!(app.status.tone_for_test(), StatusTone::Warning);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn gui_core_revision_guard_catches_a_write_after_the_early_check() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-revision-race-{}.toml",
            std::process::id()
        ));
        let config = Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":one".into(),
                replacement: "initial".into(),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        let _ = fs::remove_file(&path);
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        let mut external = config.clone();
        external.expansion[0].replacement = "external edit".into();
        external.save_atomic(&path).unwrap();

        let mut stale_candidate = app.config.clone();
        stale_candidate.expansion[0].replacement = "stale GUI edit".into();
        assert!(app
            .save_config_candidate(&stale_candidate)
            .unwrap_err()
            .contains("changed externally"));
        assert_eq!(
            Config::load(&path).unwrap().expansion[0].replacement,
            "external edit"
        );
        assert_eq!(app.config.expansion[0].replacement, "initial");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn import_path_expands_home_prefix_without_shell_evaluation() {
        assert_eq!(
            expand_user_path_with_home(
                "~/matches.yml",
                Some(std::ffi::OsStr::new("/tmp/wayexpand-home")),
            ),
            PathBuf::from("/tmp/wayexpand-home/matches.yml")
        );
        assert_eq!(
            expand_user_path_with_home(
                "/tmp/matches.yml",
                Some(std::ffi::OsStr::new("/tmp/wayexpand-home")),
            ),
            PathBuf::from("/tmp/matches.yml")
        );
        assert_eq!(
            expand_user_path_with_home("~/matches.yml", None),
            PathBuf::from("~/matches.yml")
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
                expansion: vec![ExpansionConfig {
                    id: ExpansionConfig::new_id(),
                    trigger: ":undo-test".into(),
                    replacement: "previous value".into(),
                    description: String::new(),
                    tags: Vec::new(),
                    category: String::new(),
                    app_filter: Vec::new(),
                    match_mode: MatchMode::Immediate,
                    command: None,
                    enabled: true,
                    propagate_case: false,
                }],
                hotkey: Vec::new(),
                settings: Settings::default(),
                organization: OrganizationPolicy::default(),
            });
        }
        assert_eq!(app.undo.len(), MAX_UNDO_HISTORY);
        assert!(app.undo_bytes <= MAX_UNDO_BYTES);
        assert_eq!(
            app.undo_bytes,
            app.undo
                .iter()
                .map(|entry| entry.estimated_bytes)
                .sum::<usize>()
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn undo_history_respects_its_byte_budget() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-undo-bytes-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        for _ in 0..=MAX_UNDO_HISTORY {
            app.remember_undo(Config {
                expansion: vec![ExpansionConfig {
                    id: ExpansionConfig::new_id(),
                    trigger: ":large-undo".into(),
                    replacement: "x".repeat(300_000),
                    description: String::new(),
                    tags: Vec::new(),
                    category: String::new(),
                    app_filter: Vec::new(),
                    match_mode: MatchMode::Immediate,
                    command: None,
                    enabled: true,
                    propagate_case: false,
                }],
                hotkey: Vec::new(),
                settings: Settings::default(),
                organization: OrganizationPolicy::default(),
            });
        }
        assert!(app.undo.len() < MAX_UNDO_HISTORY);
        assert!(app.undo_bytes <= MAX_UNDO_BYTES);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn saved_snippet_edit_can_be_undone_back_to_disk() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-undo-save-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let config = Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":undo-save".into(),
                replacement: "before".into(),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        app.config.expansion[0].replacement = "after".into();
        let previous = Config::load(&path).unwrap();
        app.remember_undo(previous);

        app.undo();

        assert_eq!(app.config.expansion[0].replacement, "before");
        assert_eq!(
            Config::load(&path).unwrap().expansion[0].replacement,
            "before"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn undo_delta_restores_modified_deleted_inserted_and_reordered_snippets() {
        let existing = ExpansionConfig {
            id: "stable-a".into(),
            trigger: ":before".into(),
            replacement: "old text".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        };
        let deleted = ExpansionConfig {
            id: "stable-b".into(),
            trigger: ":deleted".into(),
            replacement: "restore me".into(),
            ..existing.clone()
        };
        let mut modified = existing.clone();
        modified.trigger = ":after".into();
        modified.replacement = "new text".into();
        let inserted = ExpansionConfig {
            id: "stable-c".into(),
            trigger: ":inserted".into(),
            ..existing.clone()
        };
        let previous = Config {
            expansion: vec![existing.clone(), deleted.clone()],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        let current = Config {
            expansion: vec![inserted, modified],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };

        let entry = UndoEntry::between(&previous, &current).unwrap();
        let restored = entry.restore(&current).unwrap();
        assert_eq!(restored.expansion, [existing, deleted]);
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
        app.save_settings(&egui::Context::default());
        assert_eq!(app.config.settings.max_buffer_chars, 128);
        assert_eq!(app.status.tone_for_test(), status::StatusTone::Error);
        assert!(app.status.text().contains("outside the allowed range"));
        // The dialog keeps the reason next to the field that caused it, not
        // only on the status line at the far edge of the window.
        assert!(app
            .settings_error
            .as_deref()
            .is_some_and(|error| error.contains("outside the allowed range")));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_non_numeric_buffer_limit_is_reported_without_touching_the_configuration() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-gui-buffer-{}.toml", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        let before = app.config.settings.max_buffer_chars;
        app.settings_open = true;

        app.settings_buffer = "half a screen".into();
        app.save_settings(&egui::Context::default());

        assert_eq!(app.config.settings.max_buffer_chars, before);
        assert_eq!(app.status.tone_for_test(), status::StatusTone::Error);
        // The dialog stays open so the value can be corrected in place.
        assert!(app.settings_open);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn escape_closes_one_dialog_at_a_time_and_keeps_a_loaded_import() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-gui-dialogs-{}.toml", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        app.diagnostics_open = true;
        app.import_open = true;
        app.import_preview = Some((
            app.config.clone(),
            wayexpand_core::EspansoImportReport::default(),
        ));
        app.settings_open = true;
        assert!(app.any_dialog_open());

        app.close_topmost_dialog();
        assert!(!app.settings_open);
        assert!(app.import_open);

        // The first Escape on the import dialog drops the preview, not the
        // dialog: a loaded library is expensive to reproduce.
        app.close_topmost_dialog();
        assert!(app.import_open);
        assert!(app.import_preview.is_none());

        app.close_topmost_dialog();
        assert!(!app.import_open);
        assert!(app.diagnostics_open);

        app.close_topmost_dialog();
        assert!(!app.any_dialog_open());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn the_window_title_names_the_open_file_and_marks_unsaved_edits() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-gui-title-{}.toml", std::process::id()));
        let config = Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
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
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        let _ = fs::remove_file(&path);
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        let ctx = egui::Context::default();

        app.sync_window_title(&ctx);
        let clean = app.window_title.clone();
        assert!(clean.contains(path.file_name().unwrap().to_str().unwrap()));
        assert!(!clean.starts_with('•'));

        app.draft.as_mut().unwrap().replacement = "changed".into();
        app.sync_window_title(&ctx);
        assert!(app.window_title.starts_with('•'));

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_font_scale_change_persists_without_consuming_the_undo_history() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-fontscale-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        let ctx = egui::Context::default();

        app.apply_font_scale(&ctx, FontScale::Large);

        assert_eq!(app.config.settings.font_scale, FontScale::Large);
        assert_eq!(app.settings_font_scale, FontScale::Large);
        assert!(app.undo.is_empty());
        assert_eq!(
            Config::load(&path).unwrap().settings.font_scale,
            FontScale::Large
        );
        fs::remove_file(path).unwrap();
    }

    /// Renders every panel and dialog headlessly. Layout code is not
    /// otherwise exercised by the unit tests, so this is what catches an
    /// out-of-range index, a mismatched `Grid`/`ScrollArea` id, or a
    /// borrow-order mistake in a dialog that is only reachable by clicking.
    #[test]
    fn every_panel_and_dialog_renders_for_both_languages_and_settings_tabs() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-gui-render-{}.toml", std::process::id()));
        let config = Config {
            expansion: vec![
                ExpansionConfig {
                    id: ExpansionConfig::new_id(),
                    trigger: ":plain".into(),
                    replacement: "plain text".into(),
                    description: "A plain snippet".into(),
                    tags: vec!["demo".into()],
                    category: "email".into(),
                    app_filter: vec!["konsole".into()],
                    match_mode: MatchMode::Immediate,
                    command: None,
                    enabled: true,
                    propagate_case: false,
                },
                ExpansionConfig {
                    id: ExpansionConfig::new_id(),
                    trigger: ":cmd".into(),
                    replacement: "fallback".into(),
                    description: String::new(),
                    tags: Vec::new(),
                    category: String::new(),
                    app_filter: Vec::new(),
                    match_mode: MatchMode::WordBoundary,
                    command: Some(wayexpand_core::CommandConfig {
                        program: "uname".into(),
                        args: vec!["-s".into()],
                        timeout_ms: 500,
                        cache_ms: 0,
                        environment: wayexpand_core::CommandEnvironment::default(),
                        pass_env: Vec::new(),
                    }),
                    enabled: false,
                    propagate_case: true,
                },
            ],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        let _ = fs::remove_file(&path);
        config.save_atomic(&path).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        app.diagnostics_open = true;
        app.daemon_reachable = Some(true);
        app.daemon_capabilities = Some(runtime::DaemonCapabilities {
            capture_sensitive_focus: Some(false),
            capture_exclusive: Some(false),
            capture_reliable_key_state: Some(true),
            capture_key_passthrough: Some(false),
            capture_composition_aware: Some(false),
            window_tracker_connected: Some(true),
            inject_atomic_replace: Some(false),
            inject_full_unicode: Some(true),
            inject_cursor_reposition: Some(true),
            inject_key_passthrough: Some(true),
        });
        app.import_open = true;
        app.settings_open = true;
        app.pending_action = Some(PendingAction::Delete);

        let ctx = egui::Context::default();
        for language in [Language::English, Language::German] {
            // Set the pair directly rather than through `set_language`: that
            // would persist to the real user preferences file.
            app.language = language;
            app.strings.set_language(language);
            for tab in [SettingsTab::Appearance, SettingsTab::Engine] {
                app.settings_tab = tab;
                for width in [420.0, 640.0, 980.0] {
                    for selected in [Some(0), Some(1), None] {
                        app.set_selected_index(selected);
                        app.draft = selected
                            .map(|index| Draft::from_expansion(&app.config.expansion[index]));
                        let input = egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 760.0),
                            )),
                            ..Default::default()
                        };
                        let _ = run_gui_test_frame(&ctx, input, |ui| {
                            let frame_ctx = ui.ctx().clone();
                            let palette = Palette::for_pack(app.colorpack, app.dark_mode);
                            app.sync_window_title(&frame_ctx);
                            app.render_toolbar(ui, &palette);
                            app.render_diagnostics(&frame_ctx, &palette);
                            app.render_import_dialog(&frame_ctx, &palette);
                            app.render_settings_dialog(&frame_ctx, &palette);
                            app.render_status_bar(ui, &palette);
                            app.render_snippet_list(ui, &palette);
                            app.render_editor_actions(ui, &palette);
                            app.render_editor(ui, &palette);
                            app.render_pending_action(&frame_ctx, &palette);
                        });
                    }
                }
            }
        }
        // Exercise the narrowest supported desktop window with 200% text;
        // this is a render smoke test, not a pixel-perfect clipping oracle.
        theme::install_pack(&ctx, app.colorpack, FontScale::Huge);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 480.0),
            )),
            ..Default::default()
        };
        let _ = run_gui_test_frame(&ctx, input, |ui| {
            let frame_ctx = ui.ctx().clone();
            let palette = Palette::for_pack(app.colorpack, app.dark_mode);
            app.render_toolbar(ui, &palette);
            app.render_diagnostics(&frame_ctx, &palette);
            app.render_import_dialog(&frame_ctx, &palette);
            app.render_settings_dialog(&frame_ctx, &palette);
            app.render_status_bar(ui, &palette);
            app.render_snippet_list(ui, &palette);
            app.render_editor_actions(ui, &palette);
            app.render_editor(ui, &palette);
            app.render_pending_action(&frame_ctx, &palette);
        });
        app.create_new_snippet();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(420.0, 760.0),
            )),
            ..Default::default()
        };
        let _ = run_gui_test_frame(&ctx, input, |ui| {
            let palette = Palette::for_pack(app.colorpack, app.dark_mode);
            app.render_editor_actions(ui, &palette);
            app.render_editor(ui, &palette);
        });
        assert!(app.new_draft);
        assert_eq!(app.config.expansion.len(), 2);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn first_run_screen_renders_and_creates_a_real_test_expansion() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-first-run-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        assert!(app.config.expansion.is_empty());
        let ctx = egui::Context::default();
        for language in [Language::English, Language::German] {
            app.language = language;
            app.strings.set_language(language);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 600.0),
                )),
                ..Default::default()
            };
            let _ = run_gui_test_frame(&ctx, input, |ui| {
                let palette = Palette::for_pack(app.colorpack, app.dark_mode);
                app.render_editor(ui, &palette);
            });
        }
        app.create_test_snippet();
        assert_eq!(app.config.expansion.len(), 1, "{}", app.status.text());
        assert_eq!(app.config.expansion[0].trigger, ":wayexpand-test");
        assert_eq!(app.config.expansion[0].replacement, "WayExpand is working!");
        app.filter = "working".into();
        assert!(app.visible_indices().is_empty());
        app.search_fields.replacements = true;
        assert_eq!(app.visible_indices(), vec![0]);
        assert_eq!(Config::load(&path).unwrap().expansion.len(), 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn raw_input_setup_command_is_hidden_until_explicit_acknowledgement() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-evdev-consent-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        app.evdev_setup_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let render = |app: &mut GuiApp| {
            let output = run_gui_test_frame(&ctx, egui::RawInput::default(), |ui| {
                app.render_evdev_setup(
                    &ui.ctx().clone(),
                    &Palette::for_pack(app.colorpack, app.dark_mode),
                );
            });
            format!(
                "{:?}",
                output.platform_output.accesskit_update.unwrap().nodes
            )
        };

        let before_acknowledgement = render(&mut app);
        assert!(before_acknowledgement.contains("password-field"));
        assert!(!before_acknowledgement.contains("wayexpand setup --mode maximum"));

        app.evdev_setup_acknowledged = true;
        let after_acknowledgement = render(&mut app);
        assert!(after_acknowledgement.contains("wayexpand setup --mode maximum"));
        assert!(after_acknowledgement.contains("not run by the GUI"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_selection_left_behind_by_a_shrinking_library_is_reported_not_indexed() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-gui-stale-{}.toml", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        // A configuration reloaded from disk (or replaced by an import) can
        // be shorter than the one the selection was made against.
        app.selected = Some(4);

        assert!(app.selected_index().is_none());
        assert!(!app.draft_is_dirty());
        app.save_selected();
        app.duplicate_selected();
        app.perform_delete_selected();
        app.toggle_enabled(4);

        assert_eq!(app.status.tone_for_test(), status::StatusTone::Warning);
        assert!(app.config.expansion.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn switching_language_retranslates_the_status_line_wording() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-language-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();

        app.language = Language::German;
        app.strings.set_language(Language::German);
        app.undo.clear();
        app.undo();

        assert_eq!(app.status.tone_for_test(), status::StatusTone::Warning);
        assert_eq!(app.status.text(), "Nichts zum Rückgängigmachen");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn format_preserving_saves_keep_manual_comments() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-format-preservation-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let config = Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":sig".into(),
                replacement: "Regards".into(),
                description: "Signature".into(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        let text = format!(
            "# Maintained by the team; keep this note.\n\n{}",
            toml_edit::ser::to_string_pretty(&config).unwrap()
        );
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let mut app = GuiApp::load(path.clone()).unwrap();
        let mut candidate = app.config.clone();
        candidate.expansion[0].replacement = "Best regards".into();

        app.save_config_candidate(&candidate).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with("# Maintained by the team; keep this note."));
        assert!(saved.contains("replacement = \"Best regards\""));
        assert_eq!(
            Config::load(&path).unwrap().expansion[0].replacement,
            "Best regards"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn format_merge_matches_duplicate_triggers_in_order() {
        let old = persistence::read_config_document(
            "[[expansion]]\n# disabled duplicate one\ntrigger = ':dup'\nenabled = false\n\n[[expansion]]\n# disabled duplicate two\ntrigger = ':dup'\nenabled = false\n",
        )
        .unwrap();
        let new = persistence::read_config_document(
            "[[expansion]]\ntrigger = ':dup'\nenabled = false\n\n[[expansion]]\ntrigger = ':dup'\nenabled = false\n",
        )
        .unwrap();
        let merged = persistence::merge_config_document(old, new).to_string();
        assert!(
            merged.find("# disabled duplicate one").unwrap()
                < merged.find("# disabled duplicate two").unwrap()
        );
        assert_eq!(merged.matches("trigger = ':dup'").count(), 2);
    }

    #[test]
    fn format_merge_follows_stable_ids_when_triggers_change_and_reorder() {
        let old = persistence::read_config_document(
            "# note for A\n[[expansion]]\nid = '00000000-0000-4000-8000-000000000001'\ntrigger = ':old-a'\n\n# note for B\n[[expansion]]\nid = '00000000-0000-4000-8000-000000000002'\ntrigger = ':old-b'\n",
        )
        .unwrap();
        let new = persistence::read_config_document(
            "[[expansion]]\nid = '00000000-0000-4000-8000-000000000002'\ntrigger = ':new-b'\n\n[[expansion]]\nid = '00000000-0000-4000-8000-000000000001'\ntrigger = ':new-a'\n",
        )
        .unwrap();

        let merged = persistence::merge_config_document(old, new).to_string();
        let b_id = merged.find("00000000-0000-4000-8000-000000000002").unwrap();
        let b_note = merged.find("# note for B").unwrap();
        let a_id = merged.find("00000000-0000-4000-8000-000000000001").unwrap();
        let a_note = merged.find("# note for A").unwrap();
        assert!(b_note < b_id && b_id < a_note && a_note < a_id, "{merged}");
        assert!(merged.contains("trigger = ':new-b'"));
        assert!(merged.contains("trigger = ':new-a'"));
    }

    #[test]
    fn gui_selection_tracks_the_snippet_id_when_config_order_changes() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-selection-id-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut app = GuiApp::load(path.clone()).unwrap();
        app.config.expansion = vec![
            import_expansion(":first", "first"),
            import_expansion(":second", "second"),
        ];
        app.select(1);
        let selected_id = app.selected_id.clone().unwrap();
        app.config.expansion.swap(0, 1);

        assert_eq!(app.selected_index(), Some(0));
        assert_eq!(
            app.config.expansion[app.selected_index().unwrap()].id,
            selected_id
        );
        fs::remove_file(path).unwrap();
    }
}
