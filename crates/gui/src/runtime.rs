//! Bounded background runtime for daemon control, desktop probes, and config I/O.

use crate::{diagnostics, status::Status};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};
use toml_edit::DocumentMut;
use wayexpand_backend_ibus::engine_available as ibus_engine_available;
use wayexpand_backend_selection::{
    plan_routes, probe_capabilities, recommended_route, Capabilities, RecommendedRoute, RoutePlan,
};
use wayexpand_core::{
    discover_backends, CompatibilityMode, Config, DaemonStatus, FleetConfig,
    InsertionRouteCapabilities, InsertionStrategy, CONTROL_STATUS_SCHEMA,
};

const REQUEST_CAPACITY: usize = 8;
const RESULT_CAPACITY: usize = 16;

pub(crate) enum Request {
    Diagnostics {
        config_path: PathBuf,
        announce: bool,
    },
    SupportPreview,
    Control {
        daemon_operation: wayexpand_core::DaemonOperation,
        operation: Operation,
    },
    ReloadConfig {
        path: PathBuf,
    },
    SaveConfig {
        request_id: u64,
        path: PathBuf,
        // Boxed: both are large and would otherwise size every request.
        candidate: Box<Config>,
        base_document: Box<DocumentMut>,
        expected_revision: wayexpand_core::ConfigRevision,
    },
}

pub(crate) enum Operation {
    Reload(Status),
    Pause { paused: bool },
    Status,
}

pub(crate) enum Completion {
    Diagnostics(Box<DiagnosticsSnapshot>),
    SupportPreview(Result<String, String>),
    Control {
        operation: Operation,
        result: anyhow::Result<String>,
    },
    ConfigReloaded(Box<Result<ReloadSnapshot, String>>),
    ConfigSaved {
        request_id: u64,
        result: Result<(wayexpand_core::ConfigRevision, DocumentMut), SaveFailure>,
    },
}

/// Why a background save did not land, classified so the editor can react
/// without parsing message text.
#[derive(Debug)]
pub(crate) enum SaveFailure {
    /// The file changed on disk since it was loaded.
    Conflict,
    /// Another writer holds the configuration lock.
    Busy,
    /// Anything else, as a summary that never echoes snippet content.
    Failed(String),
}

pub(crate) struct ReloadSnapshot {
    pub config: Config,
    pub revision: wayexpand_core::ConfigRevision,
    pub document: DocumentMut,
    pub search_index: crate::library::SearchIndex,
}

pub(crate) struct DiagnosticsSnapshot {
    pub backend_status: Vec<wayexpand_core::BackendStatus>,
    pub recommended_route: Option<RecommendedRoute>,
    pub route_plan: RoutePlan,
    pub selection_capabilities: Capabilities,
    pub fleet_status: String,
    pub protocol_probes: Vec<(String, String)>,
    pub compositor_info: String,
    pub permission_health: String,
    pub daemon_status: String,
    pub daemon_capabilities: Option<DaemonCapabilities>,
    pub daemon_reachable: Option<bool>,
    pub route_state: Option<RouteState>,
    pub paused: Option<bool>,
    pub ibus_route: wayexpand_backend_ibus::chain::IbusChainReport,
    pub announce: bool,
}

/// Whether the WayExpand IBus engine is live and serving input contexts,
/// from its published activity file (a small, private file read).
pub(crate) fn ibus_engine_active() -> bool {
    wayexpand_backend_ibus::telemetry::read_status()
        .is_some_and(|reading| reading.live && reading.status.engines > 0)
}

pub(crate) type RouteState = wayexpand_core::DaemonRouteState;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DaemonCapabilities {
    pub capture_source: Option<String>,
    pub raw_capture_fallback_enabled: Option<bool>,
    pub raw_capture_app_count: Option<usize>,
    pub capture_app_scope_state: Option<String>,
    pub window_process_identity_state: Option<String>,
    pub injection_mode: Option<&'static str>,
    pub injection_max_text_chars: Option<usize>,
    pub injection_throughput_chars_per_sec: Option<u32>,
    pub injection_latency_p50_us: Option<u64>,
    pub injection_latency_mean_us: Option<u64>,
    pub injection_latency_p95_us: Option<u64>,
    pub injection_latency_p99_us: Option<u64>,
    pub output_interrupt_sample_count: Option<u64>,
    pub output_interrupt_dropped_total: Option<u64>,
    pub output_interrupt_p50_us: Option<u64>,
    pub output_interrupt_p95_us: Option<u64>,
    pub output_interrupt_p99_us: Option<u64>,
    pub matcher_latency_p50_us: Option<u64>,
    pub matcher_latency_p95_us: Option<u64>,
    pub matcher_latency_p99_us: Option<u64>,
    pub capture_sensitive_focus: Option<bool>,
    pub capture_exclusive: Option<bool>,
    pub capture_reliable_key_state: Option<bool>,
    pub capture_key_passthrough: Option<bool>,
    pub capture_composition_aware: Option<bool>,
    pub capture_local_compose_aware: Option<bool>,
    pub capture_layout_aware: Option<bool>,
    pub window_tracker_connected: Option<bool>,
    pub window_tracker_state: Option<String>,
    pub window_tracker_reconnect_attempts_total: Option<u64>,
    pub window_tracker_disconnects_total: Option<u64>,
    pub window_tracker_last_failure_reason: Option<String>,
    pub window_tracker_last_connected_unix: Option<u64>,
    pub window_tracker_reconnect_backoff_ms: Option<u64>,
    pub window_identity_exact: Option<bool>,
    pub inject_atomic_replace: Option<bool>,
    pub inject_full_unicode: Option<bool>,
    pub inject_cursor_reposition: Option<bool>,
    pub inject_key_passthrough: Option<bool>,
    pub replacement_assurance: Option<ReplacementAssurance>,
    pub output_refused_total: Option<u64>,
    pub output_last_refusal: Option<OutputRefusal>,
    pub output_uncertain_total: Option<u64>,
    pub output_partial_total: Option<u64>,
    pub partial_write_total: Option<u64>,
    pub compatibility_mode: Option<CompatibilityMode>,
    pub evdev_quarantined_events_total: Option<u64>,
    pub usage_flush_max_duration_us: Option<u64>,
    pub usage_flush_failures_total: Option<u64>,
    pub usage_worker_failures_total: Option<u64>,
    pub usage_queue_rejected_total: Option<u64>,
    pub usage_events_recorded_total: Option<u64>,
    pub usage_events_persisted_total: Option<u64>,
    pub usage_events_dropped_total: Option<u64>,
    pub recent_expansion_outcomes: Vec<ExpansionTrace>,
    pub recent_expansion_outcomes_reported: bool,
}

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct ExpansionTrace {
    pub at_unix: u64,
    pub duration_us: Option<u64>,
    pub interrupt_to_backend_return_us: Option<u64>,
    pub backend: String,
    pub expected_insert_units: Option<usize>,
    pub trigger_recognized: String,
    pub focus_admission: String,
    pub backend_selected: String,
    pub erase_acknowledged: String,
    pub insertion_attempted: bool,
    pub completion: String,
    pub application_result: String,
    pub recovery: String,
    pub progress_tracked: bool,
    pub erased_units: usize,
    pub inserted_units: usize,
}

/// What the daemon's output route promises before it erases a trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplacementAssurance {
    /// The route verifies the target text before erasing.
    Verified,
    /// A tracker confirms the window, but the erased text is not verified.
    FocusTrackedBestEffort,
    /// Neither target text nor focused window is verified.
    Unverified,
    /// A keystroke route without consent: typed expansions are refused.
    Blocked,
    Unavailable,
}

/// Why the daemon refused an output before typing anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputRefusal {
    CompatibilityModeOff,
    FocusChanged,
    FocusUnconfirmed,
}

impl DaemonCapabilities {
    pub fn insertion_plan(
        &self,
        output_characters: usize,
        paced_events: usize,
    ) -> InsertionStrategy {
        let route = self.injection_mode.map(|mode| InsertionRouteCapabilities {
            atomic_verified_replace: self.inject_atomic_replace == Some(true)
                && self.replacement_assurance == Some(ReplacementAssurance::Verified),
            native_text_commit: matches!(mode, "ei_text" | "input-method-v2 text")
                || mode.contains("direct commit"),
            paced_throughput_chars_per_sec: self.injection_throughput_chars_per_sec,
            max_text_chars: self.injection_max_text_chars.unwrap_or(0),
        });
        wayexpand_core::plan_replacement(
            output_characters,
            paced_events,
            route,
            self.compatibility_mode.unwrap_or(CompatibilityMode::Off),
        )
    }

    pub fn provides_full_protection(&self) -> bool {
        self.capture_sensitive_focus == Some(true)
            && self.capture_composition_aware == Some(true)
            && self.capture_key_passthrough == Some(true)
            && self.capture_layout_aware == Some(true)
            && self.window_identity_exact == Some(true)
            && self.inject_atomic_replace == Some(true)
            && self.inject_full_unicode == Some(true)
            && self.inject_key_passthrough == Some(true)
    }

    pub fn parse(response: &str) -> Option<Self> {
        let status = DaemonStatus::parse(response);
        if !status.field_is_unique("status_schema")
            || status.status_schema() != Some(CONTROL_STATUS_SCHEMA)
        {
            return None;
        }

        let mut capabilities = Self::default();
        let mut found = false;
        if let Some(value) = status.field("inject_insertion_mode") {
            capabilities.injection_mode = match value {
                "ei_text" | "ei_text (UTF-8 insertion)" => Some("ei_text"),
                "libei keysym fallback" | "ei_keyboard keysym fallback (12ms key pacing)" => {
                    Some("libei keysym fallback")
                }
                "wlroots virtual-keyboard key synthesis" => {
                    Some("wlroots virtual-keyboard key synthesis")
                }
                "input-method-v2 text" => Some("input-method-v2 text"),
                "Fcitx5 direct commit (libei fallback available)" => {
                    Some("Fcitx5 direct commit (libei fallback available)")
                }
                _ => None,
            };
            found = true;
        }
        if let Some(value) = status.field("compatibility_mode") {
            capabilities.compatibility_mode = match value {
                "off" => Some(CompatibilityMode::Off),
                "tracked-focus" => Some(CompatibilityMode::TrackedFocus),
                "untracked-focus" => Some(CompatibilityMode::UntrackedFocus),
                _ => None,
            };
        }
        if let Some(value) = status.u64_field("inject_max_text_chars") {
            capabilities.injection_max_text_chars = usize::try_from(value).ok();
            found |= capabilities.injection_max_text_chars.is_some();
        }
        if let Some(value) = status.u64_field("inject_expected_throughput_chars_per_sec") {
            capabilities.injection_throughput_chars_per_sec = u32::try_from(value).ok();
            found |= capabilities.injection_throughput_chars_per_sec.is_some();
        }
        for (key, slot) in [
            (
                "injection_latency_mean_us",
                &mut capabilities.injection_latency_mean_us,
            ),
            (
                "injection_latency_p50_us",
                &mut capabilities.injection_latency_p50_us,
            ),
            (
                "injection_latency_p95_us",
                &mut capabilities.injection_latency_p95_us,
            ),
            (
                "injection_latency_p99_us",
                &mut capabilities.injection_latency_p99_us,
            ),
            (
                "matcher_latency_p50_us",
                &mut capabilities.matcher_latency_p50_us,
            ),
            (
                "matcher_latency_p95_us",
                &mut capabilities.matcher_latency_p95_us,
            ),
            (
                "matcher_latency_p99_us",
                &mut capabilities.matcher_latency_p99_us,
            ),
        ] {
            if let Some(value) = status.u64_field(key) {
                *slot = Some(value);
                found = true;
            }
        }
        for (key, slot) in [
            (
                "output_interrupt_to_backend_return_sample_count",
                &mut capabilities.output_interrupt_sample_count,
            ),
            (
                "output_interrupt_to_backend_return_dropped_total",
                &mut capabilities.output_interrupt_dropped_total,
            ),
            (
                "output_interrupt_to_backend_return_p50_us",
                &mut capabilities.output_interrupt_p50_us,
            ),
            (
                "output_interrupt_to_backend_return_p95_us",
                &mut capabilities.output_interrupt_p95_us,
            ),
            (
                "output_interrupt_to_backend_return_p99_us",
                &mut capabilities.output_interrupt_p99_us,
            ),
        ] {
            if let Some(value) = status.u64_field(key) {
                *slot = Some(value);
                found = true;
            }
        }
        capabilities.replacement_assurance =
            status
                .field("replacement_assurance")
                .and_then(|value| match value {
                    "verified" => Some(ReplacementAssurance::Verified),
                    "focus-tracked-best-effort" => {
                        Some(ReplacementAssurance::FocusTrackedBestEffort)
                    }
                    "unverified" => Some(ReplacementAssurance::Unverified),
                    "blocked" => Some(ReplacementAssurance::Blocked),
                    "unavailable" => Some(ReplacementAssurance::Unavailable),
                    _ => None,
                });
        capabilities.capture_source = status.field("source").map(str::to_owned);
        found |= capabilities.capture_source.is_some();
        if let Some(value) = status.bool_field("raw_capture_fallback_enabled") {
            capabilities.raw_capture_fallback_enabled = Some(value);
            found = true;
        }
        if let Some(value) = status.u64_field("raw_capture_app_count") {
            capabilities.raw_capture_app_count = usize::try_from(value).ok();
            found |= capabilities.raw_capture_app_count.is_some();
        }
        capabilities.capture_app_scope_state = status
            .field("capture_app_scope_state")
            .filter(|value| {
                matches!(
                    *value,
                    "unrestricted" | "unknown" | "allowed" | "outside_allowlist"
                )
            })
            .map(str::to_owned);
        found |= capabilities.capture_app_scope_state.is_some();
        capabilities.window_process_identity_state = status
            .field("window_process_identity_state")
            .filter(|value| {
                matches!(
                    *value,
                    "not-applicable"
                        | "not-requested"
                        | "compositor-application-id"
                        | "process-executable"
                        | "process-id-unavailable"
                        | "process-metadata-unavailable"
                        | "different-user"
                        | "executable-permission-denied"
                        | "executable-invalid"
                        | "process-changed-during-lookup"
                        | "process-name-unavailable"
                        | "executable-link-unavailable"
                        | "compositor-process-id"
                )
            })
            .map(str::to_owned);
        found |= capabilities.window_process_identity_state.is_some();
        capabilities.window_tracker_state = status
            .field("window_tracker_state")
            .filter(|value| {
                matches!(
                    *value,
                    "unavailable" | "starting" | "connected" | "reconnecting" | "stopped"
                )
            })
            .map(str::to_owned);
        capabilities.window_tracker_last_failure_reason = status
            .field("window_tracker_last_failure_reason")
            .filter(|value| {
                matches!(
                    *value,
                    "none"
                        | "probe_unavailable"
                        | "connect_failed"
                        | "tracker_disconnected"
                        | "configuration_changed"
                        | "compositor_unsupported"
                        | "supervisor_start_failed"
                )
            })
            .map(str::to_owned);
        for (key, slot) in [
            (
                "window_tracker_reconnect_attempts_total",
                &mut capabilities.window_tracker_reconnect_attempts_total,
            ),
            (
                "window_tracker_disconnects_total",
                &mut capabilities.window_tracker_disconnects_total,
            ),
            (
                "window_tracker_last_connected_unix",
                &mut capabilities.window_tracker_last_connected_unix,
            ),
            (
                "window_tracker_reconnect_backoff_ms",
                &mut capabilities.window_tracker_reconnect_backoff_ms,
            ),
        ] {
            if let Some(value) = status.u64_field(key) {
                *slot = Some(value);
                found = true;
            }
        }
        found |= capabilities.window_tracker_state.is_some();
        found |= capabilities.window_tracker_last_failure_reason.is_some();
        capabilities.output_last_refusal =
            status
                .field("output_last_refusal")
                .and_then(|value| match value {
                    "compatibility-mode-off" => Some(OutputRefusal::CompatibilityModeOff),
                    "focus-changed" => Some(OutputRefusal::FocusChanged),
                    "focus-unconfirmed" => Some(OutputRefusal::FocusUnconfirmed),
                    _ => None,
                });
        capabilities.output_refused_total = status.u64_field("output_refused_total");
        capabilities.output_uncertain_total = status.u64_field("output_uncertain_total");
        capabilities.output_partial_total = status.u64_field("output_partial_total");
        capabilities.partial_write_total = status.u64_field("partial_write_total");
        capabilities.evdev_quarantined_events_total =
            status.u64_field("evdev_quarantined_events_total");
        found |= capabilities.evdev_quarantined_events_total.is_some();
        for (key, slot) in [
            (
                "usage_flush_max_duration_us",
                &mut capabilities.usage_flush_max_duration_us,
            ),
            (
                "usage_flush_failures_total",
                &mut capabilities.usage_flush_failures_total,
            ),
            (
                "usage_worker_failures_total",
                &mut capabilities.usage_worker_failures_total,
            ),
            (
                "usage_queue_rejected_total",
                &mut capabilities.usage_queue_rejected_total,
            ),
            (
                "usage_events_recorded_total",
                &mut capabilities.usage_events_recorded_total,
            ),
            (
                "usage_events_persisted_total",
                &mut capabilities.usage_events_persisted_total,
            ),
            (
                "usage_events_dropped_total",
                &mut capabilities.usage_events_dropped_total,
            ),
        ] {
            if let Some(value) = status.u64_field(key) {
                *slot = Some(value);
                found = true;
            }
        }
        let recent_outcomes = status
            .field("recent_expansion_outcomes")
            .or_else(|| status.field("recent_expansion_failures"));
        capabilities.recent_expansion_outcomes_reported = recent_outcomes.is_some();
        capabilities.recent_expansion_outcomes = recent_outcomes
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default();
        found |= recent_outcomes.is_some();
        found |= capabilities.replacement_assurance.is_some();
        for (key, slot) in [
            (
                "capture_sensitive_focus",
                &mut capabilities.capture_sensitive_focus,
            ),
            ("capture_exclusive", &mut capabilities.capture_exclusive),
            (
                "capture_reliable_key_state",
                &mut capabilities.capture_reliable_key_state,
            ),
            (
                "capture_key_passthrough",
                &mut capabilities.capture_key_passthrough,
            ),
            (
                "capture_composition_aware",
                &mut capabilities.capture_composition_aware,
            ),
            (
                "capture_local_compose_aware",
                &mut capabilities.capture_local_compose_aware,
            ),
            (
                "capture_layout_aware",
                &mut capabilities.capture_layout_aware,
            ),
            (
                "window_tracker_connected",
                &mut capabilities.window_tracker_connected,
            ),
            (
                "window_identity_exact",
                &mut capabilities.window_identity_exact,
            ),
            (
                "inject_atomic_replace",
                &mut capabilities.inject_atomic_replace,
            ),
            ("inject_full_unicode", &mut capabilities.inject_full_unicode),
            (
                "inject_cursor_reposition",
                &mut capabilities.inject_cursor_reposition,
            ),
            (
                "inject_key_passthrough",
                &mut capabilities.inject_key_passthrough,
            ),
        ] {
            if let Some(value) = status.bool_field(key) {
                *slot = Some(value);
                found = true;
            }
        }
        found.then_some(capabilities)
    }
}

pub(crate) fn start() -> std::io::Result<(
    SyncSender<Request>,
    SyncSender<Request>,
    Receiver<Completion>,
)> {
    let (control_sender, control_receiver) = mpsc::sync_channel(REQUEST_CAPACITY);
    let (diagnostics_sender, diagnostics_receiver) = mpsc::sync_channel(REQUEST_CAPACITY);
    let (completion_sender, completion_receiver) = mpsc::sync_channel(RESULT_CAPACITY);

    let control_completion_sender = completion_sender.clone();
    thread::Builder::new()
        .name("wayexpand-gui-control".into())
        .spawn(move || {
            while let Ok(request) = control_receiver.recv() {
                let completion = match request {
                    Request::Control {
                        daemon_operation,
                        operation,
                    } => Completion::Control {
                        operation,
                        result: control_command(daemon_operation),
                    },
                    Request::ReloadConfig { path } => {
                        Completion::ConfigReloaded(Box::new(load_config_snapshot(path)))
                    }
                    Request::SaveConfig {
                        request_id,
                        path,
                        candidate,
                        base_document,
                        expected_revision,
                    } => Completion::ConfigSaved {
                        request_id,
                        result: save_config(path, *candidate, *base_document, expected_revision),
                    },
                    Request::Diagnostics { .. } | Request::SupportPreview => continue,
                };
                if control_completion_sender.send(completion).is_err() {
                    break;
                }
            }
        })?;

    thread::Builder::new()
        .name("wayexpand-gui-diagnostics".into())
        .spawn(move || {
            while let Ok(request) = diagnostics_receiver.recv() {
                let completion = match request {
                    Request::Diagnostics {
                        config_path,
                        announce,
                    } => Completion::Diagnostics(Box::new(run_diagnostics(config_path, announce))),
                    Request::SupportPreview => Completion::SupportPreview(run_support_preview()),
                    _ => continue,
                };
                if completion_sender.send(completion).is_err() {
                    break;
                }
            }
        })?;
    Ok((control_sender, diagnostics_sender, completion_receiver))
}

fn run_support_preview() -> Result<String, String> {
    use std::{
        fs,
        os::unix::fs::DirBuilderExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

    static PREVIEW_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    const LIMIT: u64 = 1024 * 1024;
    let cli = std::env::current_exe()
        .ok()
        .and_then(|gui| gui.parent().map(|parent| parent.join("wayexpand")))
        .filter(|path| path.is_file())
        .ok_or_else(|| "The WayExpand CLI executable was not found beside the GUI.".to_owned())?;
    let directory = std::env::temp_dir().join(format!(
        "wayexpand-support-preview-{}-{}.json",
        std::process::id(),
        PREVIEW_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut directory_builder = fs::DirBuilder::new();
    directory_builder.recursive(false).mode(0o700);
    directory_builder
        .create(&directory)
        .map_err(|error| format!("could not create a private preview directory: {error}"))?;
    let path = directory.join("report.json");

    let mut command = Command::new(cli);
    configure_process_group(&mut command);
    let child = match command
        .args(["support-bundle", "--output"])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = fs::remove_dir_all(&directory);
            return Err(format!(
                "could not start support report collection: {error}"
            ));
        }
    };
    let mut supervisor = ChildSupervisor::new(child);
    let deadline = Instant::now() + Duration::from_secs(25);
    let status = loop {
        if Instant::now() >= deadline {
            supervisor.kill_group();
            let _ = supervisor.reap();
            let _ = fs::remove_dir_all(&directory);
            return Err("support report collection exceeded its 25 second limit".into());
        }
        match supervisor.has_exited() {
            Ok(true) => match supervisor.reap() {
                Ok(status) => break status,
                Err(error) => {
                    let _ = fs::remove_dir_all(&directory);
                    return Err(format!("could not collect support report: {error}"));
                }
            },
            Ok(false) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                let _ = fs::remove_dir_all(&directory);
                return Err(format!("could not check report collector: {error}"));
            }
        }
    };
    if !status.success() {
        let _ = fs::remove_dir_all(&directory);
        return Err("support report collection failed; no report was saved".into());
    }
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = fs::remove_dir_all(&directory);
            return Err(format!("support report was not created: {error}"));
        }
    };
    if !metadata.file_type().is_file() || metadata.len() > LIMIT {
        let _ = fs::remove_dir_all(&directory);
        return Err("support report is not a regular file or exceeds the size limit".into());
    }
    let report = fs::read_to_string(&path).map_err(|error| error.to_string());
    let _ = fs::remove_dir_all(&directory);
    report
}

pub(crate) fn save_config(
    path: PathBuf,
    candidate: Config,
    base_document: DocumentMut,
    expected_revision: wayexpand_core::ConfigRevision,
) -> Result<(wayexpand_core::ConfigRevision, DocumentMut), SaveFailure> {
    // This is the final whole-library gate, intentionally on the save worker
    // rather than the UI thread. Nothing is serialized or committed unless
    // this complete validation succeeds.
    candidate
        .validate()
        .map_err(|error| SaveFailure::Failed(error.safe_summary()))?;
    let replacement = toml_edit::ser::to_document(&candidate).map_err(|error| {
        SaveFailure::Failed(format!("could not serialize configuration: {error}"))
    })?;
    let document = crate::persistence::merge_config_document(base_document, replacement);
    let revision = Config::save_atomic_text_if_revision_matches(
        path,
        &document.to_string(),
        &expected_revision,
    )
    .map_err(|error| match error {
        wayexpand_core::ConfigError::RevisionConflict => SaveFailure::Conflict,
        wayexpand_core::ConfigError::Busy { .. } => SaveFailure::Busy,
        error => SaveFailure::Failed(error.safe_summary()),
    })?;
    Ok((revision, document))
}

pub(crate) fn load_config_snapshot(path: PathBuf) -> Result<ReloadSnapshot, String> {
    let loaded = Config::load_versioned(&path).map_err(|error| error.safe_summary())?;
    let document = crate::persistence::read_config_document(loaded.source())
        .map_err(|error| error.to_string())?;
    let search_index = crate::library::SearchIndex::new(&loaded.config);
    Ok(ReloadSnapshot {
        config: loaded.config,
        revision: loaded.revision,
        document,
        search_index,
    })
}

fn run_diagnostics(config_path: PathBuf, announce: bool) -> DiagnosticsSnapshot {
    let backend_status = discover_backends();
    let selection_capabilities = probe_capabilities();
    let ibus_available = ibus_engine_available();
    let policy = wayexpand_core::load_organization_policy().ok();
    let recommended_route = policy.as_ref().and_then(|policy| {
        recommended_route(&selection_capabilities, ibus_available, |route| {
            wayexpand_backend_selection::route_allowed_by_policy(policy, route)
        })
    });
    let route_plan = plan_routes(&selection_capabilities, ibus_available, |route| {
        policy.as_ref().is_some_and(|policy| {
            wayexpand_backend_selection::route_allowed_by_policy(policy, route)
        })
    });
    let fleet_status = match Config::load(config_path) {
        Ok(config) => match wayexpand_core::load_organization_policy().and_then(|policy| {
            FleetConfig::load_standard_with_base_and_policy(config, &policy)
                .map_err(|error| error.to_string())
        }) {
            Ok(fleet) => {
                let mut status = format!(
                    "active · {} files · {} expansions · {} hotkeys",
                    fleet.stats.total_files_loaded,
                    fleet.stats.total_expansions,
                    fleet.stats.total_hotkeys
                );
                if !fleet.policy_violations.is_empty() {
                    status.push_str(" · policy: ");
                    status.push_str(&fleet.policy_violations.join("; "));
                }
                status
            }
            Err(error) => format!("invalid: {error}"),
        },
        Err(error) => format!("invalid: {}", error.safe_summary()),
    };
    let protocol_probes = diagnostics::probe_protocols();
    let daemon_response = control_command(wayexpand_core::DaemonOperation::Status);
    let daemon_reachable = Some(daemon_response.is_ok());
    let (daemon_status, daemon_capabilities, route_state, paused) = match daemon_response {
        Ok(response) => (
            response.trim().replace('\n', " · "),
            DaemonCapabilities::parse(&response),
            wayexpand_core::DaemonStatus::parse(&response).route_state(),
            wayexpand_core::DaemonStatus::parse(&response).bool_field("paused"),
        ),
        Err(error) => (format!("Unavailable: {error}"), None, None, None),
    };
    let compositor_info = diagnostics::compositor_info();
    let ibus_route = wayexpand_backend_ibus::chain::diagnose();
    let permission_health = diagnostics::permission_health(
        &backend_status,
        daemon_capabilities.as_ref(),
        &daemon_status,
    );
    DiagnosticsSnapshot {
        backend_status,
        recommended_route,
        route_plan,
        selection_capabilities,
        fleet_status,
        protocol_probes,
        compositor_info,
        permission_health,
        daemon_status,
        daemon_capabilities,
        daemon_reachable,
        route_state,
        paused,
        ibus_route,
        announce,
    }
}

pub(crate) fn status_field(response: &str, key: &str) -> Option<String> {
    wayexpand_core::DaemonStatus::parse(response)
        .field(key)
        .or_else(|| {
            response.split(" · ").find_map(|part| {
                part.strip_prefix(key)
                    .and_then(|part| part.strip_prefix('='))
            })
        })
        .map(str::to_owned)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DaemonBuildState {
    Current,
    RestartRequired,
    Unverified,
}

/// A responsive daemon can still be running old safety or output behavior.
/// Compare exact build identities when available, as `wayexpand doctor` does.
pub(crate) fn daemon_build_state(response: &str) -> DaemonBuildState {
    let daemon_commit = status_field(response, "daemon_commit");
    let daemon_version = status_field(response, "daemon_version");
    let gui_commit = env!("WAYEXPAND_COMMIT");
    let gui_version = env!("WAYEXPAND_BUILD_VERSION");
    if gui_version.contains(".dirty-unknown") {
        return DaemonBuildState::Unverified;
    }
    let Some(daemon_commit) = daemon_commit.filter(|commit| commit != "unknown") else {
        return DaemonBuildState::Unverified;
    };
    if gui_commit == "unknown" {
        return DaemonBuildState::Unverified;
    }
    if daemon_commit != gui_commit {
        return DaemonBuildState::RestartRequired;
    }
    match daemon_version.as_deref() {
        Some("unknown") => DaemonBuildState::Unverified,
        Some(version) if version == gui_version => DaemonBuildState::Current,
        Some(_) => DaemonBuildState::RestartRequired,
        None if gui_version.contains(".dirty.") => DaemonBuildState::Unverified,
        None => DaemonBuildState::Current,
    }
}

pub(crate) fn control_command(
    operation: wayexpand_core::DaemonOperation,
) -> anyhow::Result<String> {
    wayexpand_core::DaemonClient::from_environment()?
        .execute(operation)
        .map_err(Into::into)
}

pub(crate) fn insert_target(generation: u64, token: &str, trigger: &str) -> anyhow::Result<String> {
    wayexpand_core::DaemonClient::from_environment()?
        .execute(wayexpand_core::DaemonOperation::InsertTarget {
            generation,
            token: token.to_owned(),
            trigger: trigger.to_owned(),
        })
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    fn current_schema_fixture(fixture: &str) -> String {
        let (_, fields) = fixture
            .split_once('\n')
            .expect("status fixture includes a schema line");
        format!("status_schema={}\n{fields}", super::CONTROL_STATUS_SCHEMA)
    }

    #[test]
    fn status_field_uses_the_shared_daemon_parser() {
        assert_eq!(
            super::status_field("source=input-method\nbackend=input-method-v2\n", "backend"),
            Some("input-method-v2".into())
        );
        assert_eq!(super::status_field("state=connected\n", "backend"), None);
    }

    #[test]
    fn daemon_capability_parser_preserves_unknowns_and_separates_io_guarantees() {
        let capabilities = super::DaemonCapabilities::parse(&current_schema_fixture(
            "status_schema=38\nsource=evdev\nraw_capture_fallback_enabled=true\nraw_capture_app_count=14\ncapture_app_scope_state=allowed\nwindow_process_identity_state=compositor-process-id\ncapture_sensitive_focus=false\ncapture_exclusive=true\ncapture_composition_aware=false\ncapture_local_compose_aware=true\nwindow_tracker_connected=true\nwindow_tracker_state=reconnecting\nwindow_tracker_reconnect_attempts_total=3\nwindow_tracker_disconnects_total=2\nwindow_tracker_last_failure_reason=tracker_disconnected\nwindow_tracker_last_connected_unix=1794268800\nwindow_tracker_reconnect_backoff_ms=1000\ninject_atomic_replace=false\ninject_full_unicode=true\ninject_insertion_mode=libei keysym fallback\ninject_max_text_chars=250\ninject_expected_throughput_chars_per_sec=83\n",
        ))
        .unwrap();
        assert_eq!(capabilities.injection_mode, Some("libei keysym fallback"));
        assert_eq!(capabilities.injection_max_text_chars, Some(250));
        assert_eq!(capabilities.injection_throughput_chars_per_sec, Some(83));
        assert_eq!(capabilities.capture_sensitive_focus, Some(false));
        assert_eq!(capabilities.capture_source.as_deref(), Some("evdev"));
        assert_eq!(capabilities.raw_capture_fallback_enabled, Some(true));
        assert_eq!(capabilities.raw_capture_app_count, Some(14));
        assert_eq!(
            capabilities.capture_app_scope_state.as_deref(),
            Some("allowed")
        );
        assert_eq!(
            capabilities.window_process_identity_state.as_deref(),
            Some("compositor-process-id")
        );
        assert_eq!(capabilities.capture_exclusive, Some(true));
        assert_eq!(capabilities.capture_composition_aware, Some(false));
        assert_eq!(capabilities.capture_local_compose_aware, Some(true));
        assert_eq!(capabilities.capture_reliable_key_state, None);
        assert_eq!(capabilities.window_tracker_connected, Some(true));
        assert_eq!(
            capabilities.window_tracker_state.as_deref(),
            Some("reconnecting")
        );
        assert_eq!(
            capabilities.window_tracker_reconnect_attempts_total,
            Some(3)
        );
        assert_eq!(capabilities.window_tracker_disconnects_total, Some(2));
        assert_eq!(
            capabilities.window_tracker_last_failure_reason.as_deref(),
            Some("tracker_disconnected")
        );
        assert_eq!(
            capabilities.window_tracker_last_connected_unix,
            Some(1_794_268_800)
        );
        assert_eq!(
            capabilities.window_tracker_reconnect_backoff_ms,
            Some(1_000)
        );
        assert_eq!(capabilities.inject_atomic_replace, Some(false));
        assert_eq!(capabilities.inject_full_unicode, Some(true));
        assert_eq!(capabilities.inject_key_passthrough, None);
        assert_eq!(
            super::DaemonCapabilities::parse("status_schema=11\nstate=connected\n"),
            None
        );
        assert_eq!(
            super::DaemonCapabilities::parse("status_schema=24\nstate=connected\n"),
            None
        );
    }

    #[test]
    fn tracker_diagnostic_labels_are_allowlisted_before_gui_display() {
        let capabilities = super::DaemonCapabilities::parse(&current_schema_fixture(
            "status_schema=38\nwindow_tracker_connected=false\nwindow_tracker_state=title=private-window\nwindow_tracker_last_failure_reason=/home/user/secret\n",
        ))
        .unwrap();
        assert_eq!(capabilities.window_tracker_state, None);
        assert_eq!(capabilities.window_tracker_last_failure_reason, None);
    }

    #[test]
    fn daemon_build_state_requires_exact_tree_identity_for_dirty_builds() {
        let commit = env!("WAYEXPAND_COMMIT");
        let version = env!("WAYEXPAND_BUILD_VERSION");
        assert_eq!(
            super::daemon_build_state(&format!(
                "daemon_commit={commit}\ndaemon_version={version}\n"
            )),
            if commit == "unknown" || version == "unknown" || version.contains(".dirty-unknown") {
                super::DaemonBuildState::Unverified
            } else {
                super::DaemonBuildState::Current
            }
        );
        assert_eq!(
            super::daemon_build_state(&format!(
                "daemon_commit={commit}\ndaemon_version={version}-other-tree\n"
            )),
            if commit == "unknown" || version.contains(".dirty-unknown") {
                super::DaemonBuildState::Unverified
            } else {
                super::DaemonBuildState::RestartRequired
            }
        );
        assert_eq!(
            super::daemon_build_state(&format!("daemon_commit={commit}\n")),
            if commit == "unknown"
                || version.contains(".dirty.")
                || version.contains(".dirty-unknown")
            {
                super::DaemonBuildState::Unverified
            } else {
                super::DaemonBuildState::Current
            }
        );
        assert_eq!(
            super::daemon_build_state("daemon_commit=older-build\n"),
            if commit == "unknown" || version.contains(".dirty-unknown") {
                super::DaemonBuildState::Unverified
            } else {
                super::DaemonBuildState::RestartRequired
            }
        );
        assert_eq!(
            super::daemon_build_state("status_schema=11\n"),
            super::DaemonBuildState::Unverified
        );
        assert_eq!(
            super::daemon_build_state("daemon_commit=unknown\n"),
            super::DaemonBuildState::Unverified
        );
    }

    #[test]
    fn daemon_capability_parser_recognizes_the_negotiated_libei_fallback_label() {
        let capabilities = super::DaemonCapabilities::parse(&current_schema_fixture(
            "status_schema=38\ninject_insertion_mode=ei_keyboard keysym fallback (12ms key pacing)\ninject_max_text_chars=8\ninject_expected_throughput_chars_per_sec=83\ncompatibility_mode=tracked-focus\nmatcher_latency_p50_us=12\ninjection_latency_p95_us=3400\n",
        ))
        .unwrap();
        assert_eq!(capabilities.injection_mode, Some("libei keysym fallback"));
        assert_eq!(
            capabilities.compatibility_mode,
            Some(super::CompatibilityMode::TrackedFocus)
        );
        assert_eq!(capabilities.matcher_latency_p50_us, Some(12));
        assert_eq!(capabilities.injection_latency_p95_us, Some(3400));
        assert_eq!(
            capabilities.insertion_plan(8, 8),
            super::InsertionStrategy::PacedSyntheticKeystrokes { predicted_ms: 97 }
        );
        assert_eq!(
            capabilities.insertion_plan(9, 9),
            super::InsertionStrategy::ClipboardAssistedPaste
        );
    }

    #[test]
    fn daemon_capability_parser_reads_usage_persistence_health() {
        let capabilities = super::DaemonCapabilities::parse(&current_schema_fixture(
            "status_schema=38\nusage_flush_max_duration_us=1200\nusage_flush_failures_total=2\nusage_worker_failures_total=1\nusage_queue_rejected_total=3\nusage_events_recorded_total=100\nusage_events_persisted_total=94\nusage_events_dropped_total=6\n",
        ))
        .unwrap();
        assert_eq!(capabilities.usage_flush_max_duration_us, Some(1200));
        assert_eq!(capabilities.usage_flush_failures_total, Some(2));
        assert_eq!(capabilities.usage_worker_failures_total, Some(1));
        assert_eq!(capabilities.usage_queue_rejected_total, Some(3));
        assert_eq!(capabilities.usage_events_recorded_total, Some(100));
        assert_eq!(capabilities.usage_events_persisted_total, Some(94));
        assert_eq!(capabilities.usage_events_dropped_total, Some(6));
    }

    #[test]
    fn daemon_capability_parser_reads_replacement_assurance_and_refusals() {
        let capabilities = super::DaemonCapabilities::parse(&current_schema_fixture(
            "status_schema=38\nreplacement_assurance=focus-tracked-best-effort\nfocus_assurance=tracker\noutput_refused_total=4\noutput_last_refusal=focus-changed\noutput_uncertain_total=1\noutput_partial_total=1\nevdev_quarantined_events_total=7\n",
        ))
        .unwrap();
        assert_eq!(
            capabilities.replacement_assurance,
            Some(super::ReplacementAssurance::FocusTrackedBestEffort)
        );
        assert_eq!(capabilities.output_refused_total, Some(4));
        assert_eq!(
            capabilities.output_last_refusal,
            Some(super::OutputRefusal::FocusChanged)
        );
        assert_eq!(capabilities.output_uncertain_total, Some(1));
        assert_eq!(capabilities.output_partial_total, Some(1));
        assert_eq!(capabilities.evdev_quarantined_events_total, Some(7));
        assert!(!capabilities.recent_expansion_outcomes_reported);

        let blocked = super::DaemonCapabilities::parse(&current_schema_fixture(
            "status_schema=38\nreplacement_assurance=blocked\nfocus_assurance=unconfirmed\noutput_last_refusal=none\n",
        ))
        .unwrap();
        assert_eq!(
            blocked.replacement_assurance,
            Some(super::ReplacementAssurance::Blocked)
        );
        assert_eq!(blocked.output_last_refusal, None);

        let traced = super::DaemonCapabilities::parse(&current_schema_fixture(
            r#"status_schema=38
recent_expansion_outcomes=[{"at_unix":1,"duration_us":42,"backend":"input-method-v2","expected_insert_units":5,"trigger_recognized":"yes","focus_admission":"passed","backend_selected":"yes","erase_acknowledged":"atomic replacement acknowledged","insertion_attempted":true,"completion":"applied","application_result":"unverified"},{"at_unix":2,"duration_us":900,"backend":"libei","expected_insert_units":11,"trigger_recognized":"yes","focus_admission":"passed","backend_selected":"yes","erase_acknowledged":"yes","insertion_attempted":true,"completion":"partial_write","application_result":"unknown","recovery":"manual_review","progress_tracked":true,"erased_units":4,"inserted_units":3}]
"#,
        ))
        .unwrap();
        assert_eq!(traced.recent_expansion_outcomes.len(), 2);
        assert_eq!(traced.recent_expansion_outcomes[0].completion, "applied");
        assert_eq!(traced.recent_expansion_outcomes[0].duration_us, Some(42));
        assert_eq!(
            traced.recent_expansion_outcomes[0].expected_insert_units,
            Some(5)
        );
        assert_eq!(
            traced.recent_expansion_outcomes[0].application_result,
            "unverified"
        );
        assert_eq!(
            traced.recent_expansion_outcomes[1].completion,
            "partial_write"
        );
        assert_eq!(
            traced.recent_expansion_outcomes[1].application_result,
            "unknown"
        );
        assert!(traced.recent_expansion_outcomes[1].progress_tracked);
        assert_eq!(traced.recent_expansion_outcomes[1].erased_units, 4);
        assert_eq!(traced.recent_expansion_outcomes[1].inserted_units, 3);
        assert_eq!(
            traced.recent_expansion_outcomes[1].interrupt_to_backend_return_us,
            None
        );
        assert!(traced.recent_expansion_outcomes_reported);
    }

    #[test]
    fn interrupted_output_timing_is_parsed_without_claiming_visible_text_latency() {
        let traced = super::DaemonCapabilities::parse(&current_schema_fixture(
            r#"status_schema=38
output_interrupt_to_backend_return_sample_count=8
output_interrupt_to_backend_return_p50_us=1200
output_interrupt_to_backend_return_p95_us=2400
output_interrupt_to_backend_return_p99_us=4100
recent_expansion_outcomes=[{"completion":"partial_write","application_result":"unknown","interrupt_to_backend_return_us":1800}]
"#,
        ))
        .unwrap();
        assert_eq!(
            traced.recent_expansion_outcomes[0].interrupt_to_backend_return_us,
            Some(1800)
        );
        assert_eq!(traced.output_interrupt_sample_count, Some(8));
        assert_eq!(traced.output_interrupt_p50_us, Some(1200));
        assert_eq!(traced.output_interrupt_p95_us, Some(2400));
        assert_eq!(traced.output_interrupt_p99_us, Some(4100));
        let label = crate::lang::Strings::new(crate::lang::Language::English)
            .trace_interrupt_timing(Some(1800))
            .unwrap();
        assert!(label.contains("backend returned in 1800 µs"));
        assert!(label.contains("client result unverified"));
    }

    #[test]
    fn daemon_capability_parser_rejects_stale_or_missing_status_schema() {
        let capabilities = "capture_sensitive_focus=true\ninject_atomic_replace=true\n";
        let current_schema = super::CONTROL_STATUS_SCHEMA;
        assert_eq!(
            super::DaemonCapabilities::parse(&format!("status_schema=5\n{capabilities}")),
            None
        );
        assert_eq!(
            super::DaemonCapabilities::parse(&format!(
                "status_schema={}\n{capabilities}",
                super::CONTROL_STATUS_SCHEMA + 1
            )),
            None
        );
        assert_eq!(
            super::DaemonCapabilities::parse(&format!(
                "status_schema=invalid\nstatus_schema={current_schema}\n{capabilities}"
            )),
            None
        );
        assert_eq!(
            super::DaemonCapabilities::parse(&format!(
                "status_schema={current_schema}\nstatus_schema=5\n{capabilities}"
            )),
            None
        );
        assert_eq!(super::DaemonCapabilities::parse(capabilities), None);
    }

    #[test]
    fn full_protection_requires_every_end_to_end_guarantee() {
        let full = super::DaemonCapabilities {
            capture_sensitive_focus: Some(true),
            capture_key_passthrough: Some(true),
            capture_composition_aware: Some(true),
            capture_layout_aware: Some(true),
            window_identity_exact: Some(true),
            inject_atomic_replace: Some(true),
            inject_full_unicode: Some(true),
            inject_key_passthrough: Some(true),
            ..Default::default()
        };
        assert!(full.provides_full_protection());
        assert!(!super::DaemonCapabilities::default().provides_full_protection());
        assert!(!super::DaemonCapabilities {
            window_identity_exact: Some(false),
            ..full
        }
        .provides_full_protection());
    }
}
