//! Deduplicating status publication: carries backend mode and runtime capabilities forward between updates.

use crate::*;

#[derive(Default)]
pub(crate) struct StatusPublisher {
    last: Option<StatusSnapshot>,
    last_reactor_refresh: Option<std::time::Instant>,
    backend_policy_blocked: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct StatusSnapshot {
    source: String,
    backend: String,
    backend_mode: String,
    state: String,
    paused: bool,
    config_path: PathBuf,
    config_state: String,
    metrics: CommandMetrics,
    latency: latency::Snapshot,
    capture_capabilities: InputSourceCapabilities,
    injection_capabilities: InjectorCapabilities,
    window_tracker_connected: bool,
    window_identity_exact: bool,
    backend_policy_blocked: bool,
}

impl StatusPublisher {
    pub(crate) fn update_backend_policy_blocked(&mut self, blocked: bool) -> bool {
        let changed = self.backend_policy_blocked != blocked;
        self.backend_policy_blocked = blocked;
        changed
    }

    /// Limit diagnostic serialization on ordinary reactor turns. State
    /// transitions and output reconnect paths still publish immediately via
    /// `publish`; this cadence only applies to the periodic refresh.
    pub(crate) fn reactor_refresh_due(&mut self) -> bool {
        self.reactor_refresh_due_at(std::time::Instant::now())
    }

    fn reactor_refresh_due_at(&mut self, now: std::time::Instant) -> bool {
        if self
            .last_reactor_refresh
            .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(1))
        {
            return false;
        }
        self.last_reactor_refresh = Some(now);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn publish(
        &mut self,
        control: &control::ControlServer,
        source: &str,
        backend: &str,
        backend_mode: &str,
        state: &str,
        config_path: &Path,
        config_state: &str,
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
            config_state: config_state.to_owned(),
            metrics,
            latency: latency::snapshot(),
            capture_capabilities,
            injection_capabilities,
            window_tracker_connected,
            window_identity_exact: control.focus_snapshot().exact_window_identity,
            backend_policy_blocked: self.backend_policy_blocked,
        };
        // Always rebuild the body: other paths (output reconnect, input-method
        // pass-through setup) publish directly through `crate::status`, so an
        // unchanged snapshot does not mean the served status is unchanged.
        // Skipping here once left a capability-less "connected" status in
        // place after a reconnect. `ControlServer::set_status` already
        // ignores a body identical to the one being served.
        control.set_status(
            status::daemon_status_body_with_runtime_capabilities_config_state_and_policy(
                &snapshot.source,
                &snapshot.backend,
                &snapshot.state,
                snapshot.paused,
                snapshot.config_path.as_path(),
                &snapshot.config_state,
                &snapshot.backend_mode,
                snapshot.metrics,
                snapshot.latency,
                snapshot.capture_capabilities,
                snapshot.injection_capabilities,
                snapshot.window_tracker_connected,
                snapshot.window_identity_exact,
                snapshot.backend_policy_blocked,
            ),
        );
        self.last = Some(snapshot);
        // A direct state transition also refreshes the periodic cadence so the
        // next ordinary reactor turn does not rebuild diagnostics twice.
        self.last_reactor_refresh = Some(std::time::Instant::now());
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
    set_daemon_status_with_config_state_and_metrics(
        publisher,
        control,
        source,
        backend,
        state,
        config_path,
        if config_healthy {
            "ok"
        } else {
            "reload-rejected"
        },
        metrics,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn set_daemon_status_with_config_state_and_metrics(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_state: &str,
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
        config_state,
        metrics,
        capture_capabilities,
        injection_capabilities,
        window_tracker_connected,
    );
}

pub(crate) fn set_daemon_status_with_config_state(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_state: &str,
) {
    set_daemon_status_with_config_state_and_metrics(
        publisher,
        control,
        source,
        backend,
        state,
        config_path,
        config_state,
        CommandMetrics::default(),
    );
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
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
    set_daemon_status_with_runtime_capabilities_and_config_state(
        publisher,
        control,
        source,
        backend,
        state,
        config_path,
        if config_healthy {
            "ok"
        } else {
            "reload-rejected"
        },
        metrics,
        backend_mode,
        capture_capabilities,
        injection_capabilities,
        window_tracker_connected,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn set_daemon_status_with_runtime_capabilities_and_config_state(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_state: &str,
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
        config_state,
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
    fn periodic_reactor_status_refresh_is_bounded_to_one_per_second() {
        let mut publisher = StatusPublisher::default();
        let start = std::time::Instant::now();
        assert!(publisher.reactor_refresh_due_at(start));
        assert!(!publisher.reactor_refresh_due_at(start + std::time::Duration::from_millis(999)));
        assert!(publisher.reactor_refresh_due_at(start + std::time::Duration::from_secs(1)));
    }

    #[test]
    fn republishing_restores_status_overwritten_by_a_direct_update() {
        let control = control::ControlServer::detached();
        let mut publisher = StatusPublisher::default();
        let capabilities = InjectorCapabilities {
            atomic_replace: true,
            replacement_guarantee: wayexpand_core::ReplacementGuarantee::Atomic,
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
                InputSourceCapabilities {
                    layout_aware: true,
                    ..InputSourceCapabilities::default()
                },
                capabilities,
                false,
            );
        };
        // Diagnostic counters, the recent outcome rings, and assurance state
        // are process-wide and other tests update them concurrently. Compare
        // the fields supplied by this publisher explicitly, so a newly added
        // process-global status field cannot make this assertion flaky.
        const PUBLISHER_FIELDS: &[&str] = &[
            "source",
            "backend",
            "route_id",
            "backend_mode",
            "status_schema",
            "daemon_commit",
            "daemon_version",
            "state",
            "paused",
            "config",
            "config_state",
            "capture_sensitive_focus",
            "capture_exclusive",
            "capture_reliable_key_state",
            "capture_key_passthrough",
            "capture_composition_aware",
            "capture_local_compose_aware",
            "capture_layout_aware",
            "capture_layout_state",
            "window_tracker_connected",
            "window_identity_exact",
            "inject_atomic_replace",
            "inject_full_unicode",
            "inject_cursor_reposition",
            "inject_key_passthrough",
            "inject_insertion_mode",
            "inject_max_text_chars",
            "inject_expected_throughput_chars_per_sec",
            "command_queue_depth",
            "command_in_flight",
            "expansion_command_queue_depth",
            "expansion_command_in_flight",
            "hotkey_queue_depth",
            "hotkey_in_flight",
            "command_queue_rejected_total",
            "command_timeout_total",
            "command_cancelled_total",
            "command_failure_total",
            "backend_policy_blocked",
        ];
        let published = |control: &control::ControlServer| {
            let status = control.status_text();
            let fields = status
                .lines()
                .filter(|line| {
                    line.split_once('=')
                        .is_some_and(|(key, _)| PUBLISHER_FIELDS.contains(&key))
                })
                .map(str::to_owned)
                .collect::<Vec<_>>();
            assert_eq!(fields.len(), PUBLISHER_FIELDS.len(), "{status}");
            fields
        };
        publish(&mut publisher);
        assert!(control
            .status_text()
            .contains("backend_policy_blocked=false\n"));
        assert!(publisher.update_backend_policy_blocked(true));
        publish(&mut publisher);
        let full = published(&control);
        assert!(full
            .iter()
            .any(|line| line == "backend_policy_blocked=true"));
        assert!(control
            .status_text()
            .contains("matching_unavailable_reason=organization_policy_route_incompatible\n"));
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
