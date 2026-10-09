use std::path::Path;

use crate::build_info;
use crate::control::ControlServer;
use crate::latency::Snapshot as LatencySnapshot;
use wayexpand_core::{
    CommandMetrics, InjectorCapabilities, InputSourceCapabilities, CONTROL_STATUS_SCHEMA,
};

/// A complete control-socket status body.
///
/// `daemon_status_body_with_mode` is the only way to build one, and
/// `ControlServer::set_status` accepts nothing else, so every status the
/// daemon publishes carries the whole documented field set. This is not
/// ceremony: a hand-formatted status line in the input-method reconnect path
/// shipped without `backend_mode` or any of the nine command-metric fields,
/// and asserted `paused=false` and `config_state=ok` whatever the real state
/// was. The contract test covered this builder, which that line never called.
pub struct StatusBody(String);

impl StatusBody {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

#[allow(clippy::too_many_arguments)]
pub fn daemon_status_body_with_mode(
    source: &str,
    backend: &str,
    state: &str,
    paused: bool,
    config_path: &Path,
    config_healthy: bool,
    backend_mode: &str,
    metrics: CommandMetrics,
) -> StatusBody {
    daemon_status_body_with_latency(
        source,
        backend,
        state,
        paused,
        config_path,
        config_healthy,
        backend_mode,
        metrics,
        crate::latency::snapshot(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn daemon_status_body_with_latency(
    source: &str,
    backend: &str,
    state: &str,
    paused: bool,
    config_path: &Path,
    config_healthy: bool,
    backend_mode: &str,
    metrics: CommandMetrics,
    latency: LatencySnapshot,
) -> StatusBody {
    daemon_status_body_with_runtime_capabilities(
        source,
        backend,
        state,
        paused,
        config_path,
        config_healthy,
        backend_mode,
        metrics,
        latency,
        InputSourceCapabilities::default(),
        InjectorCapabilities::default(),
        false,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn daemon_status_body_with_runtime_capabilities(
    source: &str,
    backend: &str,
    state: &str,
    paused: bool,
    config_path: &Path,
    config_healthy: bool,
    backend_mode: &str,
    metrics: CommandMetrics,
    latency: LatencySnapshot,
    capture: InputSourceCapabilities,
    injection: InjectorCapabilities,
    window_tracker_connected: bool,
    window_identity_exact: bool,
) -> StatusBody {
    let matcher_latency = crate::latency::matcher_snapshot();
    let injection_latency_profiles = crate::latency::profiles_json();
    StatusBody(format!(
        "source={source}\nbackend={backend}\nbackend_mode={backend_mode}\nstatus_schema={CONTROL_STATUS_SCHEMA}\ndaemon_commit={}\nstate={state}\npaused={}\nconfig={}\nconfig_state={}\ncapture_sensitive_focus={}\ncapture_exclusive={}\ncapture_reliable_key_state={}\ncapture_key_passthrough={}\ncapture_composition_aware={}\ncapture_local_compose_aware={}\ncapture_layout_aware={}\nwindow_tracker_connected={}\nwindow_identity_exact={}\ninject_atomic_replace={}\ninject_full_unicode={}\ninject_cursor_reposition={}\ninject_key_passthrough={}\ninject_insertion_mode={}\ninject_max_text_chars={}\ninject_expected_throughput_chars_per_sec={}\ncommand_queue_depth={}\ncommand_in_flight={}\nexpansion_command_queue_depth={}\nexpansion_command_in_flight={}\nhotkey_queue_depth={}\nhotkey_in_flight={}\ncommand_queue_rejected_total={}\ncommand_timeout_total={}\ncommand_failure_total={}\ninjection_latency_sample_count={}\ninjection_latency_window_count={}\ninjection_latency_p50_us={}\ninjection_latency_p95_us={}\ninjection_latency_p99_us={}\ninjection_latency_profiles={}\nmatcher_latency_sample_count={}\nmatcher_latency_window_count={}\nmatcher_latency_p50_us={}\nmatcher_latency_p95_us={}\nmatcher_latency_p99_us={}",
        build_info::COMMIT,
        paused,
        config_path.display(),
        if config_healthy { "ok" } else { "reload-rejected" },
        capture.sensitive_focus,
        capture.exclusive_capture,
        capture.reliable_key_state,
        capture.key_passthrough,
        capture.composition_aware,
        capture.local_compose_aware,
        capture.layout_aware,
        window_tracker_connected,
        window_identity_exact,
        injection.atomic_replace,
        injection.full_unicode,
        injection.cursor_reposition,
        injection.key_passthrough,
        injection.insertion_mode,
        injection.max_text_chars,
        injection
            .expected_throughput_chars_per_sec
            .unwrap_or(0),
        metrics.command_queue_depth,
        metrics.command_in_flight,
        metrics.expansion_command_queue_depth,
        metrics.expansion_command_in_flight,
        metrics.hotkey_queue_depth,
        metrics.hotkey_in_flight,
        metrics.command_queue_rejected_total,
        metrics.command_timeout_total,
        metrics.command_failure_total,
        latency.sample_count,
        latency.window_count,
        latency.p50_us,
        latency.p95_us,
        latency.p99_us,
        injection_latency_profiles,
        matcher_latency.sample_count,
        matcher_latency.window_count,
        matcher_latency.p50_us,
        matcher_latency.p95_us,
        matcher_latency.p99_us,
    ))
}

pub fn set_daemon_status(
    control: &ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
    metrics: CommandMetrics,
) {
    set_daemon_status_with_mode(
        control,
        source,
        backend,
        state,
        config_path,
        config_healthy,
        "unknown",
        metrics,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn set_daemon_status_with_mode(
    control: &ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
    backend_mode: &str,
    metrics: CommandMetrics,
) {
    control.set_status(daemon_status_body_with_mode(
        source,
        backend,
        state,
        control
            .pause_requested
            .load(std::sync::atomic::Ordering::Acquire),
        config_path,
        config_healthy,
        backend_mode,
        metrics,
    ));
}

pub fn set_daemon_status_direct(
    control: &ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
) {
    set_daemon_status(
        control,
        source,
        backend,
        state,
        config_path,
        config_healthy,
        CommandMetrics::default(),
    );
}
