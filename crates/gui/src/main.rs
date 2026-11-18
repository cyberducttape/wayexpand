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
mod recovery;
mod runtime;
mod screenshot;
mod settings;
mod setup_record;
mod status;
mod theme;

use anyhow::{Context, Result};
use app::UndoEntry;
use colorpack::{ColorPack, ColorScheme};
use dialogs::{AppDetection, PendingAction};
use editor::{broker_action_ids, broker_action_parameters, Draft};
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
    route_contract_by_id, Capabilities, RecommendedRoute, RouteBackend,
};
use wayexpand_core::{
    default_config_path, BackendState, BackendStatus, Config, ExpansionConfig, FontScale,
    MatchMode, OrganizationPolicy, Settings,
};

/// Use the same compositor and session-type evidence as `wayexpand setup`
/// when deciding whether a saved GNOME continuation belongs to this login.
/// `XDG_CURRENT_DESKTOP` is descriptive metadata and may be absent or stale;
/// it must not make the CLI resume setup while the GUI hides the continuation.
pub(crate) fn gnome_setup_resume_pending_for_current_session(path: &Path) -> bool {
    let is_gnome = wayexpand_backend_selection::Compositor::detect()
        == wayexpand_backend_selection::Compositor::Gnome;
    let is_wayland =
        env::var("XDG_SESSION_TYPE").is_ok_and(|session| session.eq_ignore_ascii_case("wayland"));
    wayexpand_core::gnome_ibus_setup_resume_pending_for_session(path, is_gnome, is_wayland)
}

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
/// Setup can wait on an administrator password prompt (pkexec) and on the
/// desktop starting IBus, so allow well over a minute.
const SETUP_TIMEOUT: Duration = Duration::from_secs(180);

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
    trigger_precondition: &'static str,
    output_transaction: &'static str,
    application_result: &'static str,
    certified: bool,
    recommended: bool,
    blocked_by_policy: bool,
}

fn route_recommendation(
    route: &'static wayexpand_backend_selection::RouteContract,
    route_plan: Option<&wayexpand_backend_selection::RoutePlan>,
    capabilities: Option<&Capabilities>,
    backend_status: &[BackendStatus],
    ibus_report: Option<&wayexpand_backend_ibus::chain::IbusChainReport>,
) -> Option<RouteRecommendation> {
    let capture_backend = route.capture_backend()?;
    let injection_backend = route.injection_backend()?;
    let backend_kind = |backend: RouteBackend| match backend {
        RouteBackend::IBus | RouteBackend::InputMethodV2 => {
            wayexpand_core::BackendKind::InputMethodV2
        }
        RouteBackend::Evdev => wayexpand_core::BackendKind::Evdev,
        RouteBackend::Libei => wayexpand_core::BackendKind::Libei,
        RouteBackend::WlrootsVirtualKeyboard => wayexpand_core::BackendKind::WlrootsVirtualKeyboard,
    };
    let planned = route_plan.and_then(|plan| {
        plan.routes
            .iter()
            .find(|planned| planned.contract.id == route.id)
    });
    let assessment = planned.map(|planned| &planned.assessment);
    let mut capture_state = route_component_state(
        capture_backend,
        true,
        capabilities,
        assessment,
        backend_status,
    );
    let mut injection_state = route_component_state(
        injection_backend,
        false,
        capabilities,
        assessment,
        backend_status,
    );
    // IBus is one route for both directions, but its readiness links differ:
    // registration/selection starts capture, while replacement consent gates
    // insertion. Do not copy a route-level `supported` result onto both.
    if route.id == "ibus" {
        if let Some(report) = ibus_report {
            use wayexpand_backend_ibus::chain::IbusCheckState as I;
            let mut capture_states = ["component", "registered", "daemon", "selected", "engine"]
                .into_iter()
                .map(|id| report.check(id).map_or(I::Unknown, |check| check.state))
                .collect::<Vec<_>>();
            // A shared component that points into a user's home directory is
            // not a durable, multi-user-safe setup even if the current engine
            // happens to be alive. Include this only when the diagnostic is
            // available so older/minimal reports remain explicitly unknown
            // through their existing checks rather than being misclassified.
            if let Some(check) = report.check("manifest-scope") {
                capture_states.push(check.state);
            }
            // GNOME has its own source list and active-source state. An IBus
            // daemon can be healthy while GNOME has not registered or chosen
            // WayExpand; don't label capture available in that state.
            if report.desktop == wayexpand_backend_ibus::chain::IbusDesktop::Gnome {
                capture_states.extend(
                    [
                        "gnome-session",
                        "gnome-source-registered",
                        "gnome-source-active",
                    ]
                    .into_iter()
                    .map(|id| report.check(id).map_or(I::Unknown, |check| check.state)),
                );
            } else if report.desktop == wayexpand_backend_ibus::chain::IbusDesktop::KdeWayland {
                // KWin's IBus Wayland helper is an independent link: a live
                // IBus engine does not prove that KWin is delivering field
                // input to it. Surface helper failure/recovery in the route
                // recommendation instead of showing capture as available.
                if let Some(check) = report.check("desktop") {
                    capture_states.push(check.state);
                }
            }
            // These IBus checks describe repairable configuration/runtime
            // steps (register, select, start), not protocol incompatibility.
            // Keep them separate from permission grants: an administrator
            // prompt is not required merely because registration or source
            // selection needs repair.
            // A failed registration or selection is actionable only when
            // this route is supported. Never turn an explicit unsupported
            // assessment into a repair prompt just because IBus itself also
            // reports a missing component or source.
            if !matches!(
                capture_state,
                BackendState::Unavailable | BackendState::NotImplemented
            ) {
                if capture_states.contains(&I::Fail) || capture_states.contains(&I::Warn) {
                    capture_state = BackendState::ActionRequired;
                } else if capture_states
                    .iter()
                    .any(|state| matches!(state, I::Unknown | I::Skip))
                {
                    capture_state = BackendState::Implemented;
                }
            }
            if let Some(consent) = report.check("consent") {
                if !matches!(
                    injection_state,
                    BackendState::Unavailable | BackendState::NotImplemented
                ) {
                    injection_state = match consent.state {
                        I::Pass => injection_state,
                        // Compatibility consent is an explicit configuration
                        // decision, not an OS permission grant. Warn means the
                        // policy/config could not be read, so leave it unknown.
                        I::Fail => BackendState::ActionRequired,
                        I::Warn | I::Unknown | I::Skip => BackendState::Implemented,
                    };
                }
            }
        }
    }
    Some(RouteRecommendation {
        capture: backend_kind(capture_backend),
        injection: backend_kind(injection_backend),
        capture_label: &route.capture,
        injection_label: &route.injection,
        capture_state,
        injection_state,
        focus_tracking: route.focus_tracking,
        sensitive_fields: route.sensitive_fields,
        trigger_precondition: &route.trigger_precondition,
        output_transaction: &route.output_transaction,
        application_result: &route.application_result,
        certified: route.is_certified(),
        recommended: planned.is_some_and(|planned| {
            planned.standing == wayexpand_backend_selection::RouteStanding::Recommended
        }),
        blocked_by_policy: planned.is_some_and(|planned| {
            planned.standing == wayexpand_backend_selection::RouteStanding::BlockedByPolicy
        }),
    })
}

fn assessment_evidence_state(
    evidence: wayexpand_backend_selection::AssessmentEvidence,
) -> BackendState {
    use wayexpand_backend_selection::AssessmentEvidence as E;
    match evidence {
        E::Yes => BackendState::Available,
        E::No => BackendState::Unavailable,
        E::Unknown => BackendState::Implemented,
        E::RequiresAction => BackendState::ActionRequired,
    }
}

fn route_component_state(
    backend: RouteBackend,
    capture: bool,
    capabilities: Option<&Capabilities>,
    assessment: Option<&wayexpand_backend_selection::BackendAssessment>,
    backend_status: &[BackendStatus],
) -> BackendState {
    let probed_state = |kind| {
        backend_status
            .iter()
            .find(|status| status.kind == kind)
            .map(|status| status.state)
    };
    match backend {
        RouteBackend::IBus => assessment.map_or(BackendState::Implemented, |assessment| {
            assessment_evidence_state(assessment.supported)
        }),
        RouteBackend::InputMethodV2 => capabilities.map_or_else(
            || {
                probed_state(wayexpand_core::BackendKind::InputMethodV2)
                    .unwrap_or(BackendState::Implemented)
            },
            |capabilities| {
                if capabilities.has_input_method_v2 {
                    BackendState::Available
                } else {
                    BackendState::Unavailable
                }
            },
        ),
        RouteBackend::Evdev if capture => capabilities.map_or_else(
            || {
                probed_state(wayexpand_core::BackendKind::Evdev)
                    .unwrap_or(BackendState::Implemented)
            },
            |capabilities| {
                if capabilities.has_dev_input {
                    BackendState::Available
                } else {
                    match probed_state(wayexpand_core::BackendKind::Evdev) {
                        Some(BackendState::RequiresPermission) => BackendState::RequiresPermission,
                        Some(BackendState::NotImplemented) => BackendState::NotImplemented,
                        _ => BackendState::Unavailable,
                    }
                }
            },
        ),
        RouteBackend::Evdev => BackendState::NotImplemented,
        RouteBackend::Libei if !capture => {
            if capabilities.is_some_and(|capabilities| capabilities.has_direct_libei_socket) {
                BackendState::Available
            } else if let Some(portal) =
                assessment.and_then(|assessment| assessment.portal.as_ref())
            {
                match portal.authorization {
                    wayexpand_backend_selection::PortalAuthorization::NotRequested => {
                        // A desktop candidate is discovery evidence only. Do
                        // not tell users permission is required until the
                        // portal implementation and interface are observed.
                        // The explicit injection test performs that probe and
                        // asks before inserting its harmless marker.
                        if portal.implementation_discovered
                            == wayexpand_backend_selection::AssessmentEvidence::Yes
                            && portal.interface_available
                                == wayexpand_backend_selection::AssessmentEvidence::Yes
                        {
                            BackendState::RequiresPermission
                        } else {
                            BackendState::Implemented
                        }
                    }
                    wayexpand_backend_selection::PortalAuthorization::Denied => {
                        BackendState::RequiresPermission
                    }
                    wayexpand_backend_selection::PortalAuthorization::Granted => {
                        match portal.injection_negotiated {
                            wayexpand_backend_selection::AssessmentEvidence::Yes => {
                                BackendState::Available
                            }
                            wayexpand_backend_selection::AssessmentEvidence::No => {
                                BackendState::Unavailable
                            }
                            wayexpand_backend_selection::AssessmentEvidence::Unknown
                            | wayexpand_backend_selection::AssessmentEvidence::RequiresAction => {
                                BackendState::Implemented
                            }
                        }
                    }
                    wayexpand_backend_selection::PortalAuthorization::Unknown => {
                        BackendState::Implemented
                    }
                }
            } else if capabilities.is_some() {
                BackendState::Unavailable
            } else {
                probed_state(wayexpand_core::BackendKind::Libei)
                    .unwrap_or(BackendState::Implemented)
            }
        }
        RouteBackend::Libei => BackendState::NotImplemented,
        RouteBackend::WlrootsVirtualKeyboard if !capture => capabilities.map_or_else(
            || {
                probed_state(wayexpand_core::BackendKind::WlrootsVirtualKeyboard)
                    .unwrap_or(BackendState::Implemented)
            },
            |capabilities| {
                if capabilities.has_virtual_keyboard {
                    BackendState::Available
                } else {
                    BackendState::Unavailable
                }
            },
        ),
        RouteBackend::WlrootsVirtualKeyboard => BackendState::NotImplemented,
    }
}
/// Template variables offered as insert buttons, by group. Group names and
/// hover descriptions are translated in `Strings::template_variable_group`
/// and `Strings::template_variable_description`.
const TEMPLATE_VARIABLE_GROUPS: &[(TemplateVariableGroup, &[&str])] = &[
    (
        TemplateVariableGroup::DateTime,
        &[
            "{{local_date}}",
            "{{local_time}}",
            "{{local_datetime}}",
            "{{local_date:%A %-d %B %Y}}",
            "{{local_time:%-I:%M %p}}",
            "{{date+1d}}",
            "{{date}}",
            "{{time}}",
            "{{datetime}}",
            "{{unix_timestamp}}",
        ],
    ),
    (
        TemplateVariableGroup::Random,
        &[
            "{{uuid}}",
            "{{uuid:id}}",
            "{{random:hex:16}}",
            "{{random:password:20}}",
            "{{random:A|B|C}}",
        ],
    ),
    (
        TemplateVariableGroup::Fields,
        &[
            "{{field:name}}",
            "{{field:name|required|min=2|max=80}}",
            "{{choice:name=A|B}}",
            "{{choice:severity=Low|High;default=High}}",
            "{{datefield:date=today}}",
            "{{{{ }}}}",
        ],
    ),
    (
        TemplateVariableGroup::Text,
        &[
            "{{cursor}}",
            "{{newline}}",
            "{{tab}}",
            "{{username}}",
            "{{hostname}}",
        ],
    ),
];

type ReusableSnippetChoices = (u64, String, Vec<(String, String)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateVariableGroup {
    DateTime,
    Random,
    Fields,
    Text,
}

struct GuiApp {
    path: PathBuf,
    config_revision: wayexpand_core::ConfigRevision,
    preview_library_revision: wayexpand_core::ConfigRevision,
    preview_library_snippets: Arc<wayexpand_core::SnippetLibrary>,
    pending_reload_revision: Option<wayexpand_core::ConfigRevision>,
    config_document: toml_edit::DocumentMut,
    config: Config,
    selected: Option<usize>,
    selected_id: Option<String>,
    filter: String,
    search_fields: library::SearchFields,
    search_index: library::SearchIndex,
    library_revision: u64,
    /// The Quick Picker's global shortcut, from the last diagnostics run.
    picker_shortcut: Option<runtime::PickerShortcut>,
    /// What first-run setup established, from `gui-setup.toml`.
    setup_record: Option<setup_record::SetupRecord>,
    /// Whether `setup_record` reached disk; a confirmation that could not
    /// be saved counts for this session only.
    setup_record_saved: bool,
    /// The running IBus engine's instance (`ibus:<pid>:<started>`).
    ibus_instance: Option<String>,
    /// The administrator policy as loaded at startup (`None` when it could
    /// not be read). With the library's own policy it decides what the
    /// Quick Picker may do; the picker reads it fresh on every launch.
    administrator_policy: Option<wayexpand_core::OrganizationPolicy>,
    /// Why the last attempt to set it failed (a conflict, usually).
    picker_shortcut_error: Option<String>,
    /// Snippet indices the desktop test may use, keyed by library revision
    /// and first-run ordering.
    desktop_test_cache: Option<((u64, bool), Vec<usize>)>,
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
    /// The WayExpand IBus engine is running and serving input contexts.
    ibus_active: bool,
    /// The IBus route checks from the last diagnostics refresh.
    ibus_route: Option<wayexpand_backend_ibus::chain::IbusChainReport>,
    route_state: Option<runtime::RouteState>,
    diagnostics_open: bool,
    diagnostics_close_requested: bool,
    test_expansion_dialog_open: bool,
    desktop_test_trigger: String,
    desktop_test_application_class: String,
    desktop_test_target_acknowledged: bool,
    pending_desktop_test: Option<DesktopTestReview>,
    desktop_test_review: Option<DesktopTestReview>,
    desktop_test_review_open: bool,
    backend_test_help_open: bool,
    service_restart_confirm_open: bool,
    service_restart_unit: Option<String>,
    reset_dialog_open: bool,
    reset_confirm_open: bool,
    reset_scope: String,
    config_backup_dialog_open: bool,
    config_backup_path: String,
    restore_preview: Option<serde_json::Value>,
    restore_confirmed: bool,
    installation_repair_preview: Option<serde_json::Value>,
    installation_repair_dialog_open: bool,
    cli_task_kind: Option<String>,
    last_cli_task_output: Option<String>,
    setup_failure_report: Option<String>,
    setup_failure_report_open: bool,
    support_export_open: bool,
    support_preview_loading: bool,
    support_preview: Option<String>,
    support_preview_error: Option<String>,
    support_output_path: String,
    evdev_setup_open: bool,
    evdev_setup_acknowledged: bool,
    setup_consent_open: bool,
    setup_consent_acknowledged: bool,
    setup_ibus_preflight_report: Option<serde_json::Value>,
    setup_system_integration_required: bool,
    setup_system_integration_acknowledged: bool,
    guided_setup_waiting_for_route: bool,
    desktop_test_verified: bool,
    desktop_test_verified_application: Option<String>,
    daemon_status: String,
    daemon_capabilities: Option<runtime::DaemonCapabilities>,
    installation_audit: runtime::InstallationAudit,
    fleet_status: String,
    backend_status: Vec<BackendStatus>,
    recommended_route: Option<RecommendedRoute>,
    route_plan: Option<wayexpand_backend_selection::RoutePlan>,
    selection_capabilities: Option<Capabilities>,
    protocol_probes: Vec<(String, String)>,
    compositor_info: String,
    permission_health: String,
    diagnostics_sender: Option<SyncSender<runtime::Request>>,
    runtime_sender: Option<SyncSender<runtime::Request>>,
    runtime_receiver: Option<mpsc::Receiver<runtime::Completion>>,
    diagnostics_running: bool,
    diagnostics_checked_at: Option<Instant>,
    pending_control: usize,
    next_status_poll: Instant,
    /// Position in the visible snippet list to scroll into view after
    /// keyboard navigation moved the selection.
    list_scroll_target: Option<usize>,
    pending_action: Option<PendingAction>,
    profiles: Vec<ProfileEntry>,
    active_profile: Option<String>,
    profile_name_draft: String,
    profile_app_id_draft: String,
    profile_app_target: Option<String>,
    profile_app_assignments: Vec<AppProfileAssignment>,
    profile_loaded: bool,
    profile_error: Option<String>,
    profile_receiver: Option<mpsc::Receiver<(ProfileOperation, Result<String, String>)>>,
    pack_dialog_open: bool,
    pack_candidate_path: String,
    pack_catalog_path: String,
    pack_catalog_query: String,
    pack_catalog_results: Vec<serde_json::Value>,
    pack_catalog_invalid_count: usize,
    pack_catalog_signers_file: Option<String>,
    pack_action_consent: bool,
    pack_rollback_action_consent: bool,
    pack_rollback_confirmation: Option<String>,
    pack_review: Option<PackReview>,
    pack_operation_receiver:
        Option<mpsc::Receiver<(PackOperation, Result<PackWorkerResult, String>)>>,
    pack_error: Option<String>,
    import_open: bool,
    import_path: String,
    import_preview: Option<(Config, wayexpand_core::EspansoImportReport)>,
    settings_open: bool,
    settings_tab: SettingsTab,
    settings_buffer: String,
    settings_undo_chord: String,
    settings_compatibility_mode: wayexpand_core::CompatibilityMode,
    settings_keystroke_limit: String,
    settings_terminal_paste: wayexpand_core::TerminalPaste,
    settings_font_scale: FontScale,
    /// Width the one-row toolbar needed when it was last laid out. The
    /// status chips change with the route, so a fixed breakpoint cannot
    /// tell whether they fit beside the brand.
    toolbar_row_width: f32,
    settings_error: Option<String>,
    dark_mode: bool,
    onboarding_visible: bool,
    onboarding_completed: bool,
    onboarding_test_snippet_created_for: Option<String>,
    /// The post-setup desktop test must be completed before onboarding can
    /// persist a successful first-run state.
    desktop_test_dialog_required: bool,
    gnome_setup_resume_pending: bool,
    onboarding_show_troubleshooting: bool,
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
    preview_cache: Option<(u64, String, Result<String, String>)>,
    /// Parsed form fields and their source template, reused by the preview and
    /// command contract panels until the replacement text changes.
    form_fields_cache: Option<(
        String,
        Vec<wayexpand_core::FormField>,
        Result<wayexpand_core::CompiledTemplate, String>,
    )>,
    reusable_snippet_filter: String,
    reusable_snippet_choices: Option<ReusableSnippetChoices>,
    form_field_name: String,
    form_field_default: String,
    form_field_options: String,
    form_field_required: bool,
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
    /// Debug builds only: capture the window and exit (see `screenshot`).
    screenshot: Option<screenshot::ScreenshotCapture>,
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
    /// Form values used only by the editor's local template playground.
    form_preview_values: std::collections::HashMap<String, String>,
    try_live_open: bool,
    /// The user's basic/advanced editor choice this session; `None` follows
    /// the snippet (advanced options open when it uses them).
    editor_advanced: Option<bool>,
    /// The interface's default level of detail. Everyday is intentionally
    /// the first-run default; advanced configuration remains available from
    /// the visible experience switcher.
    experience: GuiExperience,
    /// Whether the diagnostics overview shows its technical evidence grid.
    diagnostics_details_open: bool,
    /// A running one-click "Turn on WayExpand" (`wayexpand setup --yes`).
    setup_task: Option<SetupTask>,
    sync_task: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    pending_save: Option<PendingSave>,
    queued_save: Option<PendingSave>,
    /// A change whose save hit a revision conflict, kept until the disk
    /// version is reloaded and it is merged or reported as not saved.
    conflicted_save: Option<PendingSave>,
    /// A change that could not be saved (the write failed, the lock stayed
    /// busy, or the reload after a conflict failed). Kept until the user
    /// retries it successfully or discards it, so a transient failure never
    /// silently drops an edit.
    failed_save: Option<PendingSave>,
    /// Set once closing was held back for `failed_save`; a second close
    /// goes through.
    close_warned_failed_save: bool,
    /// When to retry a save that found the configuration lock busy.
    save_retry_at: Option<Instant>,
    next_save_id: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ProfileEntry {
    pub(crate) name: String,
    pub(crate) valid: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct AppProfileAssignment {
    pub(crate) app_id: String,
    pub(crate) profile: String,
}

#[derive(Clone, Debug)]
pub(crate) enum ProfileOperation {
    List,
    Switch(String),
    Create(String),
    AssignApp { app_id: String, profile: String },
    UnassignApp(String),
}

#[derive(Clone, Debug)]
pub(crate) struct PackReview {
    pub(crate) details: serde_json::Value,
    pub(crate) installed: bool,
    pub(crate) diff: Option<String>,
    pub(crate) diff_error: Option<String>,
    pub(crate) rollback_details: Option<serde_json::Value>,
    pub(crate) rollback_diff: Option<String>,
    pub(crate) rollback_diff_error: Option<String>,
    pub(crate) install_output: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum PackOperation {
    Review,
    SearchCatalog {
        catalog: std::path::PathBuf,
        query: String,
    },
    Install {
        replace: bool,
        allow_actions: bool,
        fingerprint: String,
    },
    Rollback {
        id: String,
        allow_actions: bool,
        fingerprint: String,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum PackWorkerResult {
    Catalog {
        packs: Vec<serde_json::Value>,
        invalid_candidates: usize,
        signers_file: String,
    },
    Review {
        details: serde_json::Value,
        installed: bool,
        diff: Option<String>,
        diff_error: Option<String>,
        rollback_details: Option<serde_json::Value>,
        rollback_diff: Option<String>,
        rollback_diff_error: Option<String>,
    },
    Installed(String),
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
    success_message: String,
    is_setup: bool,
}

#[derive(Clone, Debug)]
struct DesktopTestReview {
    trigger: String,
    replacement: String,
    application_class: String,
    route: Option<String>,
    route_instance: Option<String>,
    desktop: String,
    session: Option<String>,
    build: String,
    config_revision: String,
    capture_activity_observed: bool,
    output_activity_observed: bool,
}

impl Drop for SetupTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

struct PendingSave {
    request_id: u64,
    candidate: Config,
    /// The in-memory configuration the candidate was derived from. The
    /// difference between the two is the change; it is replayed onto newer
    /// configurations rather than writing a stale whole-file snapshot.
    base: Config,
    preview_revision: u64,
    intent: SaveIntent,
    /// Busy-lock retries so far.
    attempts: u8,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GuiExperience {
    #[default]
    Everyday,
    Advanced,
    DeveloperAdmin,
}

impl GuiExperience {
    fn includes_advanced(self) -> bool {
        !matches!(self, Self::Everyday)
    }

    fn includes_developer_admin(self) -> bool {
        matches!(self, Self::DeveloperAdmin)
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
        let gnome_setup_resume_pending = gnome_setup_resume_pending_for_current_session(&path);
        let preview_library_snippets = config.includable_snippets();
        let preview_library_revision = config_revision.clone();
        let search_index = library::SearchIndex::new(&config);
        let selected = (!config.expansion.is_empty()).then_some(0);
        let selected_id = selected.map(|index| config.expansion[index].id.clone());
        let draft = selected.map(|index| Draft::from_expansion(&config.expansion[index]));
        let preview_input = selected
            .map(|index| config.expansion[index].trigger.clone())
            .unwrap_or_default();
        let settings_buffer = config.settings.max_buffer_chars.to_string();
        let settings_undo_chord = config.settings.undo_chord.clone().unwrap_or_default();
        let settings_compatibility_mode = config.settings.compatibility_mode;
        let settings_keystroke_limit = config.settings.keystroke_max_chars.to_string();
        let settings_terminal_paste = config.settings.terminal_paste;
        let prefs = load_gui_prefs();
        let onboarding_completed = prefs.onboarding_completed;
        let onboarding_test_snippet_created_for = prefs.onboarding_test_snippet_created_for;
        // Show the guide until setup is completed or explicitly skipped. It
        // used to depend on the library being empty, so a migrated library
        // (Espanso import, another machine) never saw it.
        let onboarding_visible = gnome_setup_resume_pending || !onboarding_completed;
        let settings_font_scale = prefs.font_scale;
        let strings = Strings::new(prefs.language);
        Ok(Self {
            path: path.clone(),
            config_revision,
            preview_library_revision,
            preview_library_snippets,
            pending_reload_revision: None,
            config_document,
            config,
            selected,
            selected_id,
            filter: String::new(),
            search_fields: library::SearchFields::default(),
            search_index,
            library_revision: 0,
            picker_shortcut: None,
            setup_record: setup_record::load(),
            setup_record_saved: true,
            ibus_instance: None,
            administrator_policy: wayexpand_core::load_organization_policy().ok(),
            picker_shortcut_error: None,
            desktop_test_cache: None,
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
            ibus_active: runtime::ibus_engine_active(),
            ibus_route: None,
            route_state: None,
            diagnostics_open: false,
            diagnostics_close_requested: false,
            test_expansion_dialog_open: false,
            desktop_test_trigger: String::new(),
            desktop_test_application_class: "Text editor".into(),
            desktop_test_target_acknowledged: false,
            pending_desktop_test: None,
            desktop_test_review: None,
            desktop_test_review_open: false,
            backend_test_help_open: false,
            service_restart_confirm_open: false,
            service_restart_unit: None,
            reset_dialog_open: false,
            reset_confirm_open: false,
            reset_scope: "integration".into(),
            config_backup_dialog_open: false,
            config_backup_path: format!("{}.bak", path.display()),
            restore_preview: None,
            restore_confirmed: false,
            installation_repair_preview: None,
            installation_repair_dialog_open: false,
            cli_task_kind: None,
            last_cli_task_output: None,
            setup_failure_report: None,
            setup_failure_report_open: false,
            support_export_open: false,
            support_preview_loading: false,
            support_preview: None,
            support_preview_error: None,
            support_output_path: std::env::var_os("HOME")
                .map(|home| {
                    PathBuf::from(home)
                        .join("wayexpand-support.json")
                        .display()
                        .to_string()
                })
                .unwrap_or_else(|| "wayexpand-support.json".into()),
            evdev_setup_open: false,
            evdev_setup_acknowledged: false,
            setup_consent_open: false,
            setup_consent_acknowledged: false,
            setup_ibus_preflight_report: None,
            setup_system_integration_required: false,
            setup_system_integration_acknowledged: false,
            guided_setup_waiting_for_route: false,
            desktop_test_verified: false,
            desktop_test_verified_application: None,
            daemon_status: strings.not_checked().into(),
            daemon_capabilities: None,
            installation_audit: runtime::InstallationAudit::default(),
            fleet_status: strings.not_checked().into(),
            backend_status: Vec::new(),
            recommended_route: None,
            route_plan: None,
            selection_capabilities: None,
            protocol_probes: Vec::new(),
            compositor_info: strings.not_checked().into(),
            permission_health: strings.not_checked().into(),
            diagnostics_sender: None,
            runtime_sender: None,
            runtime_receiver: None,
            diagnostics_running: false,
            diagnostics_checked_at: None,
            pending_control: 0,
            next_status_poll: Instant::now() + Duration::from_secs(2),
            list_scroll_target: None,
            pending_action: None,
            profiles: Vec::new(),
            active_profile: None,
            profile_name_draft: String::new(),
            profile_app_id_draft: String::new(),
            profile_app_target: None,
            profile_app_assignments: Vec::new(),
            profile_loaded: false,
            profile_error: None,
            profile_receiver: None,
            pack_dialog_open: false,
            pack_candidate_path: String::new(),
            pack_catalog_path: String::new(),
            pack_catalog_query: String::new(),
            pack_catalog_results: Vec::new(),
            pack_catalog_invalid_count: 0,
            pack_catalog_signers_file: None,
            pack_action_consent: false,
            pack_rollback_action_consent: false,
            pack_rollback_confirmation: None,
            pack_review: None,
            pack_operation_receiver: None,
            pack_error: None,
            import_open: false,
            import_path: String::new(),
            import_preview: None,
            settings_open: false,
            settings_tab: SettingsTab::Appearance,
            settings_buffer,
            settings_undo_chord,
            settings_compatibility_mode,
            settings_keystroke_limit,
            settings_terminal_paste,
            settings_font_scale,
            toolbar_row_width: 0.0,
            settings_error: None,
            dark_mode: prefs.dark_mode.unwrap_or(true),
            onboarding_visible,
            onboarding_completed,
            onboarding_test_snippet_created_for,
            desktop_test_dialog_required: false,
            gnome_setup_resume_pending,
            onboarding_show_troubleshooting: false,
            language: prefs.language,
            strings,
            colorpack: prefs.colorpack,
            command_preview_result: None,
            command_preview_key: None,
            command_preview_receiver: None,
            command_preview_cancel: None,
            theme_refresh_pending: false,
            preview_cache: None,
            form_fields_cache: None,
            reusable_snippet_filter: String::new(),
            reusable_snippet_choices: None,
            form_field_name: "name".into(),
            form_field_default: String::new(),
            form_field_options: "Low,Medium,High".into(),
            form_field_required: true,
            preview_revision: 0,
            app_detection: None,
            screenshot: screenshot::ScreenshotCapture::from_env(),
            close_after_confirm: false,
            window_title: String::new(),
            playground: playground::Playground::default(),
            form_preview_values: std::collections::HashMap::new(),
            // Collapsed by default so the snippet form gets the window's
            // height; its header stays visible as a one-line bar.
            // The in-process matcher check is useful even when a desktop
            // route is not set up yet, so make it visible in the everyday
            // workspace instead of hiding it behind an extra click.
            try_live_open: true,
            editor_advanced: None,
            experience: GuiExperience::Everyday,
            diagnostics_details_open: false,
            setup_task: None,
            sync_task: None,
            pending_save: None,
            queued_save: None,
            conflicted_save: None,
            failed_save: None,
            close_warned_failed_save: false,
            save_retry_at: None,
            next_save_id: 1,
        })
    }
}

impl eframe::App for GuiApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        if let Some(capture) = self.screenshot.as_mut() {
            let view = capture.take_view();
            capture.tick(ctx);
            match view.as_deref() {
                Some("light") => self.set_dark_mode(ctx, false),
                Some("settings") => self.open_settings(),
                Some("diagnostics") => {
                    self.diagnostics_open = true;
                    self.refresh_diagnostics(true);
                }
                Some("new") => self.request_action(PendingAction::New),
                Some("import") => self.import_open = true,
                Some("search") => self.filter = "s".into(),
                Some("preview") => {
                    self.try_live_open = true;
                    for character in
                        "Hi Jordan, ;tyI've looked into this and the fix ships on Friday.".chars()
                    {
                        let mut text = self.playground.text.clone();
                        text.push(character);
                        self.playground.text = text.clone();
                        if let Some(rewrite) = self.playground.edit(
                            &self.config,
                            &self.preview_app,
                            "unknown",
                            &text,
                            true,
                        ) {
                            self.playground.text = rewrite.text;
                        }
                    }
                }
                _ => {}
            }
        }
        self.poll_runtime(ctx);
        self.poll_command_preview(ctx);
        self.poll_setup(ctx);
        self.poll_sync(ctx);
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
        if close_requested
            && !self.draft_is_dirty()
            && self.failed_save.is_some()
            && !self.close_warned_failed_save
        {
            // A change that failed to save is not in the editor any more
            // (settings, imports, deletions), so warn once before it goes.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_warned_failed_save = true;
            self.status = Status::warning(self.strings.status_unsaved_change_blocks_close());
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
        self.render_next_step_banner(root, &palette);
        self.render_diagnostics(ctx, &palette);
        self.render_import_dialog(ctx, &palette);
        self.render_pack_dialog(ctx, &palette);
        self.render_settings_dialog(ctx, &palette);
        // Panel order decides how the remaining space is carved up: the
        // status line spans the full width, the sidebar then claims the left
        // edge, and the editor's action bar sits above the status line but
        // only across the editor itself.
        self.render_status_bar(root, &palette);
        self.render_snippet_list(root, &palette);
        self.render_editor_actions(root, &palette);
        // First run already has a dedicated, guided matcher/desktop test.
        // Avoid showing a second preview panel until the user reaches the
        // everyday snippet workspace.
        if !self.onboarding_visible || self.try_live_open {
            self.render_try_live_panel(root, &palette);
        }
        self.render_editor(root, &palette);
        self.render_pending_action(ctx, &palette);
        self.render_evdev_setup(ctx, &palette);
        self.render_setup_consent(ctx);
        self.render_installation_repair_dialog(ctx, &palette);
        if self.any_dialog_open() {
            // Dim the window behind an open dialog so it reads as one.
            // Dialog windows are in the Foreground order, so a Middle layer
            // covers every panel and stays below them.
            let backdrop =
                egui::LayerId::new(egui::Order::Middle, egui::Id::new("dialog_backdrop"));
            ctx.memory_mut(|memory| memory.areas_mut().move_to_top(backdrop));
            ctx.layer_painter(backdrop).rect_filled(
                ctx.content_rect(),
                0.0,
                egui::Color32::from_black_alpha(if self.dark_mode { 110 } else { 70 }),
            );
        }
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

/// Load the bundled application icon for every native window, including the
/// picker and generated form dialogs. Keeping this in one place prevents
/// secondary GUI surfaces from falling back to the toolkit's generic icon.
pub(crate) fn app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/icon/hicolor/256x256/apps/wayexpand.png"
    ))
    .expect("bundled app icon is a valid PNG")
}

/// Resolve the optional CONFIG argument. An unrecognized option is an error
/// rather than a path: treating `--version` as a config file would create it
/// in the working directory on the first save.
fn run_recovery(path: PathBuf, error: String) -> Result<()> {
    let prefs = load_gui_prefs();
    let strings = Strings::new(prefs.language);
    let recovery = recovery::RecoveryApp::new(path, error, strings);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 560.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "WayExpand",
        options,
        Box::new(move |creation_context| {
            fonts::install(&creation_context.egui_ctx);
            theme::install_pack(
                &creation_context.egui_ctx,
                prefs.colorpack,
                prefs.font_scale,
            );
            Ok(Box::new(recovery))
        }),
    )
    .map_err(|error| anyhow::anyhow!("GUI failed: {error}"))
}

fn config_path_argument(argument: Option<String>) -> Result<PathBuf> {
    match argument {
        Some(option) if option.starts_with('-') => {
            anyhow::bail!("unknown option {option}; see wayexpand-gui --help")
        }
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(default_config_path()),
    }
}

fn main() -> Result<()> {
    if matches!(env::args().nth(1).as_deref(), Some("--help" | "-h")) {
        println!(
            "Usage: wayexpand-gui [CONFIG]\n       wayexpand-gui --picker [--copy-only] [CONFIG]\n       wayexpand-gui --form SPEC\n\nNative Wayland settings editor for WayExpand.\n\n--picker     Quick-insert window for searching saved snippets.\n--copy-only  Copy the selected snippet for manual paste; do not request insertion.\n--form       Snippet form window; started by the daemon for snippets with fields."
        );
        return Ok(());
    }
    if matches!(env::args().nth(1).as_deref(), Some("--version" | "-V")) {
        println!("wayexpand-gui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("--form") {
        let spec = env::args()
            .nth(2)
            .ok_or_else(|| anyhow::anyhow!("--form needs a form specification"))?;
        return form::run(&spec);
    }
    if env::args().nth(1).as_deref() == Some("--picker") {
        let mut copy_only = false;
        let mut config = None;
        for argument in env::args().skip(2) {
            if argument == "--copy-only" {
                copy_only = true;
            } else if config.replace(argument).is_some() {
                anyhow::bail!("--picker accepts at most one CONFIG path");
            }
        }
        let path = config_path_argument(config)?;
        return picker::run(path, copy_only);
    }
    let path = config_path_argument(env::args().nth(1))?;
    // A library that fails to load opens a recovery screen instead of
    // stopping the GUI before its window appears.
    let mut app = match GuiApp::load(path.clone()) {
        Ok(app) => app,
        Err(error) => return run_recovery(path, error.to_string()),
    };
    app.start_runtime()?;
    app.ensure_onboarding_test_snippet();
    let saved_dark_mode = load_gui_prefs().dark_mode;
    let colorpack = app.colorpack;
    let icon = app_icon();
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
