//! Persists engine usage events away from the input reactor.
//!
//! The reactor only drains and enqueues bounded event batches. A dedicated
//! worker merges and writes them at most once a minute. Shutdown drains the
//! queue and gives the worker a bounded opportunity to finish its final save.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use tracing::warn;
#[cfg(test)]
use wayexpand_core::UsageEvent;
use wayexpand_core::{ExpansionEngine, UsageStats};

const SAVE_INTERVAL: Duration = Duration::from_secs(60);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);
const SHUTDOWN_ENQUEUE_WAIT: Duration = Duration::from_millis(250);
const QUEUE_CAPACITY: usize = 8;

static MAX_FLUSH_DURATION_US: AtomicU64 = AtomicU64::new(0);
static FLUSH_FAILURES_TOTAL: AtomicU64 = AtomicU64::new(0);
static QUEUE_REJECTED_TOTAL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageMetrics {
    pub max_flush_duration_us: u64,
    pub flush_failures_total: u64,
    pub queue_rejected_total: u64,
}

pub fn metrics_snapshot() -> UsageMetrics {
    UsageMetrics {
        max_flush_duration_us: MAX_FLUSH_DURATION_US.load(Ordering::Relaxed),
        flush_failures_total: FLUSH_FAILURES_TOTAL.load(Ordering::Relaxed),
        queue_rejected_total: QUEUE_REJECTED_TOTAL.load(Ordering::Relaxed),
    }
}

pub struct UsageRecorder {
    path: PathBuf,
    sender: Option<SyncSender<UsageMessage>>,
    pending: UsageStats,
    pending_generation: u64,
    generation: Arc<Mutex<u64>>,
    worker: Option<JoinHandle<()>>,
    worker_done: Option<Receiver<()>>,
}

enum UsageMessage {
    Batch(UsageStats),
    Clear(mpsc::SyncSender<std::io::Result<bool>>),
}

#[derive(Clone)]
pub struct UsageClearHandle {
    sender: SyncSender<UsageMessage>,
    generation: Arc<Mutex<u64>>,
    path: PathBuf,
}

impl UsageClearHandle {
    pub fn clear(&self, requested_path: &std::path::Path) -> std::io::Result<bool> {
        if requested_path != self.path {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "requested statistics path does not match daemon configuration",
            ));
        }
        let (reply, result) = mpsc::sync_channel(1);
        {
            let mut generation = self
                .generation
                .lock()
                .map_err(|_| std::io::Error::other("usage clear generation lock poisoned"))?;
            self.sender
                .try_send(UsageMessage::Clear(reply))
                .map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::WouldBlock, error.to_string())
                })?;
            *generation = generation.wrapping_add(1);
        }
        result
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::TimedOut, error.to_string()))?
    }
}

impl UsageRecorder {
    pub fn new(path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let generation = Arc::new(Mutex::new(0));
        let (done_sender, done_receiver) = mpsc::channel();
        let worker_path = path.clone();
        let worker = thread::Builder::new()
            .name("wayexpand-usage-writer".into())
            .spawn(move || {
                usage_writer(worker_path, receiver);
                let _ = done_sender.send(());
            });
        match worker {
            Ok(worker) => Self {
                path,
                sender: Some(sender),
                pending: UsageStats::default(),
                pending_generation: 0,
                generation: Arc::clone(&generation),
                worker: Some(worker),
                worker_done: Some(done_receiver),
            },
            Err(error) => {
                warn!(%error, "could not start usage statistics worker; usage events will be dropped");
                Self {
                    path,
                    sender: None,
                    pending: UsageStats::default(),
                    pending_generation: 0,
                    generation,
                    worker: None,
                    worker_done: None,
                }
            }
        }
    }

    pub fn clear_handle(&self) -> Option<UsageClearHandle> {
        self.sender.as_ref().map(|sender| UsageClearHandle {
            sender: sender.clone(),
            generation: Arc::clone(&self.generation),
            path: self.path.clone(),
        })
    }

    /// Drain engine events and enqueue one bounded batch without waiting for
    /// the worker or the filesystem.
    pub fn collect(&mut self, engine: &mut ExpansionEngine) {
        if let Ok(generation) = self.generation.lock() {
            if self.pending_generation != *generation {
                let dropped = event_count(&self.pending);
                self.pending = UsageStats::default();
                self.pending_generation = *generation;
                record_rejections(dropped);
            }
        }
        for event in engine.drain_usage_events() {
            self.pending.record(&event);
        }
        self.enqueue_pending();
    }

    fn enqueue_pending(&mut self) {
        let Some(sender) = &self.sender else {
            if !self.pending.snippets.is_empty() {
                let dropped = event_count(&self.pending);
                self.pending = UsageStats::default();
                record_rejections(dropped);
            }
            return;
        };
        if self.pending.snippets.is_empty() {
            return;
        }
        let Ok(generation) = self.generation.lock() else {
            return;
        };
        if self.pending_generation != *generation {
            let dropped = event_count(&self.pending);
            self.pending = UsageStats::default();
            self.pending_generation = *generation;
            record_rejections(dropped);
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        match sender.try_send(UsageMessage::Batch(batch)) {
            Ok(()) => {}
            Err(TrySendError::Full(UsageMessage::Batch(batch))) => self.pending = batch,
            Err(TrySendError::Disconnected(UsageMessage::Batch(batch))) => {
                let dropped = event_count(&batch).saturating_add(event_count(&self.pending));
                self.pending = UsageStats::default();
                self.sender = None;
                record_rejections(dropped);
                warn!(
                    dropped_events = dropped,
                    "usage statistics worker stopped unexpectedly"
                );
            }
            Err(
                TrySendError::Full(UsageMessage::Clear(_))
                | TrySendError::Disconnected(UsageMessage::Clear(_)),
            ) => unreachable!("only usage batches are enqueued here"),
        }
    }

    fn begin_stop(&mut self) {
        if self.worker.is_none() || self.sender.is_none() {
            return;
        }
        let enqueue_deadline = Instant::now() + SHUTDOWN_ENQUEUE_WAIT;
        while !self.pending.snippets.is_empty() && Instant::now() < enqueue_deadline {
            self.enqueue_pending();
            if !self.pending.snippets.is_empty() {
                thread::sleep(Duration::from_millis(5));
            }
        }
        if !self.pending.snippets.is_empty() {
            let dropped = event_count(&self.pending);
            self.pending = UsageStats::default();
            record_rejections(dropped);
            warn!(
                dropped_events = dropped,
                "usage events could not be queued before shutdown"
            );
        }
        // Disconnect tells the worker to persist everything already queued.
        self.sender.take();
    }

    fn wait_for_worker(&mut self) {
        if self.worker.is_none() {
            return;
        }
        let finished = self
            .worker_done
            .as_ref()
            .is_some_and(|done| done.recv_timeout(SHUTDOWN_WAIT).is_ok());
        if finished {
            if let Some(worker) = self.worker.take() {
                if worker.join().is_err() {
                    warn!("usage statistics worker panicked during shutdown");
                }
            }
        } else {
            // Never let slow or stuck storage make daemon shutdown unbounded.
            warn!("usage statistics worker exceeded its shutdown deadline; it will finish asynchronously");
            self.worker.take();
        }
        self.worker_done.take();
    }

    fn stop(&mut self) {
        self.begin_stop();
        self.wait_for_worker();
    }

    /// Start the final asynchronous drain; call `wait_for_shutdown` when
    /// other service components have finished their own teardown.
    pub fn request_shutdown(&mut self) {
        self.begin_stop();
    }

    pub fn wait_for_shutdown(&mut self) {
        self.wait_for_worker();
    }
}

impl Drop for UsageRecorder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn record_rejections(count: u64) {
    QUEUE_REJECTED_TOTAL.fetch_add(count, Ordering::Relaxed);
}

fn usage_writer(path: PathBuf, receiver: Receiver<UsageMessage>) {
    let mut pending = UsageStats::default();
    let mut last_flush = Instant::now();
    loop {
        let wait = SAVE_INTERVAL.saturating_sub(last_flush.elapsed());
        match receiver.recv_timeout(wait) {
            Ok(UsageMessage::Batch(batch)) => {
                pending.merge(&batch);
            }
            Ok(UsageMessage::Clear(reply)) => {
                pending = UsageStats::default();
                let result = UsageStats::clear(&path);
                if result.is_err() {
                    FLUSH_FAILURES_TOTAL.fetch_add(1, Ordering::Relaxed);
                }
                let _ = reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !pending.snippets.is_empty() {
                    flush_pending(&path, &mut pending);
                }
                // Reset even on an idle timeout; otherwise a zero-duration
                // receive would spin forever after the first minute.
                last_flush = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                for _ in 0..3 {
                    if pending.snippets.is_empty() || flush_pending(&path, &mut pending) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                if !pending.snippets.is_empty() {
                    warn!(
                        unpersisted_events = event_count(&pending),
                        "usage events could not be persisted after final retries"
                    );
                }
                return;
            }
        }
    }
}

fn event_count(stats: &UsageStats) -> u64 {
    stats
        .snippets
        .values()
        .fold(0_u64, |count, usage| count.saturating_add(usage.count))
}

fn flush_pending(path: &std::path::Path, pending: &mut UsageStats) -> bool {
    let started = Instant::now();
    match UsageStats::merge_and_save(path, pending) {
        Ok(()) => *pending = UsageStats::default(),
        Err(error) => {
            FLUSH_FAILURES_TOTAL.fetch_add(1, Ordering::Relaxed);
            warn!(%error, "could not save local usage statistics; retaining events for retry");
        }
    }
    let elapsed_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    MAX_FLUSH_DURATION_US.fetch_max(elapsed_us, Ordering::Relaxed);
    tracing::debug!(
        usage_flush_duration_us = elapsed_us,
        usage_flush_max_duration_us = MAX_FLUSH_DURATION_US.load(Ordering::Relaxed),
        usage_flush_failures_total = FLUSH_FAILURES_TOTAL.load(Ordering::Relaxed),
        "usage statistics persistence completed"
    );
    pending.snippets.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str) -> UsageEvent {
        UsageEvent {
            snippet_id: id.into(),
            typed_chars: 4,
            inserted_chars: 20,
            unix_timestamp: 1_700_000_000,
        }
    }

    #[test]
    fn worker_persists_events_on_shutdown_and_honours_a_clear() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-usage-recorder-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("usage-stats.json");
        let mut recorder = UsageRecorder::new(path.clone());
        recorder.pending.record(&event("a"));
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        assert_eq!(UsageStats::load(&path).snippets["a"].count, 1);

        // A clear (file removed) is respected by the next save.
        std::fs::remove_file(&path).unwrap();
        let mut recorder = UsageRecorder::new(path.clone());
        recorder.pending.record(&event("b"));
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        let stats = UsageStats::load(&path);
        assert!(!stats.snippets.contains_key("a"));
        assert_eq!(stats.snippets["b"].count, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn serialized_clear_drops_already_queued_events_but_keeps_later_events() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-usage-clear-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("usage-stats.json");
        let mut persisted = UsageStats::default();
        persisted.record(&event("persisted-before-clear"));
        persisted.save(&path).unwrap();

        let mut recorder = UsageRecorder::new(path.clone());
        recorder.pending.record(&event("queued-before-clear"));
        recorder.enqueue_pending();
        recorder.clear_handle().unwrap().clear(&path).unwrap();
        assert!(UsageStats::load(&path).snippets.is_empty());

        recorder.pending_generation = *recorder.generation.lock().unwrap();
        recorder.pending.record(&event("after-clear"));
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        let stats = UsageStats::load(&path);
        assert!(!stats.snippets.contains_key("persisted-before-clear"));
        assert!(!stats.snippets.contains_key("queued-before-clear"));
        assert_eq!(stats.snippets["after-clear"].count, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn worker_failures_are_counted_and_shutdown_remains_bounded() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wayexpand-missing-parent-{}-{suffix}/stats.json",
            std::process::id(),
        ));
        let before = metrics_snapshot().flush_failures_total;
        let mut recorder = UsageRecorder::new(path);
        recorder.pending.record(&event("failure"));
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        assert!(metrics_snapshot().flush_failures_total > before);
    }

    #[test]
    fn full_queue_does_not_block_and_keeps_aggregated_counters() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        sender
            .try_send(UsageMessage::Batch(UsageStats::default()))
            .unwrap();
        let mut pending = UsageStats::default();
        pending.record(&event("retained"));
        let mut recorder = UsageRecorder {
            path: PathBuf::new(),
            sender: Some(sender),
            pending: pending.clone(),
            pending_generation: 0,
            generation: Arc::new(Mutex::new(0)),
            worker: None,
            worker_done: None,
        };
        let started = Instant::now();
        recorder.enqueue_pending();
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(recorder.pending, pending);
    }
}
