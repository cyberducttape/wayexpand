use super::{
    CommandError, CommandMetricsState, ExpansionEngine, ExpansionResult, HotkeyError, HotkeyResult,
};
use crate::CommandConfig;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Condvar, Mutex, RwLock,
    },
    thread,
    thread::JoinHandle,
    time::Duration,
};

/// Wakeup callback for asynchronous expansion completions. It runs on a
/// worker thread and must only signal the host; draining happens on the host
/// through [`super::ExpansionEngine::drain_completed_commands`].
pub type CompletionNotifier = Arc<dyn Fn() + Send + Sync>;

pub(super) fn notify_completion(notifier: &std::sync::RwLock<Option<CompletionNotifier>>) {
    let notifier = match notifier.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    if let Some(notifier) = notifier {
        notifier();
    }
}

/// Bounded multi-consumer command queue. A condition variable releases the
/// queue mutex while workers wait, unlike a mutex-protected std MPSC
/// receiver's `recv_timeout`, which makes idle workers take turns holding the
/// receiver lock.
pub(super) struct CommandQueue<T> {
    items: Mutex<VecDeque<T>>,
    wake: Condvar,
    capacity: usize,
}

impl<T> CommandQueue<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            items: Mutex::new(VecDeque::with_capacity(capacity)),
            wake: Condvar::new(),
            capacity,
        }
    }

    fn try_send(&self, item: T) -> Result<(), T> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if items.len() >= self.capacity {
            return Err(item);
        }
        items.push_back(item);
        self.wake.notify_one();
        Ok(())
    }

    fn recv_timeout(&self, timeout: Duration) -> Option<T> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(item) = items.pop_front() {
            return Some(item);
        }
        let (mut items, _) = self
            .wake
            .wait_timeout(items, timeout)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        items.pop_front()
    }

    fn notify_all(&self) {
        self.wake.notify_all();
    }
}

pub(super) struct AsyncCommandRuntime {
    pub(super) command_sender: Arc<CommandQueue<AsyncCommandJob>>,
    pub(super) hotkey_sender: mpsc::SyncSender<HotkeyResult>,
    pub(super) receiver: mpsc::Receiver<AsyncCommandCompletion>,
    pub(super) hotkey_receiver: mpsc::Receiver<AsyncHotkeyCompletion>,
    pub(super) expansion_metrics: Arc<CommandMetricsState>,
    pub(super) hotkey_metrics: Arc<CommandMetricsState>,
    pub(super) shutdown: Arc<AtomicBool>,
    pub(super) command_workers: Vec<JoinHandle<()>>,
    pub(super) hotkey_worker: Option<JoinHandle<()>>,
}

/// Send a worker completion without making runtime shutdown dependent on a
/// receiver draining the old configuration's queue. Normal operation keeps
/// backpressure when the bounded completion channel is full; once shutdown is
/// requested, the completion can be discarded because the owning engine is
/// being replaced and must not wait indefinitely for stale output.
pub(super) fn send_completion_or_shutdown<T>(
    sender: &mpsc::SyncSender<T>,
    mut completion: T,
    shutdown: &AtomicBool,
) -> bool {
    loop {
        if shutdown.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(completion) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            Err(mpsc::TrySendError::Full(value)) => {
                completion = value;
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

impl Drop for AsyncCommandRuntime {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.command_sender.notify_all();
        for worker in self.command_workers.drain(..) {
            let _ = worker.join();
        }
        if let Some(worker) = self.hotkey_worker.take() {
            let _ = worker.join();
        }
    }
}

impl AsyncCommandRuntime {
    pub(super) fn start(
        shared_input_generation: Arc<AtomicU64>,
        completion_notifier: Arc<RwLock<Option<CompletionNotifier>>>,
        expansion_metrics: Arc<CommandMetricsState>,
        hotkey_metrics: Arc<CommandMetricsState>,
    ) -> Option<Self> {
        let command_sender = Arc::new(CommandQueue::new(super::ASYNC_COMMAND_QUEUE_CAPACITY));
        let (hotkey_sender, hotkey_receiver) =
            mpsc::sync_channel::<HotkeyResult>(super::ASYNC_COMMAND_QUEUE_CAPACITY);
        let (completion_sender, completion_receiver) =
            mpsc::sync_channel(super::ASYNC_COMMAND_QUEUE_CAPACITY);
        let (hotkey_completion_sender, hotkey_completion_receiver) =
            mpsc::sync_channel(super::ASYNC_COMMAND_QUEUE_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut command_workers = Vec::with_capacity(super::ASYNC_COMMAND_WORKER_COUNT);
        for worker_index in 0..super::ASYNC_COMMAND_WORKER_COUNT {
            let command_shutdown = Arc::clone(&shutdown);
            let worker_metrics = Arc::clone(&expansion_metrics);
            let worker_receiver = Arc::clone(&command_sender);
            let worker_completion_sender = completion_sender.clone();
            let worker_input_generation = Arc::clone(&shared_input_generation);
            let worker_notifier = Arc::clone(&completion_notifier);
            let command_worker = thread::Builder::new()
                .name(format!("wayexpand-expansion-worker-{worker_index}"))
                .spawn(move || {
                    while !command_shutdown.load(Ordering::Acquire) {
                        let Some(job) = worker_receiver.recv_timeout(Duration::from_millis(50))
                        else {
                            continue;
                        };
                        if command_shutdown.load(Ordering::Acquire) {
                            worker_metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
                            break;
                        }
                        worker_metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
                        let (config_index, generation, additional_max_size, command, result) =
                            match job {
                                AsyncCommandJob::Expansion {
                                    config_index,
                                    generation,
                                    additional_max_size,
                                    command,
                                    result,
                                } => (
                                    config_index,
                                    generation,
                                    additional_max_size,
                                    command,
                                    result,
                                ),
                                AsyncCommandJob::Form {
                                    config_index,
                                    additional_max_size,
                                    template,
                                    fields,
                                    mut context,
                                    title,
                                    origin,
                                    mut result,
                                } => {
                                    worker_metrics.in_flight.fetch_add(1, Ordering::Relaxed);
                                    let output = match super::command_runtime::run_form_helper(
                                        &title,
                                        &fields,
                                        &command_shutdown,
                                    ) {
                                        Ok(values) => {
                                            context.fields = Arc::new(values);
                                            match crate::render_template_with_cursor(
                                                &template, &context,
                                            ) {
                                                Ok((text, cursor_offset)) => {
                                                    result.cursor_offset = cursor_offset;
                                                    Ok(text)
                                                }
                                                Err(_) => Err(CommandError::IncompleteOutput),
                                            }
                                        }
                                        Err(error) => Err(error),
                                    };
                                    worker_metrics.in_flight.fetch_sub(1, Ordering::Relaxed);
                                    if !send_completion_or_shutdown(
                                        &worker_completion_sender,
                                        AsyncCommandCompletion {
                                            config_index,
                                            generation: 0,
                                            cache_ms: 0,
                                            additional_max_size,
                                            result,
                                            output,
                                            form: Some(origin),
                                        },
                                        &command_shutdown,
                                    ) {
                                        break;
                                    }
                                    notify_completion(&worker_notifier);
                                    continue;
                                }
                            };
                        let cache_ms = command.cache_ms;
                        if worker_input_generation.load(Ordering::Acquire) != generation {
                            if !send_completion_or_shutdown(
                                &worker_completion_sender,
                                AsyncCommandCompletion {
                                    config_index,
                                    generation,
                                    cache_ms,
                                    additional_max_size,
                                    result,
                                    output: Err(CommandError::StaleInput),
                                    form: None,
                                },
                                &command_shutdown,
                            ) {
                                break;
                            }
                            notify_completion(&worker_notifier);
                            continue;
                        }
                        worker_metrics.in_flight.fetch_add(1, Ordering::Relaxed);
                        let output = super::command_runtime::run_command_with_shutdown(
                            &command,
                            Some(&command_shutdown),
                        );
                        worker_metrics.in_flight.fetch_sub(1, Ordering::Relaxed);
                        if let Err(error) = &output {
                            worker_metrics.record_error(matches!(error, CommandError::Timeout));
                        }
                        if !send_completion_or_shutdown(
                            &worker_completion_sender,
                            AsyncCommandCompletion {
                                config_index,
                                generation,
                                cache_ms,
                                additional_max_size,
                                result,
                                output,
                                form: None,
                            },
                            &command_shutdown,
                        ) {
                            break;
                        }
                        notify_completion(&worker_notifier);
                    }
                });
            match command_worker {
                Ok(worker) => command_workers.push(worker),
                Err(_) => {
                    shutdown.store(true, Ordering::Release);
                    command_sender.notify_all();
                    for worker in command_workers {
                        let _ = worker.join();
                    }
                    return None;
                }
            }
        }
        let hotkey_shutdown = Arc::clone(&shutdown);
        let worker_hotkey_metrics = Arc::clone(&hotkey_metrics);
        let hotkey_worker = thread::Builder::new()
            .name("wayexpand-hotkey-worker".into())
            .spawn(move || {
                while !hotkey_shutdown.load(Ordering::Acquire) {
                    let action = match hotkey_receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(action) => action,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    if hotkey_shutdown.load(Ordering::Acquire) {
                        worker_hotkey_metrics
                            .queue_depth
                            .fetch_sub(1, Ordering::Relaxed);
                        break;
                    }
                    worker_hotkey_metrics
                        .queue_depth
                        .fetch_sub(1, Ordering::Relaxed);
                    worker_hotkey_metrics
                        .in_flight
                        .fetch_add(1, Ordering::Relaxed);
                    let output = ExpansionEngine::execute_hotkey_with_shutdown(
                        &action,
                        Some(&hotkey_shutdown),
                    );
                    worker_hotkey_metrics
                        .in_flight
                        .fetch_sub(1, Ordering::Relaxed);
                    if let Err(error) = &output {
                        worker_hotkey_metrics
                            .record_error(matches!(error, HotkeyError::Timeout(_)));
                    }
                    if !send_completion_or_shutdown(
                        &hotkey_completion_sender,
                        AsyncHotkeyCompletion { action, output },
                        &hotkey_shutdown,
                    ) {
                        break;
                    }
                }
            });
        let hotkey_worker = match hotkey_worker {
            Ok(worker) => worker,
            Err(_) => {
                shutdown.store(true, Ordering::Release);
                command_sender.notify_all();
                for worker in command_workers {
                    let _ = worker.join();
                }
                return None;
            }
        };
        Some(Self {
            command_sender,
            hotkey_sender,
            receiver: completion_receiver,
            hotkey_receiver: hotkey_completion_receiver,
            expansion_metrics,
            hotkey_metrics,
            shutdown,
            command_workers,
            hotkey_worker: Some(hotkey_worker),
        })
    }

    pub(super) fn try_send_command(
        &self,
        job: AsyncCommandJob,
    ) -> Result<(), super::QueueSendError> {
        self.expansion_metrics
            .queue_depth
            .fetch_add(1, Ordering::Relaxed);
        match self.command_sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.expansion_metrics
                    .queue_depth
                    .fetch_sub(1, Ordering::Relaxed);
                self.expansion_metrics
                    .queue_rejected_total
                    .fetch_add(1, Ordering::Relaxed);
                Err(super::QueueSendError::Full)
            }
        }
    }

    pub(super) fn try_send_hotkey(
        &self,
        action: HotkeyResult,
    ) -> Result<(), super::QueueSendError> {
        self.try_send(&self.hotkey_sender, action, &self.hotkey_metrics)
    }

    fn try_send<T>(
        &self,
        sender: &mpsc::SyncSender<T>,
        job: T,
        metrics: &CommandMetricsState,
    ) -> Result<(), super::QueueSendError> {
        metrics.queue_depth.fetch_add(1, Ordering::Relaxed);
        match sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
                metrics.queue_rejected_total.fetch_add(1, Ordering::Relaxed);
                Err(super::QueueSendError::Full)
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
                metrics.queue_rejected_total.fetch_add(1, Ordering::Relaxed);
                Err(super::QueueSendError::Disconnected)
            }
        }
    }
}

pub(super) enum AsyncCommandJob {
    Expansion {
        config_index: usize,
        generation: u64,
        additional_max_size: usize,
        command: CommandConfig,
        result: ExpansionResult,
    },
    Form {
        config_index: usize,
        additional_max_size: usize,
        template: String,
        fields: Arc<Vec<crate::FormField>>,
        context: crate::TemplateContext,
        title: String,
        origin: FormOrigin,
        result: ExpansionResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FormOrigin {
    pub(super) app_id: Option<String>,
    pub(super) instance_id: String,
}

pub(super) struct AsyncCommandCompletion {
    pub(super) config_index: usize,
    pub(super) generation: u64,
    pub(super) cache_ms: u64,
    pub(super) additional_max_size: usize,
    pub(super) result: ExpansionResult,
    pub(super) output: Result<String, CommandError>,
    pub(super) form: Option<FormOrigin>,
}

pub(super) struct AsyncHotkeyCompletion {
    pub(super) action: HotkeyResult,
    pub(super) output: Result<(), HotkeyError>,
}

#[cfg(test)]
mod tests {
    use super::CommandQueue;
    use std::{sync::Arc, thread, time::Duration};

    #[test]
    fn command_queue_supports_multiple_consumers_without_a_receiver_lock() {
        let queue = Arc::new(CommandQueue::new(32));
        for item in 0..32 {
            queue
                .try_send(item)
                .expect("queue should accept its capacity");
        }
        let workers = (0..4)
            .map(|_| {
                let queue = Arc::clone(&queue);
                thread::spawn(move || {
                    let mut count = 0;
                    while queue.recv_timeout(Duration::from_millis(20)).is_some() {
                        count += 1;
                    }
                    count
                })
            })
            .collect::<Vec<_>>();
        let consumed: usize = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .sum();
        assert_eq!(consumed, 32);
    }
}
