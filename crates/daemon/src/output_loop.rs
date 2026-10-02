//! Output backend connection and reconnection logic.
//!
//! Handles:
//! - Wlroots and libei output backend selection
//! - Portal token persistence for libei sandboxed access
//! - Exponential backoff retry logic
//! - Transient vs permanent error handling

use anyhow::Result;
use std::path::Path;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread,
    time::Duration,
};
use tracing::{info, warn};
use wayexpand_backend_libei::{LibeiInjector, LibeiOptions};
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{InjectorCapabilities, InjectorError, KeyEventState, Modifiers, TextInjector};

use crate::{control, input_loop::wait_for_retry, status};

/// Error connecting to an output backend.
#[derive(Debug)]
pub struct OutputConnectError {
    pub message: String,
    pub retryable: bool,
}

const ASYNC_OUTPUT_QUEUE_CAPACITY: usize = 64;

/// A failure reported by the serialized output actor after an operation was
/// accepted into its queue. The daemon must reconnect rather than replay the
/// operation because a non-atomic injector may have applied part of it.
#[derive(Debug)]
pub struct OutputFailure {
    pub message: String,
    pub retryable: bool,
}

/// Shut down an injector without allowing a broken portal implementation to
/// hold up daemon termination indefinitely. Keeping this policy next to the
/// output actor makes all backend shutdown paths use the same deadline.
pub fn shutdown_injector(injector: Box<dyn TextInjector>) {
    let backend = injector.name();
    let (finished_sender, finished_receiver) = std::sync::mpsc::sync_channel(1);
    info!(
        backend,
        event = "injector_shutdown_started",
        "starting bounded backend shutdown"
    );
    let spawn = thread::Builder::new()
        .name("wayexpand-injector-shutdown".into())
        .spawn(move || {
            injector.shutdown();
            let _ = finished_sender.send(());
        });
    match spawn {
        Ok(_) => match finished_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(()) => info!(
                backend,
                event = "injector_shutdown_finished",
                "backend shutdown completed"
            ),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => warn!(
                backend,
                event = "injector_shutdown_deadline_exceeded",
                "backend shutdown exceeded its deadline; leaving the worker detached"
            ),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => warn!(
                backend,
                event = "injector_shutdown_worker_failed",
                "backend shutdown worker exited without completion"
            ),
        },
        Err(error) => warn!(
            backend,
            %error,
            event = "injector_shutdown_spawn_failed",
            "could not start backend shutdown worker; backend will be reclaimed at process exit"
        ),
    }
}

enum OutputCommand {
    Replace { trigger: String, text: String },
    Erase(String),
    Insert(String),
    MoveCursor(usize),
    Key(u32),
    KeyWithModifiers(u32, Modifiers),
    KeyEvent(u32, Modifiers, KeyEventState),
    Shutdown,
}

/// A TextInjector facade that admits complete operations without waiting for
/// the backend to perform them. The worker is deliberately serialized so
/// physical input can keep reaching the matcher while a paced libei keysym
/// fallback types a replacement.
struct AsyncInjector {
    sender: SyncSender<OutputCommand>,
    failures: Option<thread::JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    capabilities: InjectorCapabilities,
    name: &'static str,
    status_detail: &'static str,
}

fn enqueue(
    sender: &SyncSender<OutputCommand>,
    command: OutputCommand,
    name: &'static str,
) -> Result<(), InjectorError> {
    sender.try_send(command).map_err(|error| InjectorError {
        backend: name,
        message: match error {
            TrySendError::Full(_) => "serialized output queue is full".into(),
            TrySendError::Disconnected(_) => "serialized output worker stopped".into(),
        },
        retryable: true,
    })
}

/// Move a connected output backend behind a bounded actor. The receiver is
/// drained by the daemon reactor so worker failures trigger normal recovery.
pub fn spawn_async_injector(
    backend: Box<dyn TextInjector>,
) -> (Box<dyn TextInjector>, Receiver<OutputFailure>) {
    let capabilities = backend.capabilities();
    let name = backend.name();
    let status_detail = backend.status_detail();
    let (sender, receiver) = sync_channel(ASYNC_OUTPUT_QUEUE_CAPACITY);
    let (failure_sender, failure_receiver) = sync_channel(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let worker = thread::Builder::new()
        .name("wayexpand-output".into())
        .spawn(move || {
            let mut backend = backend;
            while let Ok(command) = receiver.recv() {
                if worker_cancel.load(Ordering::Acquire) {
                    break;
                }
                let result = match command {
                    OutputCommand::Replace { trigger, text } => backend.replace(&trigger, &text),
                    OutputCommand::Erase(text) => backend.erase(&text),
                    OutputCommand::Insert(text) => backend.insert(&text),
                    OutputCommand::MoveCursor(count) => backend.move_cursor_left(count),
                    OutputCommand::Key(keycode) => backend.inject_key(keycode),
                    OutputCommand::KeyWithModifiers(keycode, modifiers) => {
                        backend.inject_key_with_modifiers(keycode, modifiers)
                    }
                    OutputCommand::KeyEvent(keycode, modifiers, state) => {
                        backend.inject_key_event(keycode, modifiers, state)
                    }
                    OutputCommand::Shutdown => break,
                };
                if let Err(error) = result {
                    let _ = failure_sender.send(OutputFailure {
                        message: error.message,
                        retryable: error.retryable,
                    });
                    break;
                }
            }
            backend.shutdown();
        })
        .expect("output worker thread must start");
    let injector = AsyncInjector {
        sender,
        failures: Some(worker),
        cancel,
        capabilities,
        name,
        status_detail,
    };
    (Box::new(injector), failure_receiver)
}

impl TextInjector for AsyncInjector {
    fn shutdown(mut self: Box<Self>) {
        let _ = self.sender.send(OutputCommand::Shutdown);
        if let Some(worker) = self.failures.take() {
            let _ = worker.join();
        }
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn capabilities(&self) -> InjectorCapabilities {
        self.capabilities
    }

    fn status_detail(&self) -> &'static str {
        self.status_detail
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        enqueue(
            &self.sender,
            OutputCommand::Erase(trigger.into()),
            self.name,
        )
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        enqueue(&self.sender, OutputCommand::Insert(text.into()), self.name)
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        enqueue(
            &self.sender,
            OutputCommand::Replace {
                trigger: trigger.into(),
                text: text.into(),
            },
            self.name,
        )
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        enqueue(&self.sender, OutputCommand::MoveCursor(count), self.name)
    }

    fn inject_key(&mut self, keycode: u32) -> Result<(), InjectorError> {
        enqueue(&self.sender, OutputCommand::Key(keycode), self.name)
    }

    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        enqueue(
            &self.sender,
            OutputCommand::KeyWithModifiers(keycode, modifiers),
            self.name,
        )
    }

    fn inject_key_event(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        enqueue(
            &self.sender,
            OutputCommand::KeyEvent(keycode, modifiers, state),
            self.name,
        )
    }
}

impl Drop for AsyncInjector {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl std::fmt::Display for OutputConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OutputConnectError {}

/// Connect to the specified output backend.
pub fn connect_output_backend(
    name: &str,
    persist_portal_token: bool,
    portal_token_path_arg: Option<&Path>,
) -> std::result::Result<Box<dyn TextInjector>, OutputConnectError> {
    match name {
        "wlroots" => WlrootsInjector::connect()
            .map(|injector| Box::new(injector) as Box<dyn TextInjector>)
            .map_err(|error| OutputConnectError {
                retryable: error.is_retryable(),
                message: format!("connecting wlroots output backend: {error}"),
            }),
        "libei" => LibeiInjector::connect(LibeiOptions {
            persist_portal_token,
            portal_token_path: portal_token_path_arg.map(Path::to_path_buf),
        })
        .map(|injector| Box::new(injector) as Box<dyn TextInjector>)
        .map_err(|error| OutputConnectError {
            retryable: error.is_retryable(),
            message: format!("connecting libei output backend: {error}"),
        }),
        other => Err(OutputConnectError {
            retryable: false,
            message: format!("unknown output backend {other:?}"),
        }),
    }
}

/// Connect to an output backend with exponential backoff retry.
pub fn connect_output_with_retry(
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    config_path: &Path,
    config_healthy: bool,
    persist_portal_token: bool,
    portal_token_path_arg: Option<&Path>,
) -> Result<Option<Box<dyn TextInjector>>> {
    let mut retry_delay = Duration::from_millis(250);
    loop {
        match connect_output_backend(backend, persist_portal_token, portal_token_path_arg) {
            Ok(injector) => {
                status::set_daemon_status_with_mode(
                    control,
                    source,
                    backend,
                    "connected",
                    config_path,
                    config_healthy,
                    if injector.status_detail().is_empty() {
                        "unknown"
                    } else {
                        injector.status_detail()
                    },
                    wayexpand_core::CommandMetrics::default(),
                );
                info!(backend, "output backend reconnected");
                return Ok(Some(injector));
            }
            Err(error) if error.retryable => {
                warn!(%error, backend, ?retry_delay, "output backend unavailable; retrying");
                status::set_daemon_status_direct(
                    control,
                    source,
                    backend,
                    "reconnecting",
                    config_path,
                    config_healthy,
                );
                if !wait_for_retry(&control.stop_requested, retry_delay) {
                    return Ok(None);
                }
                retry_delay = next_retry_delay(retry_delay);
            }
            Err(error) => return Err(anyhow::Error::new(error)),
        }
    }
}

/// Calculate next retry delay with exponential backoff (max 30s).
fn next_retry_delay(delay: Duration) -> Duration {
    delay.saturating_mul(2).min(Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct SlowInjector {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl TextInjector for SlowInjector {
        fn name(&self) -> &'static str {
            "slow-test"
        }

        fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
            std::thread::sleep(Duration::from_millis(75));
            self.calls
                .lock()
                .unwrap()
                .push(format!("{trigger}->{text}"));
            Ok(())
        }

        fn erase(&mut self, _: &str) -> Result<(), InjectorError> {
            Ok(())
        }

        fn insert(&mut self, _: &str) -> Result<(), InjectorError> {
            Ok(())
        }
    }

    #[test]
    fn serialized_output_admits_work_without_waiting_for_injection() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = SlowInjector {
            calls: Arc::clone(&calls),
        };
        let (mut injector, failures) = spawn_async_injector(Box::new(backend));
        let started = std::time::Instant::now();
        injector.replace(":a", "replacement").unwrap();
        assert!(started.elapsed() < Duration::from_millis(50));
        injector.replace(":b", "second").unwrap();
        injector.shutdown();

        assert!(failures.try_recv().is_err());
        assert_eq!(
            *calls.lock().unwrap(),
            vec![":a->replacement", ":b->second"]
        );
    }
}
