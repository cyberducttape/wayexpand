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
        "source={source}\nbackend={backend}\nstate={state}\npaused={paused}\nconfig={}\nconfig_state={}\ncommand_queue_depth={}\ncommand_in_flight={}\nexpansion_command_queue_depth={}\nexpansion_command_in_flight={}\nhotkey_queue_depth={}\nhotkey_in_flight={}\ncommand_queue_rejected_total={}\ncommand_timeout_total={}\ncommand_failure_total={}",
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
