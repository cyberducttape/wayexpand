mod app;
mod colorpack;
mod diagnostics;
mod dialogs;
mod editor;
mod fonts;
mod form;
mod import;
mod lang;
mod library;
mod persistence;
mod picker;
mod playground;
mod preview;
mod runtime;
mod settings;
mod status;
mod theme;

use anyhow::{Context, Result};
use app::UndoEntry;
use colorpack::{ColorPack, ColorScheme};
use dialogs::{AppDetection, PendingAction};
use editor::{broker_action_ids, Draft};
use eframe::egui::{self, Color32, RichText, ScrollArea, TextEdit};
use lang::{Language, Strings};
use settings::{load_gui_prefs, save_gui_prefs};
use status::Status;
use std::{
    env,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use theme::Palette;
use wayexpand_backend_selection::{
    route_contract_for, Capabilities, RecommendedRoute, RouteBackend,
};
use wayexpand_core::{
    default_config_path, BackendState, BackendStatus, Config, ExpansionConfig, FontScale,
    MatchMode, OrganizationPolicy, Settings,
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
const MIN_FIELD_WIDTH: f32 = 120.0;
const REPLACEMENT_EDITOR_SALT: &str = "wayexpand-replacement-editor";
const SETUP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RouteRecommendation {
    capture: wayexpand_core::BackendKind,
    injection: wayexpand_core::BackendKind,
    capture_label: &'static str,
    injection_label: &'static str,
    capture_state: BackendState,
    injection_state: BackendState,
    focus_tracking: bool,
    sensitive_fields: bool,
    atomic_replace: bool,
}

fn route_recommendation(route: RecommendedRoute) -> Option<RouteRecommendation> {
    let contract = route.contract();
    let backend_kind = |backend: RouteBackend| match backend {
        RouteBackend::IBus | RouteBackend::InputMethodV2 => {
            wayexpand_core::BackendKind::InputMethodV2
        }
        RouteBackend::Evdev => wayexpand_core::BackendKind::Evdev,
        RouteBackend::Libei => wayexpand_core::BackendKind::Libei,
        RouteBackend::WlrootsVirtualKeyboard => wayexpand_core::BackendKind::WlrootsVirtualKeyboard,
    };
    let capture = backend_kind(contract.capture_backend()?);
    let injection = backend_kind(contract.injection_backend()?);
    Some(RouteRecommendation {
        capture,
        injection,
        capture_label: &contract.capture,
        injection_label: &contract.injection,
        capture_state: BackendState::Available,
        injection_state: BackendState::Available,
        focus_tracking: contract.focus_tracking,
        sensitive_fields: contract.sensitive_fields,
        atomic_replace: contract.atomic_replace,
    })
}
/// Built-in template variables offered as insert buttons. Their hover
/// descriptions are translated in `Strings::template_variable_description`.
const TEMPLATE_VARIABLES: &[&str] = &[
    "{{date}}",
    "{{time}}",
    "{{datetime}}",
    "{{date+1d}}",
    "{{cursor}}",
    "{{username}}",
    "{{hostname}}",
    "{{unix_timestamp}}",
    "{{newline}}",
    "{{tab}}",
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
    library_revision: u64,
    visible_indices_cache: Option<library::VisibleIndicesCache>,
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
    recommended_route: Option<RecommendedRoute>,
    selection_capabilities: Option<Capabilities>,
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
    /// Matcher preview state: the saved library expanding as the user types.
    playground: playground::Playground,
    try_live_open: bool,
    /// A running one-click "Turn on WayExpand" (`wayexpand setup --yes`).
    setup_task: Option<SetupTask>,
    pending_save: Option<PendingSave>,
    queued_save: Option<PendingSave>,
    next_save_id: u64,
}

enum SaveIntent {
    Settings,
    Import(String),
    Snippet {
        is_new: bool,
        index: usize,
    },
    Undo,
    Created,
    Duplicated,
    Deleted {
        index: usize,
        trigger: String,
    },
    Toggled {
        index: usize,
        enabled: bool,
        trigger: String,
    },
}

struct SetupTask {
    receiver: mpsc::Receiver<Result<String, String>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for SetupTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

struct PendingSave {
    request_id: u64,
    candidate: Config,
    preview_revision: u64,
    intent: SaveIntent,
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
        let prefs = load_gui_prefs();
        let settings_font_scale = prefs.font_scale;
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
            library_revision: 0,
            visible_indices_cache: None,
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
            daemon_reachable: None,
            route_state: None,
            diagnostics_open: false,
            evdev_setup_open: false,
            evdev_setup_acknowledged: false,
            daemon_status: strings.not_checked().into(),
            daemon_capabilities: None,
            fleet_status: strings.not_checked().into(),
            backend_status: Vec::new(),
            recommended_route: None,
            selection_capabilities: None,
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
            playground: playground::Playground::default(),
            try_live_open: true,
            setup_task: None,
            pending_save: None,
            queued_save: None,
            next_save_id: 1,
        })
    }
}

impl eframe::App for GuiApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        self.poll_runtime(ctx);
        self.poll_command_preview(ctx);
        self.poll_setup(ctx);
        self.reap_app_detection_without_editor(ctx);
        if self.theme_refresh_pending {
            theme::install_pack(ctx, self.colorpack, self.settings_font_scale);
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
        if want_save {
            self.save_shortcut();
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
        self.render_try_live_panel(root, &palette);
        self.render_editor(root, &palette);
        self.render_pending_action(ctx, &palette);
        self.render_evdev_setup(ctx, &palette);
    }
}

/// Show a path under the home directory as `~/...`, the way a shell would,
/// so the status line spends its width on the part that identifies the file.
fn home_relative_path(path: &Path, home: Option<&std::ffi::OsStr>) -> String {
    home.map(Path::new)
        .filter(|home| !home.as_os_str().is_empty())
        .and_then(|home| path.strip_prefix(home).ok())
        .map(|relative| format!("~/{}", relative.display()))
        .unwrap_or_else(|| path.display().to_string())
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
        println!(
            "Usage: wayexpand-gui [CONFIG]\n       wayexpand-gui --picker [CONFIG]\n       wayexpand-gui --form SPEC\n\nNative Wayland settings editor for WayExpand.\n\n--picker  Quick-insert window: search your snippets and press Enter to type\n          one into the app you were using. Bind it to a desktop shortcut.\n--form    Snippet form window; started by the daemon for snippets with fields."
        );
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("--form") {
        let spec = env::args()
            .nth(2)
            .ok_or_else(|| anyhow::anyhow!("--form needs a form specification"))?;
        return form::run(&spec);
    }
    if env::args().nth(1).as_deref() == Some("--picker") {
        let path = env::args()
            .nth(2)
            .map(PathBuf::from)
            .unwrap_or_else(default_config_path);
        return picker::run(path);
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
                app.settings_font_scale,
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
mod tests;
