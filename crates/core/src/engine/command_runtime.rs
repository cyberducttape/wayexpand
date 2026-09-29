//! Asynchronous command execution and hotkey action handling.
//!
//! Organizes worker threads for executing expansion commands and hotkey actions
//! with bounded queueing and timeout enforcement.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Atomic counters shared by the expansion and hotkey workers. Keeping this
/// state with the runtime module makes queue accounting independent of the
/// engine's matching and transaction state.
pub(super) struct CommandMetricsState {
    pub(super) queue_depth: AtomicUsize,
    pub(super) in_flight: AtomicUsize,
    pub(super) queue_rejected_total: AtomicU64,
    pub(super) timeout_total: AtomicU64,
    pub(super) failure_total: AtomicU64,
}

impl CommandMetricsState {
    pub(super) fn new() -> Self {
        Self {
            queue_depth: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            queue_rejected_total: AtomicU64::new(0),
            timeout_total: AtomicU64::new(0),
            failure_total: AtomicU64::new(0),
        }
    }

    pub(super) fn record_error(&self, timeout: bool) {
        if timeout {
            self.timeout_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failure_total.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(super) enum QueueSendError {
    Full,
    Disconnected,
}
