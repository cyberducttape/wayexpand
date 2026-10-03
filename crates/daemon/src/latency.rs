//! Bounded rolling measurements of output-backend apply time.
//!
//! This measures the completed output-backend transaction, including the
//! completion acknowledgement from the serialized output actor, not
//! keystroke-to-paint latency. Keeping that distinction explicit prevents
//! backend timing from being presented as a full desktop end-to-end SLO.

use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::Instant,
};
use wayexpand_core::{ExpansionEngine, ExpansionResult, TextInjector, TransactionOutcome};

const WINDOW_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub sample_count: u64,
    pub window_count: usize,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
}

#[derive(Default)]
struct Window {
    samples_ns: VecDeque<u64>,
    total: u64,
    cached: Snapshot,
    dirty: bool,
}

impl Window {
    fn record(&mut self, elapsed_ns: u64) {
        if self.samples_ns.len() == WINDOW_CAPACITY {
            self.samples_ns.pop_front();
        }
        self.samples_ns.push_back(elapsed_ns);
        self.total = self.total.saturating_add(1);
        self.dirty = true;
    }

    fn snapshot(&mut self) -> Snapshot {
        if !self.dirty {
            return self.cached;
        }
        let mut sorted = self.samples_ns.iter().copied().collect::<Vec<_>>();
        sorted.sort_unstable();
        self.cached = Snapshot {
            sample_count: self.total,
            window_count: sorted.len(),
            p50_us: percentile_us(&sorted, 50),
            p95_us: percentile_us(&sorted, 95),
            p99_us: percentile_us(&sorted, 99),
        };
        self.dirty = false;
        self.cached
    }
}

fn percentile_us(sorted_ns: &[u64], percentile: usize) -> u64 {
    if sorted_ns.is_empty() {
        return 0;
    }
    // Nearest-rank percentile, rounded up to retain sub-millisecond detail.
    let rank = (percentile * sorted_ns.len()).div_ceil(100).max(1);
    sorted_ns[rank - 1].div_ceil(1_000)
}

fn window() -> &'static Mutex<Window> {
    static WINDOW: OnceLock<Mutex<Window>> = OnceLock::new();
    WINDOW.get_or_init(|| Mutex::new(Window::default()))
}

fn record(elapsed_ns: u64) {
    let mut state = window()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.record(elapsed_ns);
}

pub fn snapshot() -> Snapshot {
    window()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .snapshot()
}

pub fn apply(injector: &mut dyn TextInjector, result: &ExpansionResult) -> TransactionOutcome {
    measure(|| ExpansionEngine::apply(injector, result))
}

fn measure<T>(operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let outcome = operation();
    record(started.elapsed().as_nanos().min(u64::MAX as u128) as u64);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_window_reports_nearest_rank_percentiles_and_total_count() {
        let mut window = Window::default();
        for value in 1..=100 {
            window.record(value * 1_000);
        }
        let snapshot = window.snapshot();
        assert_eq!(snapshot.sample_count, 100);
        assert_eq!(snapshot.window_count, 100);
        assert_eq!(snapshot.p50_us, 50);
        assert_eq!(snapshot.p95_us, 95);
        assert_eq!(snapshot.p99_us, 99);
    }

    #[test]
    fn rolling_window_is_bounded_and_retains_the_latest_samples() {
        let mut window = Window::default();
        for value in 1..=(WINDOW_CAPACITY as u64 + 1) {
            window.record(value * 1_000);
        }
        let snapshot = window.snapshot();
        assert_eq!(snapshot.sample_count, WINDOW_CAPACITY as u64 + 1);
        assert_eq!(snapshot.window_count, WINDOW_CAPACITY);
        assert_eq!(snapshot.p50_us, (WINDOW_CAPACITY as u64 / 2) + 1);
    }

    #[test]
    fn empty_window_has_zero_percentiles() {
        assert_eq!(Window::default().snapshot(), Snapshot::default());
    }

    #[test]
    fn measurements_are_recorded_for_successes_and_failures() {
        let before = snapshot().sample_count;
        assert_eq!(measure(|| Ok::<_, ()>(())), Ok(()));
        assert_eq!(
            measure(|| Err::<(), _>("injection failed")),
            Err("injection failed")
        );
        assert!(snapshot().sample_count >= before + 2);
    }
}
