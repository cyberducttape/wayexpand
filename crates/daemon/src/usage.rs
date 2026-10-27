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
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use tracing::warn;
use wayexpand_core::{ExpansionEngine, UsageStats};
#[cfg(test)]
use wayexpand_core::{InputEvent, UsageEvent};

const SAVE_INTERVAL: Duration = Duration::from_secs(60);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);
const SHUTDOWN_ENQUEUE_WAIT: Duration = Duration::from_millis(250);
const QUEUE_CAPACITY: usize = 8;
/// Shorter than the control client's 2 s I/O timeout, so a slow clear is
/// reported to the CLI instead of surfacing as a socket read timeout.
const CLEAR_REPLY_WAIT: Duration = Duration::from_millis(1500);

static MAX_FLUSH_DURATION_US: AtomicU64 = AtomicU64::new(0);
static FLUSH_FAILURES_TOTAL: AtomicU64 = AtomicU64::new(0);
static WORKER_FAILURES_TOTAL: AtomicU64 = AtomicU64::new(0);
static QUEUE_REJECTED_TOTAL: AtomicU64 = AtomicU64::new(0);
static EVENTS_RECORDED_TOTAL: AtomicU64 = AtomicU64::new(0);
static EVENTS_PERSISTED_TOTAL: AtomicU64 = AtomicU64::new(0);
static EVENTS_DROPPED_TOTAL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageMetrics {
    pub max_flush_duration_us: u64,
    pub flush_failures_total: u64,
    pub worker_failures_total: u64,
    pub queue_rejected_total: u64,
    pub events_recorded_total: u64,
    pub events_persisted_total: u64,
    pub events_dropped_total: u64,
}

pub fn metrics_snapshot() -> UsageMetrics {
    UsageMetrics {
        max_flush_duration_us: MAX_FLUSH_DURATION_US.load(Ordering::Relaxed),
        flush_failures_total: FLUSH_FAILURES_TOTAL.load(Ordering::Relaxed),
        worker_failures_total: WORKER_FAILURES_TOTAL.load(Ordering::Relaxed),
        queue_rejected_total: QUEUE_REJECTED_TOTAL.load(Ordering::Relaxed),
        events_recorded_total: EVENTS_RECORDED_TOTAL.load(Ordering::Relaxed),
        events_persisted_total: EVENTS_PERSISTED_TOTAL.load(Ordering::Relaxed),
        events_dropped_total: EVENTS_DROPPED_TOTAL.load(Ordering::Relaxed),
    }
}

pub struct UsageRecorder {
    path: PathBuf,
    sender: Option<SyncSender<UsageMessage>>,
    pending: UsageStats,
    pending_generation: u64,
    generation: Arc<Mutex<UsageGeneration>>,
    collection_boundary: Arc<Mutex<()>>,
    worker: Option<JoinHandle<()>>,
    worker_done: Option<Receiver<()>>,
    outstanding_events: Arc<AtomicU64>,
}

enum UsageMessage {
    Batch(UsageStats),
    /// Persist everything already queued and exit. Shutdown cannot rely on
    /// channel disconnection: the control server keeps a clear handle, and
    /// therefore a sender clone, alive until after the recorder stops.
    Shutdown,
    Clear(
        mpsc::SyncSender<std::io::Result<bool>>,
        Arc<Mutex<UsageGeneration>>,
        u64,
    ),
}

#[derive(Default)]
struct UsageGeneration {
    value: u64,
    clear_pending: bool,
    discard_through_sequence: u64,
}

#[derive(Clone)]
pub struct UsageClearHandle {
    sender: SyncSender<UsageMessage>,
    generation: Arc<Mutex<UsageGeneration>>,
    collection_boundary: Arc<Mutex<()>>,
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
        let _boundary = self
            .collection_boundary
            .lock()
            .map_err(|_| std::io::Error::other("usage collection boundary lock poisoned"))?;
        {
            let mut generation = self
                .generation
                .lock()
                .map_err(|_| std::io::Error::other("usage clear generation lock poisoned"))?;
            if generation.clear_pending {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "another usage statistics clear is in progress",
                ));
            }
            generation.clear_pending = true;
            let cutoff = wayexpand_core::latest_usage_event_sequence();
            self.sender
                .try_send(UsageMessage::Clear(
                    reply,
                    Arc::clone(&self.generation),
                    cutoff,
                ))
                .map_err(|error| {
                    generation.clear_pending = false;
                    std::io::Error::new(std::io::ErrorKind::WouldBlock, error.to_string())
                })?;
        }
        drop(_boundary);
        receive_clear_result(&result, CLEAR_REPLY_WAIT)
    }
}

fn receive_clear_result(
    result: &Receiver<std::io::Result<bool>>,
    timeout: Duration,
) -> std::io::Result<bool> {
    match result.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "usage statistics clear timed out; its outcome is unknown and it may still complete. Check stats before retrying",
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(std::io::Error::other(
            "usage statistics worker stopped before confirming the clear; its outcome is unknown. Check stats before retrying",
        )),
    }
}

impl UsageRecorder {
    pub fn new(path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let generation = Arc::new(Mutex::new(UsageGeneration::default()));
        let collection_boundary = Arc::new(Mutex::new(()));
        let (done_sender, done_receiver) = mpsc::channel();
        let worker_path = path.clone();
        let outstanding_events = Arc::new(AtomicU64::new(0));
        let worker_outstanding_events = Arc::clone(&outstanding_events);
        let worker = thread::Builder::new()
            .name("wayexpand-usage-writer".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    usage_writer(worker_path, receiver, &worker_outstanding_events);
                }));
                if result.is_err() {
                    WORKER_FAILURES_TOTAL.fetch_add(1, Ordering::Relaxed);
                    let dropped = worker_outstanding_events.swap(0, Ordering::Relaxed);
                    record_dropped(dropped);
                    warn!(
                        dropped_events = dropped,
                        "usage statistics worker panicked; outstanding events were lost"
                    );
                }
                let _ = done_sender.send(());
            });
        match worker {
            Ok(worker) => Self {
                path,
                sender: Some(sender),
                pending: UsageStats::default(),
                pending_generation: 0,
                generation: Arc::clone(&generation),
                collection_boundary: Arc::clone(&collection_boundary),
                worker: Some(worker),
                worker_done: Some(done_receiver),
                outstanding_events,
            },
            Err(error) => {
                WORKER_FAILURES_TOTAL.fetch_add(1, Ordering::Relaxed);
                warn!(%error, "could not start usage statistics worker; usage events will be dropped");
                Self {
                    path,
                    sender: None,
                    pending: UsageStats::default(),
                    pending_generation: 0,
                    generation,
                    collection_boundary,
                    worker: None,
                    worker_done: None,
                    outstanding_events,
                }
            }
        }
    }

    pub fn clear_handle(&self) -> Option<UsageClearHandle> {
        self.sender.as_ref().map(|sender| UsageClearHandle {
            sender: sender.clone(),
            generation: Arc::clone(&self.generation),
            collection_boundary: Arc::clone(&self.collection_boundary),
            path: self.path.clone(),
        })
    }

    /// Drain engine events and enqueue one bounded batch without waiting for
    /// the worker or the filesystem.
    pub fn collect(&mut self, engine: &mut ExpansionEngine) {
        let boundary = Arc::clone(&self.collection_boundary);
        let Ok(_boundary) = boundary.lock() else {
            return;
        };
        let (clear_pending, generation_value, discard_through_sequence) =
            if let Ok(generation) = self.generation.lock() {
                (
                    generation.clear_pending,
                    generation.value,
                    generation.discard_through_sequence,
                )
            } else {
                return;
            };
        if clear_pending {
            // Preserve events in the engine until the clear succeeds or
            // fails. Their sequence keys let us discard only pre-clear work.
            return;
        }
        if self.pending_generation != generation_value {
            // Pre-clear events in the recorder's unqueued batch are cleared.
            self.pending = UsageStats::default();
            self.pending_generation = generation_value;
        }
        let events = engine.drain_usage_events_sequenced();
        let engine_dropped = engine.take_dropped_usage_event_count();
        EVENTS_RECORDED_TOTAL.fetch_add(
            (events.len() as u64).saturating_add(engine_dropped),
            Ordering::Relaxed,
        );
        record_dropped(engine_dropped);
        for (sequence, event) in events {
            if sequence > discard_through_sequence {
                self.pending.record(&event);
            }
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
        if generation.clear_pending {
            return;
        }
        if self.pending_generation != generation.value {
            self.pending = UsageStats::default();
            self.pending_generation = generation.value;
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        let batch_events = event_count(&batch);
        self.outstanding_events
            .fetch_add(batch_events, Ordering::Relaxed);
        match sender.try_send(UsageMessage::Batch(batch)) {
            Ok(()) => {}
            Err(TrySendError::Full(UsageMessage::Batch(batch))) => {
                decrement_outstanding(&self.outstanding_events, batch_events);
                self.pending = batch;
            }
            Err(TrySendError::Disconnected(UsageMessage::Batch(batch))) => {
                let dropped = event_count(&batch).saturating_add(event_count(&self.pending));
                decrement_outstanding(&self.outstanding_events, batch_events);
                self.pending = UsageStats::default();
                self.sender = None;
                record_rejections(dropped);
                warn!(
                    dropped_events = dropped,
                    "usage statistics worker stopped unexpectedly"
                );
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                unreachable!("only usage batches are enqueued here")
            }
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
        // Ask the worker to persist everything already queued. The control
        // server's clear handle keeps the channel connected, so dropping our
        // sender alone would leave the worker waiting for its next interval.
        if let Some(sender) = self.sender.take() {
            let deadline = Instant::now() + SHUTDOWN_ENQUEUE_WAIT;
            loop {
                match sender.try_send(UsageMessage::Shutdown) {
                    Ok(()) | Err(TrySendError::Disconnected(_)) => break,
                    Err(TrySendError::Full(_)) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(TrySendError::Full(_)) => {
                        warn!("usage statistics worker did not accept the shutdown request");
                        break;
                    }
                }
            }
        }
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
            warn!(
                unconfirmed_events = self.outstanding_events.load(Ordering::Relaxed),
                "usage statistics worker exceeded its shutdown deadline; pending events may be lost if the daemon exits"
            );
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
    record_dropped(count);
}

fn record_dropped(count: u64) {
    EVENTS_DROPPED_TOTAL.fetch_add(count, Ordering::Relaxed);
}

fn decrement_outstanding(counter: &AtomicU64, count: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_sub(count))
    });
}

fn usage_writer(path: PathBuf, receiver: Receiver<UsageMessage>, outstanding: &AtomicU64) {
    let mut pending = UsageStats::default();
    let mut last_flush = Instant::now();
    loop {
        let wait = SAVE_INTERVAL.saturating_sub(last_flush.elapsed());
        match receiver.recv_timeout(wait) {
            Ok(UsageMessage::Batch(batch)) => {
                pending.merge(&batch);
            }
            Ok(UsageMessage::Clear(reply, generation, cutoff)) => {
                let cleared_pending_events = event_count(&pending);
                let result = clear_pending(&path, &mut pending);
                if result.is_err() {
                    FLUSH_FAILURES_TOTAL.fetch_add(1, Ordering::Relaxed);
                } else {
                    decrement_outstanding(outstanding, cleared_pending_events);
                }
                if let Ok(mut generation) = generation.lock() {
                    generation.clear_pending = false;
                    if result.is_ok() {
                        generation.value = generation.value.wrapping_add(1);
                        generation.discard_through_sequence =
                            generation.discard_through_sequence.max(cutoff);
                    }
                }
                let _ = reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !pending.snippets.is_empty() {
                    flush_pending(&path, &mut pending, outstanding);
                }
                // Reset even on an idle timeout; otherwise a zero-duration
                // receive would spin forever after the first minute.
                last_flush = Instant::now();
            }
            Ok(UsageMessage::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                for _ in 0..3 {
                    if pending.snippets.is_empty()
                        || flush_pending(&path, &mut pending, outstanding)
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                if !pending.snippets.is_empty() {
                    let dropped = event_count(&pending);
                    record_dropped(dropped);
                    decrement_outstanding(outstanding, dropped);
                    warn!(
                        unpersisted_events = dropped,
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

fn clear_pending(path: &std::path::Path, pending: &mut UsageStats) -> std::io::Result<bool> {
    let cleared = UsageStats::clear(path)?;
    *pending = UsageStats::default();
    Ok(cleared)
}

fn flush_pending(
    path: &std::path::Path,
    pending: &mut UsageStats,
    outstanding: &AtomicU64,
) -> bool {
    let started = Instant::now();
    let persisted = event_count(pending);
    match UsageStats::merge_and_save(path, pending) {
        Ok(()) => {
            *pending = UsageStats::default();
            EVENTS_PERSISTED_TOTAL.fetch_add(persisted, Ordering::Relaxed);
            decrement_outstanding(outstanding, persisted);
        }
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
    fn collected_usage_reports_recorded_and_successfully_persisted_events() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-usage-counters-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("usage-stats.json");
        let config = wayexpand_core::Config::parse(
            "[[expansion]]\ntrigger = ':sig'\nreplacement = 'Best regards'\n",
        )
        .unwrap();
        let snippet_id = config.expansion[0].id.clone();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let mut results = engine.process(InputEvent::Text(":sig".into()));
        results.extend(engine.process(InputEvent::EndOfInput));
        engine.commit_applied_expansion(&results[0]);

        let recorded_before = metrics_snapshot().events_recorded_total;
        let persisted_before = metrics_snapshot().events_persisted_total;
        let mut recorder = UsageRecorder::new(path.clone());
        recorder.collect(&mut engine);
        recorder.request_shutdown();
        recorder.wait_for_shutdown();

        assert!(metrics_snapshot().events_recorded_total > recorded_before);
        assert!(metrics_snapshot().events_persisted_total > persisted_before);
        assert_eq!(
            UsageStats::load(&path).unwrap().snippets[&snippet_id].count,
            1
        );
        let _ = std::fs::remove_dir_all(directory);
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
        assert_eq!(UsageStats::load(&path).unwrap().snippets["a"].count, 1);

        // A clear (file removed) is respected by the next save.
        std::fs::remove_file(&path).unwrap();
        let mut recorder = UsageRecorder::new(path.clone());
        recorder.pending.record(&event("b"));
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        let stats = UsageStats::load(&path).unwrap();
        assert!(!stats.snippets.contains_key("a"));
        assert_eq!(stats.snippets["b"].count, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn clear_discards_engine_buffered_events_but_keeps_later_expansions() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-usage-clear-engine-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("usage-stats.json");
        let config = wayexpand_core::Config::parse(
            "[[expansion]]\ntrigger = ':sig'\nreplacement = 'Best regards'\n",
        )
        .unwrap();
        let snippet_id = config.expansion[0].id.clone();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let mut recorder = UsageRecorder::new(path.clone());

        let expand = |engine: &mut ExpansionEngine| {
            let mut results = engine.process(InputEvent::Text(":sig".into()));
            results.extend(engine.process(InputEvent::EndOfInput));
            engine.commit_applied_expansion(&results[0]);
        };
        expand(&mut engine);
        recorder
            .clear_handle()
            .unwrap()
            .clear(&path)
            .expect("clear should complete");
        expand(&mut engine);
        recorder.collect(&mut engine);
        recorder.request_shutdown();
        recorder.wait_for_shutdown();

        let stats = UsageStats::load(&path).unwrap();
        assert_eq!(stats.snippets[&snippet_id].count, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn shutdown_persists_promptly_while_a_clear_handle_is_alive() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-usage-shutdown-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("usage-stats.json");
        let mut recorder = UsageRecorder::new(path.clone());
        // The control server holds this for the daemon's whole lifetime.
        let _clear_handle = recorder.clear_handle().unwrap();
        recorder.pending.record(&event("kept"));
        let started = Instant::now();
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        assert!(started.elapsed() < SHUTDOWN_WAIT);
        assert_eq!(UsageStats::load(&path).unwrap().snippets["kept"].count, 1);
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
        assert!(UsageStats::load(&path).unwrap().snippets.is_empty());

        recorder.pending_generation = recorder.generation.lock().unwrap().value;
        recorder.pending.record(&event("after-clear"));
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        let stats = UsageStats::load(&path).unwrap();
        assert!(!stats.snippets.contains_key("persisted-before-clear"));
        assert!(!stats.snippets.contains_key("queued-before-clear"));
        assert_eq!(stats.snippets["after-clear"].count, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn clear_timeout_reports_that_the_operation_may_still_complete() {
        let (reply, result) = mpsc::sync_channel(1);
        let delayed_reply = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            reply.send(Ok(true)).unwrap();
        });

        let error = receive_clear_result(&result, Duration::from_millis(1))
            .expect_err("the caller's deadline should expire");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("outcome is unknown"));
        assert!(error.to_string().contains("Check stats before retrying"));
        delayed_reply.join().unwrap();
    }

    #[test]
    fn failed_clear_retains_writer_buffered_events() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-usage-clear-failure-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut pending = UsageStats::default();
        pending.record(&event("retained"));

        assert!(clear_pending(&directory, &mut pending).is_err());
        assert_eq!(pending.snippets["retained"].count, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_daemon_clear_preserves_reactor_pending_events_and_generation() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wayexpand-usage-clear-daemon-failure-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let mut recorder = UsageRecorder::new(path.clone());
        recorder.pending.record(&event("retained"));

        assert!(recorder.clear_handle().unwrap().clear(&path).is_err());
        assert_eq!(recorder.generation.lock().unwrap().value, 0);
        assert_eq!(recorder.pending.snippets["retained"].count, 1);
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        let _ = std::fs::remove_dir_all(path);
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
        let dropped_before = metrics_snapshot().events_dropped_total;
        let mut recorder = UsageRecorder::new(path);
        recorder.pending.record(&event("failure"));
        recorder.request_shutdown();
        recorder.wait_for_shutdown();
        assert!(metrics_snapshot().flush_failures_total > before);
        assert!(metrics_snapshot().events_dropped_total > dropped_before);
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
            generation: Arc::new(Mutex::new(UsageGeneration::default())),
            collection_boundary: Arc::new(Mutex::new(())),
            worker: None,
            worker_done: None,
            outstanding_events: Arc::new(AtomicU64::new(0)),
        };
        let started = Instant::now();
        recorder.enqueue_pending();
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(recorder.pending, pending);
    }
}
