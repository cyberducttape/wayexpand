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
        mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError},
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

/// Fixed allowance for one backend operation to be acknowledged, on top of
/// any paced typing time. The daemon reactor waits for the acknowledgement,
/// so a wedged portal or compositor must not stall it indefinitely.
const OUTPUT_COMPLETION_BASE: Duration = Duration::from_secs(5);

/// How long to wait for one operation that types `chars` characters. Paced
/// modes (libei keysym fallback) get twice their advertised typing time.
fn completion_deadline(capabilities: &InjectorCapabilities, chars: usize) -> Duration {
    let paced = capabilities
        .expected_throughput_chars_per_sec
        .filter(|rate| *rate > 0)
        .map_or(Duration::ZERO, |rate| {
            Duration::from_millis((chars as u64).saturating_mul(2_000) / u64::from(rate))
        });
    OUTPUT_COMPLETION_BASE.saturating_add(paced)
}

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
    Replace {
        trigger: String,
        text: String,
        completion: Completion,
    },
    Erase(String, Completion),
    Insert(String, Completion),
    MoveCursor(usize, Completion),
    Key(u32, Completion),
    KeyWithModifiers(u32, Modifiers, Completion),
    KeyEvent(u32, Modifiers, KeyEventState, Completion),
    Shutdown,
}

/// A serialized TextInjector actor. Each operation carries a completion
/// acknowledgement: queue admission is never reported as injection success.
/// This preserves the engine's transaction and undo invariants when a portal
/// or backend fails after the command has been submitted.
struct AsyncInjector {
    sender: SyncSender<OutputCommand>,
    failures: Option<thread::JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    failure_sender: SyncSender<OutputFailure>,
    capabilities: InjectorCapabilities,
    name: &'static str,
    status_detail: &'static str,
}

type Completion = SyncSender<Result<(), InjectorError>>;

impl AsyncInjector {
    /// Submit one operation and wait for the worker to acknowledge it.
    /// `chars` is the number of characters the backend has to type, used to
    /// size the deadline for paced modes.
    fn submit(
        &self,
        chars: usize,
        command: impl FnOnce(Completion) -> OutputCommand,
    ) -> Result<(), InjectorError> {
        let (completion_sender, completion) = sync_channel(1);
        enqueue_command(&self.sender, command(completion_sender), self.name)?;
        let deadline = completion_deadline(&self.capabilities, chars);
        match completion.recv_timeout(deadline) {
            Ok(result) => result,
            Err(RecvTimeoutError::Disconnected) => Err(InjectorError {
                backend: self.name,
                message: "serialized output worker stopped before acknowledging operation".into(),
                retryable: true,
            }),
            Err(RecvTimeoutError::Timeout) => {
                // The operation may still complete later, so its outcome is
                // unknown. Stop the worker from taking further work and route
                // the daemon through normal reconnect recovery.
                self.cancel.store(true, Ordering::Release);
                let message = format!(
                    "output backend did not acknowledge the operation within {}ms",
                    deadline.as_millis()
                );
                let _ = self.failure_sender.try_send(OutputFailure {
                    message: message.clone(),
                    retryable: true,
                });
                Err(InjectorError {
                    backend: self.name,
                    message,
                    retryable: true,
                })
            }
        }
    }
}

fn enqueue_command(
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
) -> std::result::Result<(Box<dyn TextInjector>, Receiver<OutputFailure>), OutputConnectError> {
    let capabilities = backend.capabilities();
    let name = backend.name();
    let status_detail = backend.status_detail();
    let (sender, receiver) = sync_channel(ASYNC_OUTPUT_QUEUE_CAPACITY);
    let (failure_sender, failure_receiver) = sync_channel(1);
    let worker_failure_sender = failure_sender.clone();
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
                let (result, completion) = match command {
                    OutputCommand::Replace {
                        trigger,
                        text,
                        completion,
                    } => (backend.replace(&trigger, &text), completion),
                    OutputCommand::Erase(text, completion) => (backend.erase(&text), completion),
                    OutputCommand::Insert(text, completion) => (backend.insert(&text), completion),
                    OutputCommand::MoveCursor(count, completion) => {
                        (backend.move_cursor_left(count), completion)
                    }
                    OutputCommand::Key(keycode, completion) => {
                        (backend.inject_key(keycode), completion)
                    }
                    OutputCommand::KeyWithModifiers(keycode, modifiers, completion) => (
                        backend.inject_key_with_modifiers(keycode, modifiers),
                        completion,
                    ),
                    OutputCommand::KeyEvent(keycode, modifiers, state, completion) => (
                        backend.inject_key_event(keycode, modifiers, state),
                        completion,
                    ),
                    OutputCommand::Shutdown => break,
                };
                let failed = if let Err(error) = &result {
                    let _ = worker_failure_sender.try_send(OutputFailure {
                        message: error.message.clone(),
                        retryable: error.retryable,
                    });
                    true
                } else {
                    false
                };
                let _ = completion.send(result);
                if failed {
                    break;
                }
            }
            backend.shutdown();
        })
        .map_err(|error| OutputConnectError {
            message: format!("could not start serialized output worker: {error}"),
            retryable: true,
        })?;
    let injector = AsyncInjector {
        sender,
        failures: Some(worker),
        cancel,
        failure_sender,
        capabilities,
        name,
        status_detail,
    };
    Ok((Box::new(injector), failure_receiver))
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
        self.submit(trigger.chars().count(), |completion| {
            OutputCommand::Erase(trigger.into(), completion)
        })
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.submit(text.chars().count(), |completion| {
            OutputCommand::Insert(text.into(), completion)
        })
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        let chars = trigger.chars().count() + text.chars().count();
        self.submit(chars, |completion| OutputCommand::Replace {
            trigger: trigger.into(),
            text: text.into(),
            completion,
        })
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        self.submit(count, |completion| {
            OutputCommand::MoveCursor(count, completion)
        })
    }

    fn inject_key(&mut self, keycode: u32) -> Result<(), InjectorError> {
        self.submit(1, |completion| OutputCommand::Key(keycode, completion))
    }

    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        self.submit(1, |completion| {
            OutputCommand::KeyWithModifiers(keycode, modifiers, completion)
        })
    }

    fn inject_key_event(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        self.submit(1, |completion| {
            OutputCommand::KeyEvent(keycode, modifiers, state, completion)
        })
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

    struct FailingInjector;

    impl TextInjector for FailingInjector {
        fn name(&self) -> &'static str {
            "failing-test"
        }

        fn replace(&mut self, _: &str, _: &str) -> Result<(), InjectorError> {
            Err(InjectorError {
                backend: self.name(),
                message: "simulated backend failure".into(),
                retryable: true,
            })
        }

        fn erase(&mut self, _: &str) -> Result<(), InjectorError> {
            Ok(())
        }

        fn insert(&mut self, _: &str) -> Result<(), InjectorError> {
            Ok(())
        }
    }

    #[test]
    fn serialized_output_waits_for_backend_completion() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = SlowInjector {
            calls: Arc::clone(&calls),
        };
        let (mut injector, failures) = spawn_async_injector(Box::new(backend)).unwrap();
        let started = std::time::Instant::now();
        injector.replace(":a", "replacement").unwrap();
        assert!(started.elapsed() >= Duration::from_millis(70));
        injector.replace(":b", "second").unwrap();
        injector.shutdown();

        assert!(failures.try_recv().is_err());
        assert_eq!(
            *calls.lock().unwrap(),
            vec![":a->replacement", ":b->second"]
        );
    }

    #[test]
    fn completion_deadline_allows_paced_typing_time() {
        let unpaced = InjectorCapabilities::default();
        assert_eq!(
            completion_deadline(&unpaced, 10_000),
            OUTPUT_COMPLETION_BASE
        );

        let paced = InjectorCapabilities {
            expected_throughput_chars_per_sec: Some(83),
            ..InjectorCapabilities::default()
        };
        // 250 characters at 83/s is ~3s of typing; allow twice that.
        let deadline = completion_deadline(&paced, 250);
        assert!(deadline >= OUTPUT_COMPLETION_BASE + Duration::from_secs(6));
        assert!(deadline <= OUTPUT_COMPLETION_BASE + Duration::from_secs(7));

        let zero_rate = InjectorCapabilities {
            expected_throughput_chars_per_sec: Some(0),
            ..InjectorCapabilities::default()
        };
        assert_eq!(completion_deadline(&zero_rate, 250), OUTPUT_COMPLETION_BASE);
    }

    #[test]
    fn backend_failure_is_returned_to_the_transaction_caller() {
        let (mut injector, failures) = spawn_async_injector(Box::new(FailingInjector)).unwrap();
        let error = injector
            .replace(":a", "replacement")
            .expect_err("backend failure must not be reported as queue admission success");
        assert_eq!(error.message, "simulated backend failure");

        let failure = failures
            .recv_timeout(Duration::from_secs(1))
            .expect("worker failure should be reported");
        assert_eq!(failure.message, "simulated backend failure");
        assert!(failure.retryable);
    }
}
