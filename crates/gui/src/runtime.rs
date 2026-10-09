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
    probe_capabilities, recommended_route, Capabilities, RecommendedRoute,
};
use wayexpand_core::{discover_backends, Config, DaemonStatus, FleetConfig, CONTROL_STATUS_SCHEMA};

const REQUEST_CAPACITY: usize = 8;
const RESULT_CAPACITY: usize = 16;

pub(crate) enum Request {
    Diagnostics {
        config_path: PathBuf,
        announce: bool,
    },
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
    Diagnostics(DiagnosticsSnapshot),
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
    pub selection_capabilities: Capabilities,
    pub fleet_status: String,
    pub protocol_probes: Vec<(String, String)>,
    pub daemon_status: String,
    pub daemon_capabilities: Option<DaemonCapabilities>,
    pub daemon_reachable: Option<bool>,
    pub route_state: Option<RouteState>,
    pub paused: Option<bool>,
    pub announce: bool,
}

pub(crate) type RouteState = wayexpand_core::DaemonRouteState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DaemonCapabilities {
    pub injection_mode: Option<&'static str>,
    pub injection_max_text_chars: Option<usize>,
    pub injection_throughput_chars_per_sec: Option<u32>,
    pub injection_latency_p50_us: Option<u64>,
    pub injection_latency_p95_us: Option<u64>,
    pub injection_latency_p99_us: Option<u64>,
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
    pub window_identity_exact: Option<bool>,
    pub inject_atomic_replace: Option<bool>,
    pub inject_full_unicode: Option<bool>,
    pub inject_cursor_reposition: Option<bool>,
    pub inject_key_passthrough: Option<bool>,
}

impl DaemonCapabilities {
    pub fn provides_full_protection(self) -> bool {
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
                    Request::Diagnostics { .. } => continue,
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
                let Request::Diagnostics {
                    config_path,
                    announce,
                } = request
                else {
                    continue;
                };
                let completion = Completion::Diagnostics(run_diagnostics(config_path, announce));
                if completion_sender.send(completion).is_err() {
                    break;
                }
            }
        })?;
    Ok((control_sender, diagnostics_sender, completion_receiver))
}

pub(crate) fn save_config(
    path: PathBuf,
    candidate: Config,
    base_document: DocumentMut,
    expected_revision: wayexpand_core::ConfigRevision,
) -> Result<(wayexpand_core::ConfigRevision, DocumentMut), SaveFailure> {
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

fn load_config_snapshot(path: PathBuf) -> Result<ReloadSnapshot, String> {
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
    let recommended_route = wayexpand_core::load_organization_policy()
        .ok()
        .and_then(|policy| {
            recommended_route(&selection_capabilities, ibus_available, |route| {
                wayexpand_backend_selection::route_allowed_by_policy(&policy, route)
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
    DiagnosticsSnapshot {
        backend_status,
        recommended_route,
        selection_capabilities,
        fleet_status,
        protocol_probes,
        daemon_status,
        daemon_capabilities,
        daemon_reachable,
        route_state,
        paused,
        announce,
    }
}

pub(crate) fn status_field(response: &str, key: &str) -> Option<String> {
    wayexpand_core::DaemonStatus::parse(response)
        .field(key)
        .map(str::to_owned)
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
        let capabilities = super::DaemonCapabilities::parse(
            "status_schema=6\ncapture_sensitive_focus=false\ncapture_exclusive=true\ncapture_composition_aware=false\ncapture_local_compose_aware=true\nwindow_tracker_connected=true\ninject_atomic_replace=false\ninject_full_unicode=true\ninject_insertion_mode=libei keysym fallback\ninject_max_text_chars=250\ninject_expected_throughput_chars_per_sec=83\n",
        )
        .unwrap();
        assert_eq!(capabilities.injection_mode, Some("libei keysym fallback"));
        assert_eq!(capabilities.injection_max_text_chars, Some(250));
        assert_eq!(capabilities.injection_throughput_chars_per_sec, Some(83));
        assert_eq!(capabilities.capture_sensitive_focus, Some(false));
        assert_eq!(capabilities.capture_exclusive, Some(true));
        assert_eq!(capabilities.capture_composition_aware, Some(false));
        assert_eq!(capabilities.capture_local_compose_aware, Some(true));
        assert_eq!(capabilities.capture_reliable_key_state, None);
        assert_eq!(capabilities.window_tracker_connected, Some(true));
        assert_eq!(capabilities.inject_atomic_replace, Some(false));
        assert_eq!(capabilities.inject_full_unicode, Some(true));
        assert_eq!(capabilities.inject_key_passthrough, None);
        assert_eq!(
            super::DaemonCapabilities::parse("status_schema=6\nstate=connected\n"),
            None
        );
    }

    #[test]
    fn daemon_capability_parser_recognizes_the_negotiated_libei_fallback_label() {
        let capabilities = super::DaemonCapabilities::parse(
            "status_schema=6\ninject_insertion_mode=ei_keyboard keysym fallback (12ms key pacing)\nmatcher_latency_p50_us=12\ninjection_latency_p95_us=3400\n",
        )
        .unwrap();
        assert_eq!(capabilities.injection_mode, Some("libei keysym fallback"));
        assert_eq!(capabilities.matcher_latency_p50_us, Some(12));
        assert_eq!(capabilities.injection_latency_p95_us, Some(3400));
    }

    #[test]
    fn daemon_capability_parser_rejects_stale_or_missing_status_schema() {
        let capabilities = "capture_sensitive_focus=true\ninject_atomic_replace=true\n";
        assert_eq!(
            super::DaemonCapabilities::parse(&format!("status_schema=5\n{capabilities}")),
            None
        );
        assert_eq!(
            super::DaemonCapabilities::parse(&format!("status_schema=7\n{capabilities}")),
            None
        );
        assert_eq!(
            super::DaemonCapabilities::parse(&format!(
                "status_schema=invalid\nstatus_schema=6\n{capabilities}"
            )),
            None
        );
        assert_eq!(
            super::DaemonCapabilities::parse(&format!(
                "status_schema=6\nstatus_schema=5\n{capabilities}"
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
