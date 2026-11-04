//! Output backend connections made off the reactor thread.
//!
//! Connecting libei through the RemoteDesktop portal can take up to the
//! portal's 30-second deadline. A reconnect made on the reactor would stop
//! it from handling stop requests, reloads, inserts, and status for that
//! long, so the reactor starts the attempt here and collects the result on a
//! later turn; the worker wakes it when the attempt ends.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, OnceLock,
    },
    thread::{self, JoinHandle},
};

use wayexpand_core::TextInjector;

use crate::output_loop::{connect_output_backend_with_cancellation, OutputConnectError};
use crate::waker::Waker;

pub(crate) type ConnectResult = Result<Box<dyn TextInjector>, OutputConnectError>;

/// Input-method and output setup may overlap, but a stalled portal must not
/// let repeated reconnects create an unbounded number of detached workers.
pub(crate) const MAX_CONNECT_WORKERS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConnectWorkerStats {
    pub active: usize,
    pub started_total: u64,
    pub completed_total: u64,
    pub cancelled_total: u64,
    pub rejected_total: u64,
}

struct WorkerCounters {
    active: AtomicUsize,
    started_total: AtomicU64,
    completed_total: AtomicU64,
    cancelled_total: AtomicU64,
    rejected_total: AtomicU64,
}

fn counters() -> &'static WorkerCounters {
    static COUNTERS: OnceLock<WorkerCounters> = OnceLock::new();
    COUNTERS.get_or_init(|| WorkerCounters {
        active: AtomicUsize::new(0),
        started_total: AtomicU64::new(0),
        completed_total: AtomicU64::new(0),
        cancelled_total: AtomicU64::new(0),
        rejected_total: AtomicU64::new(0),
    })
}

fn connection_worker_reaper() -> Option<SyncSender<JoinHandle<()>>> {
    static REAPER: OnceLock<Option<SyncSender<JoinHandle<()>>>> = OnceLock::new();
    REAPER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<JoinHandle<()>>(MAX_CONNECT_WORKERS);
            match thread::Builder::new()
                .name("wayexpand-connect-reaper".into())
                .spawn(move || {
                    while let Ok(worker) = receiver.recv() {
                        if worker.join().is_err() {
                            tracing::error!("output connection worker panicked while being reaped");
                        }
                    }
                }) {
                Ok(_) => Some(sender),
                Err(error) => {
                    tracing::error!(%error, "could not start output connection reaper");
                    None
                }
            }
        })
        .clone()
}

pub(crate) fn worker_stats() -> ConnectWorkerStats {
    let counters = counters();
    ConnectWorkerStats {
        active: counters.active.load(Ordering::Acquire),
        started_total: counters.started_total.load(Ordering::Relaxed),
        completed_total: counters.completed_total.load(Ordering::Relaxed),
        cancelled_total: counters.cancelled_total.load(Ordering::Relaxed),
        rejected_total: counters.rejected_total.load(Ordering::Relaxed),
    }
}

fn reserve_worker() -> bool {
    let active = &counters().active;
    let mut current = active.load(Ordering::Acquire);
    loop {
        if current >= MAX_CONNECT_WORKERS {
            counters().rejected_total.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        match active.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                counters().started_total.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            Err(observed) => current = observed,
        }
    }
}

struct WorkerGuard;

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        counters().active.fetch_sub(1, Ordering::AcqRel);
        counters().completed_total.fetch_add(1, Ordering::Relaxed);
    }
}

/// What a background attempt connects with; owned so it can cross threads.
pub(crate) struct ConnectRequest {
    pub backend: &'static str,
    pub persist_portal_token: bool,
    pub fcitx5_direct_commit: bool,
    pub portal_token_path: Option<PathBuf>,
}

/// One connection attempt in flight. Dropping it signals cooperative
/// cancellation to the worker. Portal operations race that signal against the
/// portal deadline, and EIS setup checks it between bounded reads.
pub(crate) struct BackgroundConnect {
    result: Receiver<ConnectResult>,
    cancellation: wayexpand_backend_libei::ConnectCancellation,
    finished: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl BackgroundConnect {
    pub(crate) fn start(request: ConnectRequest, waker: Waker) -> Self {
        Self::spawn(waker, move |cancellation| {
            connect_output_backend_with_cancellation(
                request.backend,
                request.persist_portal_token,
                request.fcitx5_direct_commit,
                request.portal_token_path.as_deref(),
                cancellation,
            )
        })
    }

    fn spawn(
        waker: Waker,
        connect: impl FnOnce(wayexpand_backend_libei::ConnectCancellation) -> ConnectResult
            + Send
            + 'static,
    ) -> Self {
        let (sender, result) = mpsc::sync_channel(1);
        let fallback_sender = sender.clone();
        let cancellation = wayexpand_backend_libei::ConnectCancellation::default();
        let worker_cancellation = cancellation.clone();
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let worker_waker = waker.clone();
        let worker = if reserve_worker() {
            match thread::Builder::new()
                .name("wayexpand-connect".into())
                .spawn(move || {
                    let _guard = WorkerGuard;
                    let response = connect(worker_cancellation);
                    worker_finished.store(true, Ordering::Release);
                    let _ = sender.send(response);
                    worker_waker.wake();
                }) {
                Ok(worker) => Some(worker),
                Err(error) => {
                    // Spawn failure releases the reservation locally and is
                    // surfaced as a retryable result instead of a detached,
                    // permanently disconnected receiver.
                    counters().active.fetch_sub(1, Ordering::AcqRel);
                    counters().completed_total.fetch_add(1, Ordering::Relaxed);
                    let _ = fallback_sender.send(Err(OutputConnectError {
                        message: format!("could not start output connection worker: {error}"),
                        retryable: true,
                    }));
                    waker.wake();
                    None
                }
            }
        } else {
            let _ = sender.send(Err(OutputConnectError {
                message: "maximum concurrent output connection attempts reached".into(),
                retryable: true,
            }));
            waker.wake();
            None
        };
        Self {
            result,
            cancellation,
            finished,
            worker,
        }
    }

    /// The attempt's result once it has finished, without waiting.
    pub(crate) fn poll(&self) -> Option<ConnectResult> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(OutputConnectError {
                message: "output connection worker ended without a result".into(),
                retryable: true,
            })),
        }
    }
}

impl Drop for BackgroundConnect {
    fn drop(&mut self) {
        if self.worker.is_some()
            && !self.finished.load(Ordering::Acquire)
            && !self.cancellation.is_cancelled()
        {
            counters().cancelled_total.fetch_add(1, Ordering::Relaxed);
            self.cancellation.cancel();
        }
        // Joining is safe only once the worker has already exited. A portal
        // may still be inside a bounded but slow call during cancellation;
        // hand that handle to the one bounded reaper instead of blocking the
        // reactor or detaching the worker.
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            } else if let Some(reaper) = connection_worker_reaper() {
                if let Err(error) = reaper.try_send(worker) {
                    let worker = match error {
                        TrySendError::Full(worker) | TrySendError::Disconnected(worker) => worker,
                    };
                    tracing::error!(
                        active_workers = counters().active.load(Ordering::Relaxed),
                        "connection reaper queue unavailable; worker remains bounded by the global connection cap"
                    );
                    drop(worker);
                }
            } else {
                tracing::error!(
                    active_workers = counters().active.load(Ordering::Relaxed),
                    "connection worker could not be joined; it remains bounded by the global connection cap"
                );
                drop(worker);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn wait_for(connect: &BackgroundConnect) -> ConnectResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = connect.poll() {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "connection attempt never finished"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_slow_connection_does_not_block_the_caller_and_wakes_it() {
        let _serial = TEST_LOCK.lock().unwrap();
        let waker = Waker::new().unwrap();
        let (release, gate) = mpsc::channel::<()>();
        let connect = BackgroundConnect::spawn(waker.clone(), move |_| {
            let _ = gate.recv();
            Err(OutputConnectError {
                message: "portal timed out".into(),
                retryable: false,
            })
        });
        // Still in flight: the caller gets control back immediately.
        assert!(connect.poll().is_none());
        release.send(()).unwrap();
        let error = match wait_for(&connect) {
            Err(error) => error,
            Ok(_) => panic!("the attempt was set up to fail"),
        };
        assert!(!error.retryable);
        assert_eq!(error.message, "portal timed out");
    }

    #[test]
    fn a_failed_backend_name_is_reported_through_poll() {
        let _serial = TEST_LOCK.lock().unwrap();
        let connect = BackgroundConnect::start(
            ConnectRequest {
                backend: "unknown",
                persist_portal_token: false,
                fcitx5_direct_commit: false,
                portal_token_path: None,
            },
            Waker::new().unwrap(),
        );
        assert!(matches!(wait_for(&connect), Err(error) if !error.retryable));
    }

    #[test]
    fn abandoned_attempt_drops_a_late_connection_instead_of_installing_it() {
        let _serial = TEST_LOCK.lock().unwrap();
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        struct DropTrackedInjector(Arc<AtomicBool>);

        impl TextInjector for DropTrackedInjector {
            fn name(&self) -> &'static str {
                "drop-tracked-test"
            }

            fn erase(&mut self, _trigger: &str) -> Result<(), wayexpand_core::InjectorError> {
                Ok(())
            }

            fn insert(&mut self, _text: &str) -> Result<(), wayexpand_core::InjectorError> {
                Ok(())
            }
        }

        impl Drop for DropTrackedInjector {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let (release, blocked) = mpsc::channel::<()>();
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_drop = std::sync::Arc::clone(&dropped);
        let connect = BackgroundConnect::spawn(Waker::new().unwrap(), move |_| {
            blocked.recv().unwrap();
            Ok(Box::new(DropTrackedInjector(worker_drop)) as Box<dyn TextInjector>)
        });

        assert!(connect.poll().is_none());
        drop(connect);
        release.send(()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while !dropped.load(std::sync::atomic::Ordering::Acquire) {
            assert!(Instant::now() < deadline, "late injector was not dropped");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn stalled_reconnects_have_a_hard_worker_limit_and_return_to_baseline() {
        let _serial = TEST_LOCK.lock().unwrap();
        let baseline = worker_stats();
        let mut releases = Vec::new();
        let mut attempts = Vec::new();
        for _ in 0..MAX_CONNECT_WORKERS {
            let (release, wait) = mpsc::channel::<()>();
            releases.push(release);
            attempts.push(BackgroundConnect::spawn(Waker::new().unwrap(), move |_| {
                let _ = wait.recv();
                Err(OutputConnectError {
                    message: "released test worker".into(),
                    retryable: true,
                })
            }));
        }
        assert_eq!(worker_stats().active, baseline.active + MAX_CONNECT_WORKERS);

        let excess = BackgroundConnect::spawn(Waker::new().unwrap(), |_| {
            panic!("connection closure must not run beyond the concurrency cap")
        });
        assert!(matches!(
            excess.poll(),
            Some(Err(OutputConnectError {
                retryable: true,
                ..
            }))
        ));
        assert_eq!(worker_stats().active, baseline.active + MAX_CONNECT_WORKERS);

        for release in releases {
            release.send(()).unwrap();
        }
        for attempt in &attempts {
            assert!(matches!(wait_for(attempt), Err(error) if error.retryable));
        }
        drop(attempts);
        assert_eq!(worker_stats().active, baseline.active);
        assert!(worker_stats().rejected_total > baseline.rejected_total);
    }

    #[test]
    fn dropping_an_attempt_signals_cancellation_to_its_worker() {
        let _serial = TEST_LOCK.lock().unwrap();
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        let baseline = worker_stats();
        let observed = Arc::new(AtomicBool::new(false));
        let worker_observed = Arc::clone(&observed);
        let connect = BackgroundConnect::spawn(Waker::new().unwrap(), move |cancellation| {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !cancellation.is_cancelled() {
                assert!(
                    Instant::now() < deadline,
                    "worker did not receive cancellation"
                );
                thread::sleep(Duration::from_millis(5));
            }
            worker_observed.store(true, Ordering::Release);
            Err(OutputConnectError {
                message: "cancelled".into(),
                retryable: true,
            })
        });

        drop(connect);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !observed.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "cancelled worker did not finish");
            thread::sleep(Duration::from_millis(5));
        }
        while worker_stats().active != baseline.active {
            assert!(
                Instant::now() < deadline,
                "worker lifecycle counter did not return to baseline"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert!(worker_stats().cancelled_total > baseline.cancelled_total);
    }
}
