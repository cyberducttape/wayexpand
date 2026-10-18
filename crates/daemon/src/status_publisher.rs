//! Deduplicating status publication: carries backend mode and runtime capabilities forward between updates.

use crate::*;

#[derive(Default)]
pub(crate) struct StatusPublisher {
    last: Option<StatusSnapshot>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct StatusSnapshot {
    source: String,
    backend: String,
    backend_mode: String,
    state: String,
    paused: bool,
    config_path: PathBuf,
    config_healthy: bool,
    metrics: CommandMetrics,
    latency: latency::Snapshot,
    capture_capabilities: InputSourceCapabilities,
    injection_capabilities: InjectorCapabilities,
    window_tracker_connected: bool,
    window_identity_exact: bool,
}

impl StatusPublisher {
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &mut self,
        control: &control::ControlServer,
        source: &str,
        backend: &str,
        backend_mode: &str,
        state: &str,
        config_path: &Path,
        config_healthy: bool,
        metrics: CommandMetrics,
        capture_capabilities: InputSourceCapabilities,
        injection_capabilities: InjectorCapabilities,
        window_tracker_connected: bool,
    ) {
        let snapshot = StatusSnapshot {
            source: source.to_owned(),
            backend: backend.to_owned(),
            backend_mode: backend_mode.to_owned(),
            state: state.to_owned(),
            paused: control
                .pause_requested
                .load(std::sync::atomic::Ordering::Acquire),
            config_path: config_path.to_path_buf(),
            config_healthy,
            metrics,
            latency: latency::snapshot(),
            capture_capabilities,
            injection_capabilities,
            window_tracker_connected,
            window_identity_exact: control.focus_snapshot().exact_window_identity,
        };
        // Always rebuild the body: other paths (output reconnect, input-method
        // pass-through setup) publish directly through `crate::status`, so an
        // unchanged snapshot does not mean the served status is unchanged.
        // Skipping here once left a capability-less "connected" status in
        // place after a reconnect. `ControlServer::set_status` already
        // ignores a body identical to the one being served.
        control.set_status(status::daemon_status_body_with_runtime_capabilities(
            &snapshot.source,
            &snapshot.backend,
            &snapshot.state,
            snapshot.paused,
            snapshot.config_path.as_path(),
            snapshot.config_healthy,
            &snapshot.backend_mode,
            snapshot.metrics,
            snapshot.latency,
            snapshot.capture_capabilities,
            snapshot.injection_capabilities,
            snapshot.window_tracker_connected,
            snapshot.window_identity_exact,
        ));
        self.last = Some(snapshot);
    }
}

pub(crate) fn set_daemon_status(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
) {
    set_daemon_status_with_metrics(
        publisher,
        control,
        source,
        backend,
        state,
        config_path,
        config_healthy,
        CommandMetrics::default(),
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn set_daemon_status_with_metrics(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
    metrics: CommandMetrics,
) {
    let connected = state == "connected";
    let backend_mode = if connected {
        publisher.last.as_ref().map_or_else(
            || "unknown".to_owned(),
            |snapshot| snapshot.backend_mode.clone(),
        )
    } else {
        "unknown".to_owned()
    };
    let capture_capabilities = if connected {
        publisher
            .last
            .as_ref()
            .map_or_else(InputSourceCapabilities::default, |snapshot| {
                snapshot.capture_capabilities
            })
    } else {
        InputSourceCapabilities::default()
    };
    let injection_capabilities = if connected {
        publisher
            .last
            .as_ref()
            .map_or_else(InjectorCapabilities::default, |snapshot| {
                snapshot.injection_capabilities
            })
    } else {
        InjectorCapabilities::default()
    };
    let window_tracker_connected = publisher
        .last
        .as_ref()
        .is_some_and(|snapshot| snapshot.window_tracker_connected);
    publisher.publish(
        control,
        source,
        backend,
        &backend_mode,
        state,
        config_path,
        config_healthy,
        metrics,
        capture_capabilities,
        injection_capabilities,
        window_tracker_connected,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn set_daemon_status_with_runtime_capabilities(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
    metrics: CommandMetrics,
    backend_mode: &str,
    capture_capabilities: InputSourceCapabilities,
    injection_capabilities: InjectorCapabilities,
    window_tracker_connected: bool,
) {
    publisher.publish(
        control,
        source,
        backend,
        backend_mode,
        state,
        config_path,
        config_healthy,
        metrics,
        capture_capabilities,
        injection_capabilities,
        window_tracker_connected,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn republishing_restores_status_overwritten_by_a_direct_update() {
        let control = control::ControlServer::detached();
        let mut publisher = StatusPublisher::default();
        let capabilities = InjectorCapabilities {
            atomic_replace: true,
            ..InjectorCapabilities::default()
        };
        let publish = |publisher: &mut StatusPublisher| {
            set_daemon_status_with_runtime_capabilities(
                publisher,
                &control,
                "evdev",
                "libei",
                "connected",
                Path::new("/tmp/config.toml"),
                true,
                CommandMetrics::default(),
                "ei_text",
                InputSourceCapabilities::default(),
                capabilities,
                false,
            );
        };
        // Latency, usage, and output counters are process-wide and other
        // tests update them concurrently; compare only what the publisher
        // sets.
        let published = |control: &control::ControlServer| {
            control
                .status_text()
                .lines()
                .filter(|line| {
                    ![
                        "injection_latency_",
                        "matcher_latency_",
                        "usage_",
                        "output_",
                        "input_",
                        "control_",
                        "memory_",
                    ]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
                })
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        publish(&mut publisher);
        let full = published(&control);
        assert!(
            full.iter().any(|line| line == "inject_atomic_replace=true"),
            "{full:?}"
        );

        // An output reconnect publishes directly, without capabilities.
        status::set_daemon_status_direct(
            &control,
            "evdev",
            "libei",
            "connected",
            Path::new("/tmp/config.toml"),
            true,
        );
        assert_ne!(published(&control), full);

        publish(&mut publisher);
        assert_eq!(published(&control), full);
    }
}
