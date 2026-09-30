use std::path::Path;

use crate::control::ControlServer;
use crate::latency::Snapshot as LatencySnapshot;
use wayexpand_core::CommandMetrics;

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
    StatusBody(format!(
        "source={source}\nbackend={backend}\nbackend_mode={backend_mode}\nstate={state}\npaused={paused}\nconfig={}\nconfig_state={}\ncommand_queue_depth={}\ncommand_in_flight={}\nexpansion_command_queue_depth={}\nexpansion_command_in_flight={}\nhotkey_queue_depth={}\nhotkey_in_flight={}\ncommand_queue_rejected_total={}\ncommand_timeout_total={}\ncommand_failure_total={}\ninjection_latency_sample_count={}\ninjection_latency_window_count={}\ninjection_latency_p50_us={}\ninjection_latency_p95_us={}\ninjection_latency_p99_us={}",
        config_path.display(),
        if config_healthy { "ok" } else { "reload-rejected" },
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

#[allow(clippy::too_many_arguments)]
pub fn set_daemon_status_with_latency(
    control: &ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
    metrics: CommandMetrics,
    latency: LatencySnapshot,
) {
    control.set_status(daemon_status_body_with_latency(
        source,
        backend,
        state,
        control
            .pause_requested
            .load(std::sync::atomic::Ordering::Acquire),
        config_path,
        config_healthy,
        "unknown",
        metrics,
        latency,
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
