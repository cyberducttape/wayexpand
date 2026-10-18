//! Process-lifetime reliability counters and memory use for the status body.
//!
//! These are observations for operators (reconnect churn, dropped input,
//! control-socket saturation, memory growth over a soak), not inputs to any
//! safety decision.

use std::sync::atomic::{AtomicU64, Ordering};

static OUTPUT_RECONNECTS: AtomicU64 = AtomicU64::new(0);
static INPUT_RECONNECTS: AtomicU64 = AtomicU64::new(0);
static CONTROL_REJECTED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn note_output_reconnect() {
    OUTPUT_RECONNECTS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_input_reconnect() {
    INPUT_RECONNECTS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_control_rejected() {
    CONTROL_REJECTED.fetch_add(1, Ordering::Relaxed);
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) output_reconnect_total: u64,
    pub(crate) input_reconnect_total: u64,
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
        output_reconnect_total: OUTPUT_RECONNECTS.load(Ordering::Relaxed),
        input_reconnect_total: INPUT_RECONNECTS.load(Ordering::Relaxed),
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
