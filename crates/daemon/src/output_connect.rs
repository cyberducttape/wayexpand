//! Output backend connections made off the reactor thread.
//!
//! Connecting libei through the RemoteDesktop portal can take up to the
//! portal's 30-second deadline. A reconnect made on the reactor would stop
//! it from handling stop requests, reloads, inserts, and status for that
//! long, so the reactor starts the attempt here and collects the result on a
//! later turn; the worker wakes it when the attempt ends.

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use wayexpand_core::TextInjector;

use crate::output_loop::{connect_output_backend_with_cancellation, OutputConnectError};
use crate::waker::Waker;

pub(crate) type ConnectResult = Result<Box<dyn TextInjector>, OutputConnectError>;

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
        let cancellation = wayexpand_backend_libei::ConnectCancellation::default();
        let worker_cancellation = cancellation.clone();
        let spawned = thread::Builder::new()
            .name("wayexpand-connect".into())
            .spawn(move || {
                let _ = sender.send(connect(worker_cancellation));
                waker.wake();
            });
        if let Err(error) = spawned {
            // The closure (and its sender) is gone, so poll() reports this
            // as a retryable failure on the next turn.
            tracing::warn!(%error, "could not start output connection worker");
        }
        Self {
            result,
            cancellation,
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
        self.cancellation.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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
    fn dropping_an_attempt_signals_cancellation_to_its_worker() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

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
    }
}
