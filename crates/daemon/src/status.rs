use std::path::Path;

use crate::control::ControlServer;

pub fn daemon_status_body(
    source: &str,
    backend: &str,
    state: &str,
    paused: bool,
    config_path: &Path,
    config_healthy: bool,
) -> String {
    format!(
        "source={source}\nbackend={backend}\nstate={state}\npaused={paused}\nconfig={}\nconfig_state={}",
        config_path.display(),
        if config_healthy { "ok" } else { "reload-rejected" },
    )
}

pub fn set_daemon_status(
    control: &ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
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
    ));
}
