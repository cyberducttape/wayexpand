//! Bounded rolling measurements of output-backend apply time.
//!
//! This measures the completed output-backend transaction, including the
//! completion acknowledgement from the serialized output actor, not
//! keystroke-to-paint latency. Keeping that distinction explicit prevents
//! backend timing from being presented as a full desktop end-to-end SLO.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock, TryLockError,
    },
    time::{Duration, Instant},
};
use wayexpand_core::{ExpansionEngine, ExpansionResult, TextInjector, TransactionOutcome};

const WINDOW_CAPACITY: usize = 1024;
/// Status is rebuilt every reactor turn (every key event) and the matcher
/// window changes on each one; sorting the window that often would cost more
/// than the matching it measures. Counts stay live; percentiles refresh at
/// most this often.
const PERCENTILE_REFRESH: Duration = Duration::from_secs(1);

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
    pub mean_us: u64,
}

#[derive(Default)]
struct Window {
    samples_ns: VecDeque<u64>,
    total: u64,
    cached: Snapshot,
    dirty: bool,
    percentiles_at: Option<Instant>,
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
        self.cached.sample_count = self.total;
        self.cached.window_count = self.samples_ns.len();
        if !self.dirty
            || self
                .percentiles_at
                .is_some_and(|computed| computed.elapsed() < PERCENTILE_REFRESH)
        {
            return self.cached;
        }
        self.percentiles_at = Some(Instant::now());
        let mut sorted = self.samples_ns.iter().copied().collect::<Vec<_>>();
        sorted.sort_unstable();
        self.cached = Snapshot {
            sample_count: self.total,
            window_count: sorted.len(),
            p50_us: percentile_us(&sorted, 50),
            p95_us: percentile_us(&sorted, 95),
            p99_us: percentile_us(&sorted, 99),
            mean_us: sorted
                .iter()
                .map(|sample| u128::from(*sample))
                .sum::<u128>()
                .div_ceil(sorted.len() as u128 * 1_000) as u64,
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

static MATCHER_DROPPED_SAMPLES: AtomicU64 = AtomicU64::new(0);
static OUTPUT_DROPPED_SAMPLES: AtomicU64 = AtomicU64::new(0);
static PROFILE_DROPPED_SAMPLES: AtomicU64 = AtomicU64::new(0);

/// Number of matcher timing samples skipped because a diagnostics reader held
/// the rolling-window lock. Recording is best-effort so metrics never delay
/// input processing.
pub fn matcher_dropped_samples() -> u64 {
    MATCHER_DROPPED_SAMPLES.load(Ordering::Relaxed)
}

/// Output timing samples omitted because a diagnostics snapshot held the
/// aggregate rolling-window lock.
pub fn output_dropped_samples() -> u64 {
    OUTPUT_DROPPED_SAMPLES.load(Ordering::Relaxed)
}

/// Per-mode output timing samples omitted because a diagnostics snapshot held
/// the segmented profile lock.
pub fn profile_dropped_samples() -> u64 {
    PROFILE_DROPPED_SAMPLES.load(Ordering::Relaxed)
}

fn profile_windows() -> &'static Mutex<ProfileWindows> {
    static WINDOWS: OnceLock<Mutex<ProfileWindows>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(ProfileWindows::default()))
}

fn increment_saturating(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

fn record_matcher(elapsed_ns: u64) {
    record_nonblocking(matcher_window(), elapsed_ns, &MATCHER_DROPPED_SAMPLES);
}

fn snapshot_cache() -> &'static Mutex<Snapshot> {
    static CACHE: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Snapshot::default()))
}

fn nonblocking_snapshot(source: &Mutex<Window>, cache: &Mutex<Snapshot>) -> Snapshot {
    let snapshot = match source.try_lock() {
        Ok(mut state) => Some(state.snapshot()),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner().snapshot()),
        Err(TryLockError::WouldBlock) => None,
    };
    if let Some(snapshot) = snapshot {
        if let Ok(mut cached) = cache.try_lock() {
            *cached = snapshot;
        }
        snapshot
    } else {
        cache
            .try_lock()
            .map_or_else(|_| Snapshot::default(), |cached| *cached)
    }
}

fn record_nonblocking(source: &Mutex<Window>, elapsed_ns: u64, dropped: &AtomicU64) {
    match source.try_lock() {
        Ok(mut state) => state.record(elapsed_ns),
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().record(elapsed_ns),
        Err(TryLockError::WouldBlock) => increment_saturating(dropped),
    }
}

pub fn snapshot() -> Snapshot {
    nonblocking_snapshot(window(), snapshot_cache())
}

/// Snapshot of time spent processing input through the matcher and preparing
/// an expansion, excluding output injection and its serialized actor.
pub fn matcher_snapshot() -> Snapshot {
    static CACHE: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    nonblocking_snapshot(
        matcher_window(),
        CACHE.get_or_init(|| Mutex::new(Snapshot::default())),
    )
}

pub fn apply(injector: &mut dyn TextInjector, result: &ExpansionResult) -> TransactionOutcome {
    let mode = output_mode(injector.status_detail());
    let size = output_size_bucket(result.insert.chars().count());
    let mut traced = crate::failure_trace::TracedInjector::new(injector);
    let report = measure(mode, size, || {
        ExpansionEngine::apply_with_report(&mut traced, result)
    });
    traced.record_outcome(&report.outcome, &report.phases);
    let outcome = report.outcome;
    crate::output_health::note_outcome(&outcome);
    outcome
}

pub fn measure_matcher<T>(operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    let elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    record_matcher(elapsed_ns);
    result
}

fn measure<T>(mode: usize, size: usize, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let outcome = operation();
    let elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    record_nonblocking(window(), elapsed_ns, &OUTPUT_DROPPED_SAMPLES);
    match profile_windows().try_lock() {
        Ok(mut profiles) => profiles.windows[mode][size].record(elapsed_ns),
        Err(TryLockError::Poisoned(poisoned)) => {
            poisoned.into_inner().windows[mode][size].record(elapsed_ns)
        }
        Err(TryLockError::WouldBlock) => increment_saturating(&PROFILE_DROPPED_SAMPLES),
    }
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
    static CACHE: OnceLock<Mutex<String>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new("{}".to_owned()));
    profiles_json_from(profile_windows(), cache)
}

fn profiles_json_from(source: &Mutex<ProfileWindows>, cache: &Mutex<String>) -> String {
    let mut profiles = match source.try_lock() {
        Ok(profiles) => profiles,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => {
            return cache
                .try_lock()
                .map_or_else(|_| "{}".to_owned(), |cached| cached.clone());
        }
    };
    let snapshots: [[Snapshot; 3]; 5] = std::array::from_fn(|mode| {
        std::array::from_fn(|size| profiles.windows[mode][size].snapshot())
    });
    drop(profiles);
    let mut modes = serde_json::Map::new();
    for (mode_index, mode) in OUTPUT_MODES.iter().enumerate() {
        let mut sizes = serde_json::Map::new();
        for (size_index, size) in OUTPUT_SIZE_BUCKETS.iter().enumerate() {
            let snapshot = snapshots[mode_index][size_index];
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
                    "mean_us": snapshot.mean_us,
                }),
            );
        }
        if !sizes.is_empty() {
            modes.insert((*mode).to_owned(), serde_json::Value::Object(sizes));
        }
    }
    let serialized = serde_json::Value::Object(modes).to_string();
    if let Ok(mut cached) = cache.try_lock() {
        *cached = serialized.clone();
    }
    serialized
}

/// Record a very slow sample in every profile window, so status-size tests
/// see the largest profile object the daemon can report.
#[cfg(test)]
pub(crate) fn saturate_profiles_for_test() {
    for mode in 0..OUTPUT_MODES.len() {
        for size in 0..OUTPUT_SIZE_BUCKETS.len() {
            profile_windows()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .windows[mode][size]
                .record(u64::MAX / 2);
        }
    }
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
    fn matcher_measurement_skips_contended_window_without_blocking() {
        let mut guard = matcher_window().lock().unwrap();
        let dropped_before = matcher_dropped_samples();
        assert_eq!(measure_matcher(|| 42), 42);
        assert_eq!(matcher_dropped_samples(), dropped_before + 1);
        guard.record(1_000);
    }

    #[test]
    fn output_measurement_drops_contended_samples_instead_of_waiting() {
        let aggregate = window().lock().unwrap();
        let profiles = profile_windows().lock().unwrap();
        let output_dropped_before = output_dropped_samples();
        let profiles_dropped_before = profile_dropped_samples();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender.send(measure(0, 0, || 17)).unwrap();
        });

        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(100)),
            Ok(17),
            "recording must return while diagnostics holds both metric locks"
        );
        drop(aggregate);
        drop(profiles);
        worker.join().unwrap();
        assert!(output_dropped_samples() > output_dropped_before);
        assert!(profile_dropped_samples() > profiles_dropped_before);
    }

    #[test]
    fn snapshot_reads_fall_back_to_the_last_cached_value_without_waiting() {
        let source = std::sync::Arc::new(Mutex::new(Window::default()));
        let cache = Mutex::new(Snapshot {
            sample_count: 8,
            window_count: 3,
            p50_us: 4,
            p95_us: 7,
            p99_us: 8,
            mean_us: 5,
        });
        let source_guard = source.lock().unwrap();
        let source_for_worker = std::sync::Arc::clone(&source);
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(nonblocking_snapshot(&source_for_worker, &cache))
                .unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_millis(100));
        drop(source_guard);
        assert_eq!(result.unwrap().sample_count, 8);
        worker.join().unwrap();
    }

    #[test]
    fn profile_snapshot_uses_cached_json_when_the_window_is_contended() {
        let source = std::sync::Arc::new(Mutex::new(ProfileWindows::default()));
        let cache = std::sync::Arc::new(Mutex::new("{\"last\":1}".to_owned()));
        let source_guard = source.lock().unwrap();
        let source_for_worker = std::sync::Arc::clone(&source);
        let cache_for_worker = std::sync::Arc::clone(&cache);
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(profiles_json_from(&source_for_worker, &cache_for_worker))
                .unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_millis(100));
        drop(source_guard);
        assert_eq!(result.unwrap(), "{\"last\":1}");
        worker.join().unwrap();
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
