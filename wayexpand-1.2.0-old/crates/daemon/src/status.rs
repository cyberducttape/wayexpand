use std::path::Path;

use crate::control::ControlServer;
use wayexpand_core::CommandMetrics;

pub fn daemon_status_body(
    source: &str,
    backend: &str,
    state: &str,
    paused: bool,
    config_path: &Path,
    config_healthy: bool,
    metrics: CommandMetrics,
) -> String {
    format!(
        "source={source}\nbackend={backend}\nstate={state}\npaused={paused}\nconfig={}\nconfig_state={}\ncommand_queue_depth={}\ncommand_queue_rejected_total={}\ncommand_timeout_total={}\ncommand_failure_total={}",
        config_path.display(),
        if config_healthy { "ok" } else { "reload-rejected" },
        metrics.command_queue_depth,
        metrics.command_queue_rejected_total,
        metrics.command_timeout_total,
        metrics.command_failure_total,
    )
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
    control.set_status(daemon_status_body(
        source,
        backend,
        state,
        control
            .pause_requested
            .load(std::sync::atomic::Ordering::Acquire),
        config_path,
        config_healthy,
        metrics,
    ));
}
