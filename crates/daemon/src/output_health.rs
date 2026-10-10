//! Process-wide output health for the status contract: transactions whose
//! outcome is unknown, and retired output workers that must stop before the
//! output route may reconnect.
//!
//! Both are safety decisions that otherwise surface only in the journal. An
//! uncertain transaction is never replayed, and a retired worker that is still
//! inside a compositor or portal call blocks reconnection indefinitely; status
//! consumers and `doctor` need to see either without reading logs.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use wayexpand_core::TransactionOutcome;

/// A retired worker still running after this long is reported as stalled
/// rather than recovering. It matches the base acknowledgement deadline:
/// a healthy worker stops well within the time it was given to answer.
pub const STALLED_AFTER: Duration = Duration::from_secs(5);

static UNCERTAIN_TOTAL: AtomicU64 = AtomicU64::new(0);
static RETIRING: Mutex<Retiring> = Mutex::new(Retiring {
    workers: 0,
    oldest: None,
});

struct Retiring {
    workers: usize,
    oldest: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub uncertain_total: u64,
    pub retiring_workers: usize,
    pub stalled_ms: u64,
}

impl Snapshot {
    /// `ok`, `recovering` (a retired worker is stopping), or `stalled` (it
    /// has outlived [`STALLED_AFTER`] and blocks output reconnection).
    pub fn health(&self) -> &'static str {
        if self.retiring_workers == 0 {
            "ok"
        } else if u128::from(self.stalled_ms) >= STALLED_AFTER.as_millis() {
            "stalled"
        } else {
            "recovering"
        }
    }
}

/// Count a transaction whose effect on the target text is unknown.
pub fn note_outcome(outcome: &TransactionOutcome) {
    if is_uncertain(outcome) {
        UNCERTAIN_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
}

fn is_uncertain(outcome: &TransactionOutcome) -> bool {
    matches!(outcome, TransactionOutcome::UnknownPartialFailure { .. })
}

/// Record the retired workers still running and when the oldest retired.
pub fn set_retiring(workers: usize, oldest: Option<Instant>) {
    let mut retiring = RETIRING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    retiring.workers = workers;
    retiring.oldest = oldest.filter(|_| workers > 0);
}

pub fn snapshot() -> Snapshot {
    let retiring = RETIRING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    snapshot_of(UNCERTAIN_TOTAL.load(Ordering::Relaxed), &retiring)
}

fn snapshot_of(uncertain_total: u64, retiring: &Retiring) -> Snapshot {
    Snapshot {
        uncertain_total,
        retiring_workers: retiring.workers,
        stalled_ms: retiring.oldest.map_or(0, |since| {
            u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_distinguishes_recovering_from_stalled_workers() {
        let snapshot = |retiring_workers, stalled_ms| Snapshot {
            uncertain_total: 0,
            retiring_workers,
            stalled_ms,
        };
        assert_eq!(snapshot(0, 0).health(), "ok");
        assert_eq!(snapshot(1, 10).health(), "recovering");
        assert_eq!(snapshot(1, 5_000).health(), "stalled");
    }

    #[test]
    fn only_unknown_partial_failures_are_counted_as_uncertain() {
        let error = || wayexpand_core::InjectorError {
            backend: "test",
            message: "failed".into(),
            retryable: true,
        };
        assert!(!is_uncertain(&TransactionOutcome::Applied));
        assert!(!is_uncertain(&TransactionOutcome::NotApplied {
            source: error()
        }));
        assert!(!is_uncertain(
            &TransactionOutcome::AppliedWithCursorPositionFailure { source: error() }
        ));
        assert!(is_uncertain(&TransactionOutcome::UnknownPartialFailure {
            source: error()
        }));
    }

    #[test]
    fn retiring_workers_report_their_age() {
        let retiring = Retiring {
            workers: 1,
            oldest: Some(Instant::now() - Duration::from_millis(50)),
        };
        let snapshot = snapshot_of(3, &retiring);
        assert_eq!(snapshot.uncertain_total, 3);
        assert_eq!(snapshot.retiring_workers, 1);
        assert!(snapshot.stalled_ms >= 50);
        let idle = Retiring {
            workers: 0,
            oldest: None,
        };
        assert_eq!(snapshot_of(0, &idle).stalled_ms, 0);
    }
}
