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

const OUTPUT_MODES: [&str; 5] = [
    "ei_text",
    "libei_keysym_fallback",
    "input_method_v2",
    "wlroots_virtual_keyboard",
    "other",
];
const OUTPUT_SIZE_BUCKETS: [&str; 3] = ["small", "medium", "large"];

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

#[derive(Default)]
struct ProfileWindows {
    windows: [[Window; 3]; 5],
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

fn matcher_window() -> &'static Mutex<Window> {
    static WINDOW: OnceLock<Mutex<Window>> = OnceLock::new();
    WINDOW.get_or_init(|| Mutex::new(Window::default()))
}

fn profile_windows() -> &'static Mutex<ProfileWindows> {
    static WINDOWS: OnceLock<Mutex<ProfileWindows>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(ProfileWindows::default()))
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

/// Snapshot of time spent processing input through the matcher and preparing
/// an expansion, excluding output injection and its serialized actor.
pub fn matcher_snapshot() -> Snapshot {
    matcher_window()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .snapshot()
}

pub fn apply(injector: &mut dyn TextInjector, result: &ExpansionResult) -> TransactionOutcome {
    let mode = output_mode(injector.status_detail());
    let size = output_size_bucket(result.insert.chars().count());
    measure(mode, size, || ExpansionEngine::apply(injector, result))
}

pub fn measure_matcher<T>(operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    let mut state = matcher_window()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.record(started.elapsed().as_nanos().min(u64::MAX as u128) as u64);
    result
}

fn measure<T>(mode: usize, size: usize, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let outcome = operation();
    let elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    record(elapsed_ns);
    let mut profiles = profile_windows()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    profiles.windows[mode][size].record(elapsed_ns);
    outcome
}

fn output_mode(detail: &str) -> usize {
    if detail.contains("keysym fallback") {
        1
    } else if detail.contains("ei_text") {
        0
    } else if detail.contains("input-method-v2") {
        2
    } else if detail.contains("wlroots virtual-keyboard") {
        3
    } else {
        4
    }
}

fn output_size_bucket(chars: usize) -> usize {
    match chars {
        0..=32 => 0,
        33..=256 => 1,
        _ => 2,
    }
}

/// Return bounded, machine-readable percentiles segmented by the negotiated
/// output mode and replacement size. Empty profiles are omitted so ordinary
/// deployments do not carry misleading zero-valued measurements.
pub fn profiles_json() -> String {
    let mut profiles = profile_windows()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut modes = serde_json::Map::new();
    for (mode_index, mode) in OUTPUT_MODES.iter().enumerate() {
        let mut sizes = serde_json::Map::new();
        for (size_index, size) in OUTPUT_SIZE_BUCKETS.iter().enumerate() {
            let snapshot = profiles.windows[mode_index][size_index].snapshot();
            if snapshot.window_count == 0 {
                continue;
            }
            sizes.insert(
                (*size).to_owned(),
                serde_json::json!({
                    "sample_count": snapshot.sample_count,
                    "window_count": snapshot.window_count,
                    "p50_us": snapshot.p50_us,
                    "p95_us": snapshot.p95_us,
                    "p99_us": snapshot.p99_us,
                }),
            );
        }
        if !sizes.is_empty() {
            modes.insert((*mode).to_owned(), serde_json::Value::Object(sizes));
        }
    }
    serde_json::Value::Object(modes).to_string()
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
        assert_eq!(measure(0, 0, || Ok::<_, ()>(())), Ok(()));
        assert_eq!(
            measure(0, 0, || Err::<(), _>("injection failed")),
            Err("injection failed")
        );
        assert!(snapshot().sample_count >= before + 2);
    }

    #[test]
    fn profiles_are_segmented_by_mode_and_replacement_size() {
        let before: serde_json::Value = profiles_json().parse().unwrap();
        let before_count = before["ei_text"]["small"]["sample_count"]
            .as_u64()
            .unwrap_or(0);
        let _ = measure(0, 0, || Ok::<_, ()>(()));
        let profiles: serde_json::Value = profiles_json().parse().unwrap();
        assert!(profiles["ei_text"]["small"]["sample_count"]
            .as_u64()
            .is_some_and(|count| count > before_count));
    }
}
