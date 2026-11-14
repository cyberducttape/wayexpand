//! Process-lifetime reliability counters and memory use for the status body.
//!
//! These are observations for operators (reconnect churn, dropped input,
//! control-socket saturation, memory growth over a soak), not inputs to any
//! safety decision.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use wayexpand_core::RouteLimit;

static OUTPUT_RECONNECTS: AtomicU64 = AtomicU64::new(0);
static INPUT_RECONNECTS: AtomicU64 = AtomicU64::new(0);
static EVDEV_QUARANTINED_EVENTS: AtomicU64 = AtomicU64::new(0);
static CONTROL_REJECTED: AtomicU64 = AtomicU64::new(0);
static OUTPUT_UNSUPPORTED: AtomicU64 = AtomicU64::new(0);
static LAST_UNSUPPORTED: AtomicU8 = AtomicU8::new(0);
static INPUT_CONNECTION_FAILURES: AtomicU64 = AtomicU64::new(0);
static OUTPUT_CONNECTION_FAILURES: AtomicU64 = AtomicU64::new(0);
static INPUT_LAST_CONNECTED_UNIX: AtomicU64 = AtomicU64::new(0);
static OUTPUT_LAST_CONNECTED_UNIX: AtomicU64 = AtomicU64::new(0);
static INPUT_BACKOFF_MS: AtomicU64 = AtomicU64::new(0);
static OUTPUT_BACKOFF_MS: AtomicU64 = AtomicU64::new(0);
static INPUT_CONNECTION_STATE: AtomicU8 = AtomicU8::new(0);
static OUTPUT_CONNECTION_STATE: AtomicU8 = AtomicU8::new(0);
static INPUT_DISCONNECT_REASON: AtomicU8 = AtomicU8::new(0);
static OUTPUT_DISCONNECT_REASON: AtomicU8 = AtomicU8::new(0);
static INPUT_RECONNECT_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static OUTPUT_RECONNECT_ATTEMPTS: AtomicU64 = AtomicU64::new(0);

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| value.as_secs())
}

fn connection_state(code: u8) -> &'static str {
    match code {
        1 => "connected",
        2 => "disconnected",
        3 => "reconnecting",
        4 => "authorization_required",
        5 => "backoff",
        _ => "unknown",
    }
}

fn disconnect_reason(code: u8) -> &'static str {
    match code {
        1 => "transport_lost",
        2 => "authorization_revoked",
        3 => "connect_failed",
        4 => "output_transaction_failed",
        _ => "none",
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ConnectionLifecycleSnapshot {
    pub input_state: &'static str,
    pub output_state: &'static str,
    pub input_last_connected_unix: u64,
    pub output_last_connected_unix: u64,
    pub input_disconnect_reason: &'static str,
    pub output_disconnect_reason: &'static str,
    pub input_attempts: u64,
    pub output_attempts: u64,
    pub input_backoff_ms: u64,
    pub output_backoff_ms: u64,
}

pub(crate) fn lifecycle_snapshot() -> ConnectionLifecycleSnapshot {
    ConnectionLifecycleSnapshot {
        input_state: connection_state(INPUT_CONNECTION_STATE.load(Ordering::Relaxed)),
        output_state: connection_state(OUTPUT_CONNECTION_STATE.load(Ordering::Relaxed)),
        input_last_connected_unix: INPUT_LAST_CONNECTED_UNIX.load(Ordering::Relaxed),
        output_last_connected_unix: OUTPUT_LAST_CONNECTED_UNIX.load(Ordering::Relaxed),
        input_disconnect_reason: disconnect_reason(INPUT_DISCONNECT_REASON.load(Ordering::Relaxed)),
        output_disconnect_reason: disconnect_reason(
            OUTPUT_DISCONNECT_REASON.load(Ordering::Relaxed),
        ),
        input_attempts: INPUT_RECONNECT_ATTEMPTS.load(Ordering::Relaxed),
        output_attempts: OUTPUT_RECONNECT_ATTEMPTS.load(Ordering::Relaxed),
        input_backoff_ms: INPUT_BACKOFF_MS.load(Ordering::Relaxed),
        output_backoff_ms: OUTPUT_BACKOFF_MS.load(Ordering::Relaxed),
    }
}

pub(crate) fn mark_initial_connections(input: bool, output: bool) {
    let now = now_unix();
    if input {
        INPUT_CONNECTION_STATE.store(1, Ordering::Relaxed);
        INPUT_LAST_CONNECTED_UNIX.store(now, Ordering::Relaxed);
    }
    if output {
        OUTPUT_CONNECTION_STATE.store(1, Ordering::Relaxed);
        OUTPUT_LAST_CONNECTED_UNIX.store(now, Ordering::Relaxed);
    }
}

pub(crate) fn note_input_disconnected(reason: &'static str) {
    INPUT_CONNECTION_STATE.store(2, Ordering::Relaxed);
    INPUT_DISCONNECT_REASON.store(reason_code(reason), Ordering::Relaxed);
}

pub(crate) fn note_input_reconnecting() {
    INPUT_CONNECTION_STATE.store(3, Ordering::Relaxed);
    INPUT_RECONNECT_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_output_disconnected(reason: &'static str) {
    OUTPUT_CONNECTION_STATE.store(2, Ordering::Relaxed);
    OUTPUT_DISCONNECT_REASON.store(reason_code(reason), Ordering::Relaxed);
}

pub(crate) fn note_output_reconnecting() {
    OUTPUT_CONNECTION_STATE.store(3, Ordering::Relaxed);
    OUTPUT_RECONNECT_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
}

fn reason_code(reason: &str) -> u8 {
    match reason {
        "transport_lost" => 1,
        "authorization_revoked" => 2,
        "connect_failed" => 3,
        "output_transaction_failed" => 4,
        _ => 0,
    }
}

pub(crate) fn note_input_backoff(delay: std::time::Duration) {
    INPUT_CONNECTION_STATE.store(5, Ordering::Relaxed);
    INPUT_BACKOFF_MS.store(
        delay.as_millis().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
}

pub(crate) fn note_output_backoff(delay: std::time::Duration) {
    OUTPUT_CONNECTION_STATE.store(5, Ordering::Relaxed);
    OUTPUT_BACKOFF_MS.store(
        delay.as_millis().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
}

pub(crate) fn note_input_connection_failure() {
    INPUT_CONNECTION_FAILURES.fetch_add(1, Ordering::Relaxed);
    INPUT_CONNECTION_STATE.store(5, Ordering::Relaxed);
    INPUT_DISCONNECT_REASON.store(3, Ordering::Relaxed);
}

pub(crate) fn note_output_connection_failure() {
    OUTPUT_CONNECTION_FAILURES.fetch_add(1, Ordering::Relaxed);
    OUTPUT_CONNECTION_STATE.store(5, Ordering::Relaxed);
    OUTPUT_DISCONNECT_REASON.store(3, Ordering::Relaxed);
}

pub(crate) fn note_output_reconnect() {
    OUTPUT_RECONNECTS.fetch_add(1, Ordering::Relaxed);
    OUTPUT_CONNECTION_STATE.store(1, Ordering::Relaxed);
    OUTPUT_LAST_CONNECTED_UNIX.store(now_unix(), Ordering::Relaxed);
    OUTPUT_BACKOFF_MS.store(0, Ordering::Relaxed);
}

pub(crate) fn note_input_reconnect() {
    INPUT_RECONNECTS.fetch_add(1, Ordering::Relaxed);
    INPUT_CONNECTION_STATE.store(1, Ordering::Relaxed);
    INPUT_LAST_CONNECTED_UNIX.store(now_unix(), Ordering::Relaxed);
    INPUT_BACKOFF_MS.store(0, Ordering::Relaxed);
}

pub(crate) fn note_evdev_quarantined_event() {
    EVDEV_QUARANTINED_EVENTS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_control_rejected() {
    CONTROL_REJECTED.fetch_add(1, Ordering::Relaxed);
}

/// An expansion the output route could not type exactly (a character
/// outside its set, too long, or a cursor move it cannot make). It was
/// refused before anything was typed.
pub(crate) fn note_output_unsupported(limit: RouteLimit) {
    OUTPUT_UNSUPPORTED.fetch_add(1, Ordering::Relaxed);
    let code = match limit {
        RouteLimit::Unicode { .. } => 1,
        RouteLimit::TooLong { .. } => 2,
        RouteLimit::CursorMove => 3,
    };
    LAST_UNSUPPORTED.store(code, Ordering::Relaxed);
}

/// Record a backend-specific preflight refusal without retaining any part of
/// the rejected text. These labels are stable status values and deliberately
/// contain no code point or snippet content.
pub(crate) fn note_output_unsupported_reason(reason: &'static str) {
    OUTPUT_UNSUPPORTED.fetch_add(1, Ordering::Relaxed);
    let code = match reason {
        "unicode-keymap" => 4,
        "fallback-too-slow" => 5,
        "layout-mismatch" => 6,
        "active-layout-unavailable" => 7,
        "keyboard-state-busy" => 8,
        "paced-interactive-limit" => 9,
        _ => return,
    };
    LAST_UNSUPPORTED.store(code, Ordering::Relaxed);
}

fn last_unsupported_name(code: u8) -> &'static str {
    match code {
        1 => "unicode",
        2 => "too-long",
        3 => "cursor",
        4 => "unicode-keymap",
        5 => "fallback-too-slow",
        6 => "layout-mismatch",
        7 => "active-layout-unavailable",
        8 => "keyboard-state-busy",
        9 => "paced-interactive-limit",
        _ => "none",
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) output_unsupported_total: u64,
    pub(crate) output_last_unsupported: &'static str,
    pub(crate) output_reconnect_total: u64,
    pub(crate) input_reconnect_total: u64,
    pub(crate) input_connection_failures_total: u64,
    pub(crate) output_connection_failures_total: u64,
    pub(crate) evdev_quarantined_events_total: u64,
    pub(crate) input_overflow_total: u64,
    pub(crate) control_rejected_total: u64,
    pub(crate) memory_rss_kib: u64,
    pub(crate) memory_high_water_kib: u64,
}

pub(crate) fn snapshot() -> Snapshot {
    let (memory_rss_kib, memory_high_water_kib) = std::fs::read_to_string("/proc/self/status")
        .map(|status| memory_kib(&status))
        .unwrap_or_default();
    Snapshot {
        output_unsupported_total: OUTPUT_UNSUPPORTED.load(Ordering::Relaxed),
        output_last_unsupported: last_unsupported_name(LAST_UNSUPPORTED.load(Ordering::Relaxed)),
        output_reconnect_total: OUTPUT_RECONNECTS.load(Ordering::Relaxed),
        input_reconnect_total: INPUT_RECONNECTS.load(Ordering::Relaxed),
        input_connection_failures_total: INPUT_CONNECTION_FAILURES.load(Ordering::Relaxed),
        output_connection_failures_total: OUTPUT_CONNECTION_FAILURES.load(Ordering::Relaxed),
        evdev_quarantined_events_total: EVDEV_QUARANTINED_EVENTS.load(Ordering::Relaxed),
        input_overflow_total: wayexpand_backend_evdev::input_queue_overflow_total(),
        control_rejected_total: CONTROL_REJECTED.load(Ordering::Relaxed),
        memory_rss_kib,
        memory_high_water_kib,
    }
}

/// Resident set size and its peak (`VmRSS`, `VmHWM`) from
/// `/proc/self/status`, in KiB; 0 for a field that is missing.
fn memory_kib(status: &str) -> (u64, u64) {
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.trim().strip_suffix("kB"))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0)
    };
    (field("VmRSS:"), field("VmHWM:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_fields_are_read_from_proc_status() {
        let status = "Name:\twayexpand-daemon\nVmHWM:\t   10240 kB\nVmRSS:\t    8192 kB\n";
        assert_eq!(memory_kib(status), (8192, 10240));
        assert_eq!(memory_kib("Name:\tx\n"), (0, 0));
    }

    #[test]
    fn this_process_reports_its_memory() {
        let snapshot = snapshot();
        assert!(snapshot.memory_rss_kib > 0);
        assert!(snapshot.memory_high_water_kib >= snapshot.memory_rss_kib);
    }
}
