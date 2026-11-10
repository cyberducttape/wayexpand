use std::path::Path;

use crate::build_info;
use crate::control::ControlServer;
use crate::latency::Snapshot as LatencySnapshot;
use crate::output_connect::worker_stats;
use wayexpand_core::{
    CommandMetrics, InjectorCapabilities, InputSourceCapabilities, CONTROL_STATUS_SCHEMA,
};

/// Keep values safe for the line-oriented status protocol. Paths are user
/// supplied and may contain newlines or other control characters, which must
/// not be allowed to create or overwrite status fields.
fn escape_status_value(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

fn matching_unavailable_reason(
    source: &str,
    state: &str,
    layout_aware: bool,
    injection_available: bool,
    replacement_admitted: bool,
    paused: bool,
    config_state: &str,
) -> &'static str {
    if state != "connected" {
        "capture_disconnected"
    } else if source == "evdev" && !layout_aware {
        "keyboard_layout_unavailable"
    } else if !injection_available {
        "replacement_route_unavailable"
    } else if paused {
        "paused"
    } else if config_state != "ok" {
        "configuration_unhealthy"
    } else if !replacement_admitted {
        "replacement_not_admitted"
    } else {
        "none"
    }
}

fn replacement_is_admitted(
    injection: &InjectorCapabilities,
    mode: wayexpand_core::CompatibilityMode,
    focus: wayexpand_core::FocusCheck,
) -> bool {
    injection.replacement_admission(mode) != wayexpand_core::ReplacementAdmission::Refused
        && crate::assurance::Assurance::of(Some(injection), mode, focus)
            != crate::assurance::Assurance::Blocked
}

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
    daemon_status_body_with_runtime_capabilities_and_config_state(
        source,
        backend,
        state,
        paused,
        config_path,
        if config_healthy {
            "ok"
        } else {
            "reload-rejected"
        },
        backend_mode,
        metrics,
        latency,
        capture,
        injection,
        window_tracker_connected,
        window_identity_exact,
    )
}

/// Set once at startup by `--terminal-apps-only`; reported as
/// `terminal_apps_only` so diagnostics can tell a terminal-scoped raw-input
/// daemon (safe beside IBus) from one that would expand everywhere.
static TERMINAL_APPS_ONLY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
const RAW_CAPTURE_ENABLED_BIT: usize = 1usize << (usize::BITS - 1);
const RAW_CAPTURE_COUNT_MASK: usize = !RAW_CAPTURE_ENABLED_BIT;
static RAW_CAPTURE_SCOPE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static CAPTURE_APP_SCOPE_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static INPUT_EVENTS_TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WINDOW_TRACKER_UPDATES_TOTAL: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
const WINDOW_CONTEXT_AVAILABLE_BIT: u8 = 1;
const WINDOW_APP_ID_AVAILABLE_BIT: u8 = 2;
static WINDOW_CONTEXT_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub(crate) fn record_window_tracker_update(window: Option<&wayexpand_core::WindowContext>) {
    WINDOW_TRACKER_UPDATES_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    WINDOW_CONTEXT_STATE.store(
        window_context_state_code(window),
        std::sync::atomic::Ordering::Relaxed,
    );
}

fn window_context_state_code(window: Option<&wayexpand_core::WindowContext>) -> u8 {
    (u8::from(window.is_some()) * WINDOW_CONTEXT_AVAILABLE_BIT)
        | (u8::from(
            window
                .and_then(|context| context.app_id.as_deref())
                .is_some(),
        ) * WINDOW_APP_ID_AVAILABLE_BIT)
}

fn window_tracker_updates_total() -> u64 {
    WINDOW_TRACKER_UPDATES_TOTAL.load(std::sync::atomic::Ordering::Relaxed)
}

fn window_context_state() -> (bool, bool) {
    let state = WINDOW_CONTEXT_STATE.load(std::sync::atomic::Ordering::Relaxed);
    (
        state & WINDOW_CONTEXT_AVAILABLE_BIT != 0,
        state & WINDOW_APP_ID_AVAILABLE_BIT != 0,
    )
}

fn capture_app_scope_state(restricted: bool, focus_known: bool, allowed: bool) -> &'static str {
    if !restricted {
        "unrestricted"
    } else if !focus_known {
        "unknown"
    } else if allowed {
        "allowed"
    } else {
        "outside_allowlist"
    }
}

fn capture_app_scope_state_code(state: &str) -> u8 {
    match state {
        "unrestricted" => 1,
        "allowed" => 2,
        "outside_allowlist" => 3,
        _ => 0,
    }
}

fn capture_app_scope_state_name(code: u8) -> &'static str {
    match code {
        1 => "unrestricted",
        2 => "allowed",
        3 => "outside_allowlist",
        _ => "unknown",
    }
}

pub(crate) fn set_capture_app_scope_state(restricted: bool, focus_known: bool, allowed: bool) {
    let state = capture_app_scope_state(restricted, focus_known, allowed);
    CAPTURE_APP_SCOPE_STATE.store(
        capture_app_scope_state_code(state),
        std::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn record_input_event() {
    INPUT_EVENTS_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn input_events_total() -> u64 {
    INPUT_EVENTS_TOTAL.load(std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn set_terminal_apps_only(enabled: bool) {
    TERMINAL_APPS_ONLY.store(enabled, std::sync::atomic::Ordering::Relaxed);
    set_capture_app_scope_state(enabled, false, false);
}

pub(crate) fn set_raw_capture_fallback(enabled: bool, app_count: usize) {
    let scope =
        app_count.min(RAW_CAPTURE_COUNT_MASK) | if enabled { RAW_CAPTURE_ENABLED_BIT } else { 0 };
    RAW_CAPTURE_SCOPE.store(scope, std::sync::atomic::Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments)]
pub fn daemon_status_body_with_runtime_capabilities_and_config_state(
    source: &str,
    backend: &str,
    state: &str,
    paused: bool,
    config_path: &Path,
    config_state: &str,
    backend_mode: &str,
    metrics: CommandMetrics,
    latency: LatencySnapshot,
    capture: InputSourceCapabilities,
    injection: InjectorCapabilities,
    window_tracker_connected: bool,
    window_identity_exact: bool,
) -> StatusBody {
    let config_path = escape_status_value(&config_path.display().to_string());
    let matcher_latency = crate::latency::matcher_snapshot();
    let injection_latency_profiles = crate::latency::profiles_json();
    let usage_metrics = crate::usage::metrics_snapshot();
    let clipboard_metrics = crate::clipboard::metrics_snapshot();
    let guard = crate::assurance::GuardState::global();
    let compatibility_mode = guard.mode();
    let focus_check = guard.focus();
    // A route that cannot replace at all (including no connected route)
    // makes no insertion promise.
    let connected =
        injection.replacement_guarantee != wayexpand_core::ReplacementGuarantee::Unsupported;
    let assurance = crate::assurance::Assurance::of(
        connected.then_some(&injection),
        compatibility_mode,
        focus_check,
    );
    let replacement_admitted = replacement_is_admitted(&injection, compatibility_mode, focus_check);
    let focus_assurance = match assurance {
        crate::assurance::Assurance::Verified => "protocol",
        crate::assurance::Assurance::Unavailable => "none",
        crate::assurance::Assurance::FocusTrackedBestEffort
        | crate::assurance::Assurance::Unverified
        | crate::assurance::Assurance::Blocked => guard.focus_assurance(),
    };
    let output_health = crate::output_health::snapshot();
    let window_tracker_diagnostics =
        crate::backend_lifecycle::window_tracker_diagnostics_snapshot();
    let counters = crate::counters::snapshot();
    let command_cleanup = wayexpand_process_supervisor::cleanup_health();
    let connect_workers = worker_stats();
    let sandbox_security =
        serde_json::to_string(&wayexpand_process_supervisor::effective_security_snapshot())
            .unwrap_or_else(|_| "{}".to_owned());
    let route_id = wayexpand_backend_selection::route_id_for(source, backend);
    let terminal_apps_only = TERMINAL_APPS_ONLY.load(std::sync::atomic::Ordering::Relaxed);
    let raw_capture_scope = RAW_CAPTURE_SCOPE.load(std::sync::atomic::Ordering::Relaxed);
    let raw_capture_fallback_enabled = raw_capture_scope & RAW_CAPTURE_ENABLED_BIT != 0;
    let raw_capture_app_count = raw_capture_scope & RAW_CAPTURE_COUNT_MASK;
    let capture_app_scope_state = capture_app_scope_state_name(
        CAPTURE_APP_SCOPE_STATE.load(std::sync::atomic::Ordering::Relaxed),
    );
    let config_diagnostics = crate::reload::diagnostics_snapshot();
    let diagnostic_totals = crate::failure_trace::totals();
    let queue_latency = crate::latency::queue_snapshot();
    let connection_lifecycle = crate::counters::lifecycle_snapshot();
    let active_config_path = escape_status_value(&config_diagnostics.active_path);
    let validation_errors = escape_status_value(&config_diagnostics.validation_errors);
    let last_reload_failure =
        escape_status_value(if config_diagnostics.last_reload_failure.is_empty() {
            "none"
        } else {
            &config_diagnostics.last_reload_failure
        });
    let capture_layout_state = if source == "evdev" && !capture.layout_aware {
        "unavailable"
    } else if capture.layout_aware {
        "tracked"
    } else {
        "not-applicable"
    };
    let capture_available = state == "connected" && (source != "evdev" || capture.layout_aware);
    let injection_available =
        injection.replacement_guarantee != wayexpand_core::ReplacementGuarantee::Unsupported;
    let route_available = capture_available && injection_available;
    let matching_unavailable_reason = matching_unavailable_reason(
        source,
        state,
        capture.layout_aware,
        injection_available,
        replacement_admitted,
        paused,
        config_state,
    );
    let matching_available = matching_unavailable_reason == "none";
    let (window_context_available, window_app_id_available) = window_context_state();
    let mut body = format!(
                "source={source}\nbackend={backend}\nroute_id={route_id}\nbackend_mode={backend_mode}\nstatus_schema={CONTROL_STATUS_SCHEMA}\ndaemon_commit={}\ndaemon_version={}\nstate={state}\nroute_available={route_available}\nmatching_available={matching_available}\nmatching_unavailable_reason={matching_unavailable_reason}\npaused={}\nconfig={}\nconfig_active_path={}\nconfig_state={}\nconfig_generation={}\nconfig_loaded_at_unix={}\nconfig_last_successful_reload_unix={}\nconfig_enabled_expansions={}\nconfig_disabled_expansions={}\nconfig_generate_only_expansions={}\nconfig_last_reload_failure={}\nconfig_validation_error_count={}\nconfig_validation_errors={}\ncapture_sensitive_focus={}\ncapture_exclusive={}\ncapture_reliable_key_state={}\ncapture_key_passthrough={}\ncapture_composition_aware={}\ncapture_local_compose_aware={}\ncapture_layout_aware={}\ncapture_layout_state={capture_layout_state}\nwindow_tracker_connected={}\nwindow_tracker_state={}\nwindow_tracker_reconnect_attempts_total={}\nwindow_tracker_disconnects_total={}\nwindow_tracker_last_failure_reason={}\nwindow_tracker_last_connected_unix={}\nwindow_tracker_reconnect_backoff_ms={}\nwindow_identity_exact={}\ninject_atomic_replace={}\ninject_full_unicode={}\ninject_cursor_reposition={}\ninject_key_passthrough={}\ninject_insertion_mode={}\ninject_max_text_chars={}\ninject_expected_throughput_chars_per_sec={}\ncommand_worker_threads={}\nhotkey_worker_threads={}\ncommand_queue_depth={}\ncommand_in_flight={}\nexpansion_command_queue_depth={}\nexpansion_command_in_flight={}\nhotkey_queue_depth={}\nhotkey_in_flight={}\ncommand_queue_rejected_total={}\ncommand_timeout_total={}\ncommand_failure_total={}\ninjection_latency_sample_count={}\ninjection_latency_window_count={}\ninjection_latency_mean_us={}\ninjection_latency_p50_us={}\ninjection_latency_p95_us={}\ninjection_latency_p99_us={}\ninjection_latency_dropped_total={}\ninjection_latency_profile_dropped_total={}\ninjection_latency_profiles={}\nmatcher_latency_sample_count={}\nmatcher_latency_window_count={}\nmatcher_latency_dropped_total={}\nmatcher_latency_p50_us={}\nmatcher_latency_p95_us={}\nmatcher_latency_p99_us={}\nusage_flush_max_duration_us={}\nusage_flush_failures_total={}\nusage_worker_failures_total={}\nusage_queue_rejected_total={}\nusage_events_recorded_total={}\nusage_events_persisted_total={}\nusage_events_dropped_total={}\nclipboard_worker_threads={}\nclipboard_reads_in_flight={}\nclipboard_reads_total={}\nclipboard_read_failures_total={}\nclipboard_deadline_misses_total={}\nclipboard_read_max_duration_us={}\noutput_health={}\noutput_uncertain_total={}\noutput_partial_total={}\npartial_write_total={}\noutput_retiring_workers={}\noutput_stalled_ms={}\nbackend_shutdown_workers_active={}\nbackend_shutdown_workers_started_total={}\nbackend_shutdown_workers_acknowledged_total={}\nbackend_shutdown_workers_timed_out_total={}\nbackend_shutdown_workers_failed_total={}\ninsertion_assurance={}\nreplacement_assurance={}\ncompatibility_mode={}\nfocus_assurance={}\noutput_refused_total={}\noutput_last_refusal={}\noutput_unsupported_total={}\noutput_last_unsupported={}\nrecent_expansion_failures={}\nrecent_expansion_outcomes={}\nsuppressed_expansions_total={}\noutput_reconnect_total={}\ninput_reconnect_total={}\ninput_connection_failures_total={}\noutput_connection_failures_total={}\ninput_overflow_total={}\nevdev_quarantined_events_total={}\ncontrol_rejected_total={}\nmemory_rss_kib={}\nmemory_high_water_kib={}\ncommand_containment={}\ncommand_containment_mode={}\ncommand_containment_self_test={}\ncommand_cleanup_pending_jobs={}\ncommand_cleanup_failed_attempts={}\ncommand_cleanup_degraded={}\nconnect_workers_active={}\nconnect_workers_started_total={}\nconnect_workers_completed_total={}\nconnect_workers_cancelled_total={}\nconnect_workers_rejected_total={}\nsandbox_security={sandbox_security}\nterminal_apps_only={terminal_apps_only}\ncapture_app_scope_state={capture_app_scope_state}\nraw_capture_fallback_enabled={raw_capture_fallback_enabled}\nraw_capture_app_count={raw_capture_app_count}\ninput_events_total={}\nmatcher_evaluations_total={}\nmatches_found_total={}\nexpansion_attempts_total={}\nexpansions_applied_total={}\nexpansions_rejected_total={}\ndiagnostic_events_dropped_total={}\ninput_connection_state={}\noutput_connection_state={}\ninput_last_connected_unix={}\noutput_last_connected_unix={}\ninput_last_disconnect_reason={}\noutput_last_disconnect_reason={}\ninput_reconnect_attempts={}\noutput_reconnect_attempts={}\ninput_reconnect_backoff_ms={}\noutput_reconnect_backoff_ms={}\nlast_successful_insertion_unix={}\noutput_queue_latency_sample_count={}\noutput_queue_latency_window_count={}\noutput_queue_latency_p50_us={}\noutput_queue_latency_p95_us={}\noutput_queue_latency_p99_us={}\noutput_queue_latency_dropped_total={}",
        build_info::COMMIT,
        build_info::VERSION,
        paused,
        config_path,
        active_config_path,
        config_state,
        config_diagnostics.generation,
        config_diagnostics.loaded_at_unix,
        config_diagnostics.last_successful_reload_unix,
        config_diagnostics.enabled_expansions,
        config_diagnostics.disabled_expansions,
        config_diagnostics.generate_only_expansions,
        last_reload_failure,
        if config_diagnostics.validation_errors.is_empty() { 0 } else { 1 },
        validation_errors,
        capture.sensitive_focus,
        capture.exclusive_capture,
        capture.reliable_key_state,
        capture.key_passthrough,
        capture.composition_aware,
        capture.local_compose_aware,
        capture.layout_aware,
        window_tracker_connected,
        window_tracker_diagnostics.state,
        window_tracker_diagnostics.reconnect_attempts_total,
        window_tracker_diagnostics.disconnects_total,
        window_tracker_diagnostics.last_failure_reason,
        window_tracker_diagnostics.last_connected_unix,
        window_tracker_diagnostics.reconnect_backoff_ms,
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
        metrics.command_worker_threads,
        metrics.hotkey_worker_threads,
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
        latency.mean_us,
        latency.p50_us,
        latency.p95_us,
        latency.p99_us,
        crate::latency::output_dropped_samples(),
        crate::latency::profile_dropped_samples(),
        injection_latency_profiles,
        matcher_latency.sample_count,
        matcher_latency.window_count,
        crate::latency::matcher_dropped_samples(),
        matcher_latency.p50_us,
        matcher_latency.p95_us,
        matcher_latency.p99_us,
        usage_metrics.max_flush_duration_us,
        usage_metrics.flush_failures_total,
        usage_metrics.worker_failures_total,
        usage_metrics.queue_rejected_total,
        usage_metrics.events_recorded_total,
        usage_metrics.events_persisted_total,
        usage_metrics.events_dropped_total,
        clipboard_metrics.worker_threads,
        clipboard_metrics.reads_in_flight,
        clipboard_metrics.reads_total,
        clipboard_metrics.read_failures_total,
        clipboard_metrics.deadline_misses_total,
        clipboard_metrics.max_read_duration_us,
        output_health.health(),
        output_health.uncertain_total,
        output_health.partial_total,
        output_health.partial_total,
        output_health.retiring_workers,
        output_health.stalled_ms,
        output_health.shutdown_workers_active,
        output_health.shutdown_workers_started_total,
        output_health.shutdown_workers_acknowledged_total,
        output_health.shutdown_workers_timed_out_total,
        output_health.shutdown_workers_failed_total,
        assurance.legacy_as_str(),
        assurance.as_str(),
        guard.mode().as_str(),
        focus_assurance,
        guard.refused_total(),
        guard.last_refusal().map_or("none", crate::assurance::Refusal::as_str),
        counters.output_unsupported_total,
        counters.output_last_unsupported,
        crate::failure_trace::recent_json(),
        crate::failure_trace::recent_outcomes_json(),
        crate::failure_trace::suppressed_expansions_total(),
        counters.output_reconnect_total,
        counters.input_reconnect_total,
        counters.input_connection_failures_total,
        counters.output_connection_failures_total,
        counters.input_overflow_total,
        counters.evdev_quarantined_events_total,
        counters.control_rejected_total,
        counters.memory_rss_kib,
        counters.memory_high_water_kib,
        wayexpand_core::command_containment_mode(),
        wayexpand_core::command_containment_policy_mode(),
        wayexpand_process_supervisor::containment_self_test_result(),
        command_cleanup.pending_jobs,
        command_cleanup.failed_attempts,
        command_cleanup.degraded,
        connect_workers.active,
        connect_workers.started_total,
        connect_workers.completed_total,
        connect_workers.cancelled_total,
        connect_workers.rejected_total,
        input_events_total(),
        diagnostic_totals.matcher_evaluations,
        diagnostic_totals.matches_found,
        diagnostic_totals.expansion_attempts,
        diagnostic_totals.expansions_applied,
        diagnostic_totals.expansions_rejected,
        diagnostic_totals.events_dropped,
        connection_lifecycle.input_state,
        connection_lifecycle.output_state,
        connection_lifecycle.input_last_connected_unix,
        connection_lifecycle.output_last_connected_unix,
        connection_lifecycle.input_disconnect_reason,
        connection_lifecycle.output_disconnect_reason,
        connection_lifecycle.input_attempts,
        connection_lifecycle.output_attempts,
        connection_lifecycle.input_backoff_ms,
        connection_lifecycle.output_backoff_ms,
        crate::failure_trace::last_successful_insertion_unix(),
        queue_latency.sample_count,
        queue_latency.window_count,
        queue_latency.p50_us,
        queue_latency.p95_us,
        queue_latency.p99_us,
        crate::latency::queue_dropped_samples(),
    );
    body.push_str(&format!(
        "\nwindow_tracker_updates_total={}\nwindow_context_available={}\nwindow_app_id_available={}\n",
        window_tracker_updates_total(),
        window_context_available,
        window_app_id_available,
    ));
    StatusBody(body)
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

#[cfg(test)]
mod matching_reason_tests {
    use super::{
        capture_app_scope_state, matching_unavailable_reason, replacement_is_admitted,
        window_context_state_code, WINDOW_APP_ID_AVAILABLE_BIT, WINDOW_CONTEXT_AVAILABLE_BIT,
    };
    use wayexpand_core::{
        CompatibilityMode, FocusCheck, InjectorCapabilities, ReplacementAdmission,
        ReplacementGuarantee,
    };

    #[test]
    fn matching_availability_reason_identifies_each_failed_prerequisite() {
        let available =
            matching_unavailable_reason("input-method", "connected", true, true, true, false, "ok");
        assert_eq!(available, "none");
        assert_eq!(
            matching_unavailable_reason("evdev", "connected", false, true, true, false, "ok"),
            "keyboard_layout_unavailable"
        );
        assert_eq!(
            matching_unavailable_reason(
                "input-method",
                "reconnecting",
                true,
                true,
                true,
                false,
                "ok"
            ),
            "capture_disconnected"
        );
        assert_eq!(
            matching_unavailable_reason(
                "input-method",
                "connected",
                true,
                false,
                false,
                false,
                "ok"
            ),
            "replacement_route_unavailable"
        );
        assert_eq!(
            matching_unavailable_reason("input-method", "connected", true, true, true, true, "ok"),
            "paused"
        );
        assert_eq!(
            matching_unavailable_reason("evdev", "connected", true, true, false, false, "ok"),
            "replacement_not_admitted",
            "connected capture and an injector are insufficient when the safety guard refuses replacement"
        );
        let non_atomic = InjectorCapabilities {
            replacement_guarantee: ReplacementGuarantee::BestEffort,
            ..InjectorCapabilities::default()
        };
        let admitted =
            replacement_is_admitted(&non_atomic, CompatibilityMode::Off, FocusCheck::Unchanged);
        assert!(!admitted);
        assert_eq!(
            matching_unavailable_reason("evdev", "connected", true, true, admitted, false, "ok"),
            "replacement_not_admitted"
        );
        let atomic = InjectorCapabilities {
            atomic_replace: true,
            replacement_guarantee: ReplacementGuarantee::Atomic,
            ..InjectorCapabilities::default()
        };
        assert!(
            replacement_is_admitted(&atomic, CompatibilityMode::Off, FocusCheck::Unconfirmed),
            "atomic replacement remains admitted with compatibility mode off"
        );
        assert_eq!(
            non_atomic.replacement_admission(CompatibilityMode::TrackedFocus),
            ReplacementAdmission::ExplicitCompatibility
        );
        assert!(replacement_is_admitted(
            &non_atomic,
            CompatibilityMode::TrackedFocus,
            FocusCheck::Unchanged
        ));
        assert!(!replacement_is_admitted(
            &non_atomic,
            CompatibilityMode::TrackedFocus,
            FocusCheck::Unconfirmed
        ));
        assert_eq!(
            matching_unavailable_reason(
                "input-method",
                "connected",
                true,
                true,
                true,
                false,
                "reload-rejected"
            ),
            "configuration_unhealthy"
        );
    }

    #[test]
    fn focus_diagnostics_distinguish_missing_context_from_missing_app_id() {
        assert_eq!(window_context_state_code(None), 0);

        let title_only = wayexpand_core::WindowContext {
            app_id: None,
            title: Some("private title".into()),
            instance_id: Some("window-1".into()),
        };
        let title_only_state = window_context_state_code(Some(&title_only));
        assert_ne!(title_only_state & WINDOW_CONTEXT_AVAILABLE_BIT, 0);
        assert_eq!(title_only_state & WINDOW_APP_ID_AVAILABLE_BIT, 0);

        let app_identified = wayexpand_core::WindowContext {
            app_id: Some("org.example.App".into()),
            ..title_only
        };
        let app_identified_state = window_context_state_code(Some(&app_identified));
        assert_ne!(app_identified_state & WINDOW_CONTEXT_AVAILABLE_BIT, 0);
        assert_ne!(app_identified_state & WINDOW_APP_ID_AVAILABLE_BIT, 0);
    }

    #[test]
    fn capture_app_scope_state_does_not_expose_identity_and_fails_closed() {
        assert_eq!(capture_app_scope_state(false, false, false), "unrestricted");
        assert_eq!(capture_app_scope_state(true, false, true), "unknown");
        assert_eq!(capture_app_scope_state(true, true, true), "allowed");
        assert_eq!(
            capture_app_scope_state(true, true, false),
            "outside_allowlist"
        );
    }
}
