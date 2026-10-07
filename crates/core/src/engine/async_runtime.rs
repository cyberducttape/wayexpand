use super::{CommandError, CommandMetricsState, ExpansionResult, HotkeyError, HotkeyResult};
use crate::CommandConfig;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
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

pub(super) struct AsyncCommandRuntime {
    pub(super) command_sender: mpsc::SyncSender<AsyncCommandJob>,
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
        for worker in self.command_workers.drain(..) {
            let _ = worker.join();
        }
        if let Some(worker) = self.hotkey_worker.take() {
            let _ = worker.join();
        }
    }
}

impl AsyncCommandRuntime {
    pub(super) fn try_send_command(
        &self,
        job: AsyncCommandJob,
    ) -> Result<(), super::QueueSendError> {
        self.try_send(&self.command_sender, job, &self.expansion_metrics)
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
