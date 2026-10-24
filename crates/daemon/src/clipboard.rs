//! `{{clipboard}}` support for the daemon.
//!
//! Reads the clipboard with `wl-paste` (wl-clipboard) through the same
//! bounded, shell-free command runner used for command-backed snippets: fixed
//! arguments, a minimal environment that passes only the Wayland session
//! variables, a short timeout, and the usual output cap. The clipboard is read
//! only for a snippet that uses the variable, and only when the user enabled
//! it (`settings.allow_clipboard`) and policy allows it. Contents are never
//! logged or cached.
//!
//! To keep `wl-paste` off the keystroke path, the engine may ask for a read a
//! little early, once the typed text can only become a `{{clipboard}}`
//! trigger. That value is held for at most [`PREFETCH_MAX_AGE`], handed to
//! the very next render, and then dropped; nothing monitors the clipboard.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use wayexpand_core::{
    run_command, ClipboardPrefetch, ClipboardReader, CommandConfig, CommandEnvironment,
};

/// How long a prefetched value may wait for its render before it is erased.
/// Typing the rest of a trigger takes well under this.
const PREFETCH_MAX_AGE: Duration = Duration::from_secs(2);

/// Upper bound on one `{{clipboard}}` read. A healthy `wl-paste` returns in a
/// few milliseconds; this only limits how long a stuck clipboard owner can
/// delay an expansion. Reading on demand (never caching) is deliberate, see
/// the module docs.
const CLIPBOARD_TIMEOUT_MS: u64 = 150;

static WORKER_THREADS: AtomicUsize = AtomicUsize::new(0);
static READS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static READS_TOTAL: AtomicU64 = AtomicU64::new(0);
static READ_FAILURES_TOTAL: AtomicU64 = AtomicU64::new(0);
static DEADLINE_MISSES_TOTAL: AtomicU64 = AtomicU64::new(0);
static MAX_READ_DURATION_US: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MetricsSnapshot {
    pub worker_threads: usize,
    pub reads_in_flight: usize,
    pub reads_total: u64,
    pub read_failures_total: u64,
    pub deadline_misses_total: u64,
    pub max_read_duration_us: u64,
}

pub(crate) fn metrics_snapshot() -> MetricsSnapshot {
    MetricsSnapshot {
        worker_threads: WORKER_THREADS.load(Ordering::Relaxed),
        reads_in_flight: READS_IN_FLIGHT.load(Ordering::Relaxed),
        reads_total: READS_TOTAL.load(Ordering::Relaxed),
        read_failures_total: READ_FAILURES_TOTAL.load(Ordering::Relaxed),
        deadline_misses_total: DEADLINE_MISSES_TOTAL.load(Ordering::Relaxed),
        max_read_duration_us: MAX_READ_DURATION_US.load(Ordering::Relaxed),
    }
}

struct WorkerRegistration;

impl WorkerRegistration {
    fn new() -> Self {
        WORKER_THREADS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for WorkerRegistration {
    fn drop(&mut self) {
        WORKER_THREADS.fetch_sub(1, Ordering::Relaxed);
    }
}

struct ActiveRead;

impl ActiveRead {
    fn new() -> Self {
        READS_IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
        READS_TOTAL.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for ActiveRead {
    fn drop(&mut self) {
        READS_IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
    }
}

fn wl_paste_command() -> CommandConfig {
    CommandConfig {
        action: None,
        program: "wl-paste".into(),
        args: vec![
            "--no-newline".into(),
            "--type".into(),
            "text/plain;charset=utf-8".into(),
        ],
        timeout_ms: CLIPBOARD_TIMEOUT_MS,
        cache_ms: 0,
        environment: CommandEnvironment::Minimal,
        pass_env: vec!["WAYLAND_DISPLAY".into(), "XDG_RUNTIME_DIR".into()],
    }
}

type ReadClipboard = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// The single in-flight or completed early read.
#[derive(Default)]
enum Prefetched {
    #[default]
    Empty,
    Pending {
        generation: u64,
        deadline: Instant,
    },
    Ready {
        generation: u64,
        at: Instant,
        value: String,
    },
    Unavailable {
        generation: u64,
    },
}

#[derive(Default)]
struct Slot {
    /// Distinguishes reads, so a finished thread never overwrites or erases
    /// a newer read's value.
    generation: u64,
    value: Prefetched,
}

struct Shared {
    slot: Mutex<Slot>,
    ready: Condvar,
}

struct ReadJob {
    generation: u64,
    deadline: Instant,
}

/// The `{{clipboard}}` reader and its prefetch hook, sharing one slot.
pub fn wl_paste_clipboard() -> (ClipboardReader, ClipboardPrefetch) {
    let command = wl_paste_command();
    prefetching_clipboard(
        Arc::new(move || run_command(&command).ok()),
        PREFETCH_MAX_AGE,
    )
}

fn prefetching_clipboard(
    read: ReadClipboard,
    max_age: Duration,
) -> (ClipboardReader, ClipboardPrefetch) {
    let shared = Arc::new(Shared {
        slot: Mutex::new(Slot::default()),
        ready: Condvar::new(),
    });
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker_shared = Arc::clone(&shared);
    let _worker = std::thread::Builder::new()
        .name("wayexpand-clipboard".into())
        .spawn(move || clipboard_worker(receiver, worker_shared, read, max_age));
    let reader = {
        let shared = Arc::clone(&shared);
        let sender = sender.clone();
        ClipboardReader(Arc::new(move || take_or_read(&shared, &sender, max_age)))
    };
    let prefetch = ClipboardPrefetch(Arc::new(move || start_prefetch(&shared, &sender)));
    (reader, prefetch)
}

fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Slot> {
    shared
        .slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn queue_read(shared: &Shared, sender: &mpsc::SyncSender<ReadJob>, slot: &mut Slot) -> Option<u64> {
    slot.generation = slot.generation.wrapping_add(1);
    let generation = slot.generation;
    let deadline = Instant::now() + Duration::from_millis(CLIPBOARD_TIMEOUT_MS);
    slot.value = Prefetched::Pending {
        generation,
        deadline,
    };
    if sender
        .try_send(ReadJob {
            generation,
            deadline,
        })
        .is_err()
    {
        slot.value = Prefetched::Unavailable { generation };
        shared.ready.notify_all();
        return None;
    }
    Some(generation)
}

fn start_prefetch(shared: &Shared, sender: &mpsc::SyncSender<ReadJob>) {
    let mut slot = lock(shared);
    // A trigger has one prefetch opportunity. Don't launch overlapping reads;
    // a later trigger can share a live in-flight operation, but a completed
    // unconsumed value is replaced so it cannot leak into a different trigger.
    if matches!(slot.value, Prefetched::Pending { .. }) {
        return;
    }
    let _ = queue_read(shared, sender, &mut slot);
}

/// Wait for the current single-flight read. A timed-out prefetch remains
/// pending until its worker finishes, so a later render never starts a second
/// clipboard command while the first may still be running.
fn take_or_read(
    shared: &Shared,
    sender: &mpsc::SyncSender<ReadJob>,
    max_age: Duration,
) -> Option<String> {
    let mut slot = lock(shared);
    let generation = match &slot.value {
        Prefetched::Ready {
            generation: _,
            at,
            value: _,
        } if at.elapsed() < max_age => {
            if let Prefetched::Ready { value, .. } = std::mem::take(&mut slot.value) {
                shared.ready.notify_all();
                return Some(value);
            }
            return None;
        }
        Prefetched::Pending { generation, .. } => *generation,
        Prefetched::Unavailable { .. } => {
            slot.value = Prefetched::Empty;
            return None;
        }
        Prefetched::Empty | Prefetched::Ready { .. } => {
            let _ = std::mem::take(&mut slot.value);
            queue_read(shared, sender, &mut slot)?
        }
    };

    loop {
        match &slot.value {
            Prefetched::Pending {
                generation: active_generation,
                deadline,
            } if *active_generation == generation => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return None;
                }
                slot = shared
                    .ready
                    .wait_timeout(slot, remaining)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0;
            }
            Prefetched::Ready {
                generation: active_generation,
                at,
                ..
            } if *active_generation == generation => {
                if at.elapsed() >= max_age {
                    slot.value = Prefetched::Empty;
                    return None;
                }
                let Prefetched::Ready { value, .. } = std::mem::take(&mut slot.value) else {
                    unreachable!()
                };
                shared.ready.notify_all();
                return Some(value);
            }
            Prefetched::Unavailable {
                generation: active_generation,
            } if *active_generation == generation => {
                slot.value = Prefetched::Empty;
                return None;
            }
            // Another concurrent render consumed this generation or a newer
            // trigger superseded it. Never enqueue a duplicate as a fallback.
            _ => return None,
        }
    }
}

fn clipboard_worker(
    receiver: mpsc::Receiver<ReadJob>,
    shared: Arc<Shared>,
    read: ReadClipboard,
    max_age: Duration,
) {
    let _worker = WorkerRegistration::new();
    while let Ok(mut job) = receiver.recv() {
        loop {
            let pending = matches!(
                lock(&shared).value,
                Prefetched::Pending { generation, .. } if generation == job.generation
            );
            if !pending {
                break;
            }
            // Each missed deadline is counted once, below, whether the job
            // expired in the queue or during its read.
            let value = if Instant::now() < job.deadline {
                let started = Instant::now();
                let active = ActiveRead::new();
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read()))
                    .unwrap_or(None);
                drop(active);
                let duration_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
                MAX_READ_DURATION_US.fetch_max(duration_us, Ordering::Relaxed);
                if value.is_none() {
                    READ_FAILURES_TOTAL.fetch_add(1, Ordering::Relaxed);
                }
                value
            } else {
                None
            };
            let completed_at = Instant::now();
            if completed_at > job.deadline {
                DEADLINE_MISSES_TOTAL.fetch_add(1, Ordering::Relaxed);
            }
            let mut slot = lock(&shared);
            let still_pending = matches!(
                slot.value,
                Prefetched::Pending { generation, .. } if generation == job.generation
            );
            let ready_until = if still_pending && completed_at <= job.deadline {
                match value {
                    Some(value) => {
                        slot.value = Prefetched::Ready {
                            generation: job.generation,
                            at: completed_at,
                            value,
                        };
                        Some(completed_at + max_age)
                    }
                    None => {
                        slot.value = Prefetched::Unavailable {
                            generation: job.generation,
                        };
                        None
                    }
                }
            } else if still_pending {
                slot.value = Prefetched::Unavailable {
                    generation: job.generation,
                };
                None
            } else {
                None
            };
            drop(slot);
            shared.ready.notify_all();

            if let Some(expiry) = ready_until {
                let remaining = expiry.saturating_duration_since(Instant::now());
                match receiver.recv_timeout(remaining) {
                    Ok(next_job) => {
                        job = next_job;
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let mut slot = lock(&shared);
                        if matches!(
                            slot.value,
                            Prefetched::Ready { generation, .. }
                                if generation == job.generation
                        ) {
                            slot.value = Prefetched::Empty;
                        }
                        drop(slot);
                        shared.ready.notify_all();
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_command_is_bounded_and_passes_only_wayland_variables() {
        let command = wl_paste_command();
        wayexpand_core::validate_command_config(&command).unwrap();
        assert_eq!(command.environment, CommandEnvironment::Minimal);
        assert_eq!(command.pass_env, ["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"]);
        assert!(command.timeout_ms <= 150);
        assert_eq!(command.cache_ms, 0);
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counting_read(delay: Duration) -> (ReadClipboard, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let read: ReadClipboard = Arc::new(move || {
            let call = counter.fetch_add(1, Ordering::SeqCst) + 1;
            std::thread::sleep(delay);
            Some(format!("value-{call}"))
        });
        (read, calls)
    }

    #[test]
    fn render_uses_a_prefetched_value_once() {
        let (read, calls) = counting_read(Duration::from_millis(30));
        let (reader, prefetch) = prefetching_clipboard(read, Duration::from_secs(5));
        (prefetch.0)();
        (prefetch.0)(); // already in flight: no second read
        assert_eq!((reader.0)().as_deref(), Some("value-1"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Consumed: the next render reads again.
        assert_eq!((reader.0)().as_deref(), Some("value-2"));
    }

    #[test]
    fn unused_prefetched_value_is_erased_after_max_age() {
        let (read, calls) = counting_read(Duration::ZERO);
        let max_age = Duration::from_millis(50);
        let (reader, prefetch) = prefetching_clipboard(read, max_age);
        (prefetch.0)();
        std::thread::sleep(Duration::from_millis(200));
        // The stale value is gone; the reader obtains a fresh generation.
        assert_eq!((reader.0)().as_deref(), Some("value-2"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn render_without_prefetch_reads_synchronously() {
        let (read, calls) = counting_read(Duration::ZERO);
        let (reader, _prefetch) = prefetching_clipboard(read, Duration::from_secs(5));
        assert_eq!((reader.0)().as_deref(), Some("value-1"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn delayed_prefetch_deadline_does_not_start_a_duplicate_read() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started_sender, started_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let release_receiver = Mutex::new(release_receiver);
        let counter = Arc::clone(&calls);
        let read: ReadClipboard = Arc::new(move || {
            let call = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if call == 1 {
                started_sender.send(()).unwrap();
                release_receiver.lock().unwrap().recv().ok()?;
            }
            Some(format!("value-{call}"))
        });
        let (reader, prefetch) = prefetching_clipboard(read, Duration::from_secs(1));

        (prefetch.0)();
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("prefetch worker started the first read");

        // A later trigger arrives while the first read is still in flight.
        // It must share the pending operation and, once its deadline passes,
        // report unavailable instead of starting a parallel read.
        (prefetch.0)();
        assert_eq!((reader.0)(), None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        release_sender.send(()).unwrap();
        let recovery_deadline = Instant::now() + Duration::from_secs(1);
        while calls.load(Ordering::SeqCst) == 1 && Instant::now() < recovery_deadline {
            (prefetch.0)();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!((reader.0)().as_deref(), Some("value-2"));
    }
}
