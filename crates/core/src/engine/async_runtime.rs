use super::{
    input_generation_matches, CommandError, CommandMetricsState, ExpansionEngine, ExpansionResult,
    HotkeyError, HotkeyResult,
};
use crate::{CommandConfig, CommandEnvironment, FormFieldKind};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex, OnceLock, RwLock, Weak,
    },
    thread,
    thread::JoinHandle,
    time::Duration,
};

fn form_template_uses_only_fields(template: &str) -> bool {
    crate::template_variables(template).iter().all(|name| {
        *name == "cursor"
            || ["field:", "prompt:", "choice:", "datefield:", "shellq:"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
    })
}

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

/// Bounded multi-consumer job queue. A condition variable releases the queue
/// mutex while workers wait, unlike a mutex-protected std MPSC receiver,
/// which makes idle workers take turns holding the receiver lock.
pub(super) struct CommandQueue<T> {
    items: Mutex<VecDeque<T>>,
    wake: Condvar,
    capacity: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum CommandQueueSendError<T> {
    Full(T),
    Unavailable(T),
}

impl<T> CommandQueue<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            items: Mutex::new(VecDeque::with_capacity(capacity)),
            wake: Condvar::new(),
            capacity,
        }
    }

    fn try_send(&self, item: T) -> Result<(), CommandQueueSendError<T>> {
        let Some(mut items) = self.lock_items() else {
            return Err(CommandQueueSendError::Unavailable(item));
        };
        if items.len() >= self.capacity {
            return Err(CommandQueueSendError::Full(item));
        }
        items.push_back(item);
        self.wake.notify_one();
        Ok(())
    }

    fn recv_timeout(&self, timeout: Duration) -> Option<T> {
        let mut items = self.lock_items()?;
        if let Some(item) = items.pop_front() {
            return Some(item);
        }
        let mut items = match self.wake.wait_timeout(items, timeout) {
            Ok((items, _)) => items,
            Err(poisoned) => {
                let (items, _) = poisoned.into_inner();
                self.recover_poisoned(items)?
            }
        };
        items.pop_front()
    }

    fn lock_items(&self) -> Option<std::sync::MutexGuard<'_, VecDeque<T>>> {
        match self.items.lock() {
            Ok(items) => Some(items),
            Err(poisoned) => self.recover_poisoned(poisoned.into_inner()),
        }
    }

    /// The queue's complete logical invariant is `len <= capacity`; FIFO and
    /// storage validity are maintained by `VecDeque`, and no user code runs
    /// while its mutex is held. Recover a poisoned lock only when its bound
    /// still holds. If it does not, keep the mutex poisoned and fail queue
    /// reads/writes closed instead of running jobs from inconsistent state.
    fn recover_poisoned<'a>(
        &self,
        items: std::sync::MutexGuard<'a, VecDeque<T>>,
    ) -> Option<std::sync::MutexGuard<'a, VecDeque<T>>> {
        if items.len() > self.capacity {
            return None;
        }
        self.items.clear_poison();
        Some(items)
    }

    fn notify_all(&self) {
        self.wake.notify_all();
    }

    fn remove_owner(&self, owner: u64)
    where
        T: PoolTaskOwner,
    {
        if let Some(mut items) = self.lock_items() {
            items.retain(|item| item.owner() != owner);
        }
        self.wake.notify_all();
    }
}

pub(super) struct AsyncCommandRuntime {
    pool: Arc<SharedWorkerPool>,
    owner: u64,
    completion_sender: mpsc::SyncSender<AsyncCommandCompletion>,
    hotkey_completion_sender: mpsc::SyncSender<AsyncHotkeyCompletion>,
    pub(super) receiver: mpsc::Receiver<AsyncCommandCompletion>,
    pub(super) hotkey_receiver: mpsc::Receiver<AsyncHotkeyCompletion>,
    pub(super) expansion_metrics: Arc<CommandMetricsState>,
    pub(super) hotkey_metrics: Arc<CommandMetricsState>,
    pub(super) shutdown: Arc<AtomicBool>,
    pub(super) command_start_gate: Arc<Mutex<()>>,
    lifecycle_gate: Mutex<()>,
    active_jobs: Arc<ActiveJobs>,
    command_queue_slots: Arc<AtomicUsize>,
    hotkey_queue_slots: Arc<AtomicUsize>,
    shared_input_generation: Arc<AtomicU64>,
    completion_notifier: Arc<RwLock<Option<CompletionNotifier>>>,
}

type PoolTask = Box<dyn FnOnce(QueueSlotGuard) + Send + 'static>;

struct QueuedPoolTask {
    owner: u64,
    run: Option<PoolTask>,
    _slot: QueueSlotGuard,
}

trait PoolTaskOwner {
    fn owner(&self) -> u64;
}

impl PoolTaskOwner for QueuedPoolTask {
    fn owner(&self) -> u64 {
        self.owner
    }
}

struct QueueSlotGuard {
    metrics: Arc<CommandMetricsState>,
    slots: Arc<AtomicUsize>,
    queued: bool,
}

impl QueueSlotGuard {
    fn started(&mut self) {
        if self.queued {
            self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            self.queued = false;
        }
    }
}

impl Drop for QueueSlotGuard {
    fn drop(&mut self) {
        if self.queued {
            self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
        }
        self.slots.fetch_sub(1, Ordering::AcqRel);
    }
}

impl QueuedPoolTask {
    fn run(self) {
        let Self { run, _slot, .. } = self;
        let task = run;
        if let Some(task) = task {
            task(_slot);
        }
    }
}

struct SharedWorkerPool {
    command_queue: Arc<CommandQueue<QueuedPoolTask>>,
    hotkey_queue: Arc<CommandQueue<QueuedPoolTask>>,
    shutdown: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl SharedWorkerPool {
    fn start() -> Option<Self> {
        let command_queue = Arc::new(CommandQueue::new(super::ASYNC_COMMAND_POOL_QUEUE_CAPACITY));
        let hotkey_queue = Arc::new(CommandQueue::new(super::ASYNC_HOTKEY_POOL_QUEUE_CAPACITY));
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut workers = Vec::with_capacity(
            super::ASYNC_COMMAND_WORKER_COUNT + super::ASYNC_HOTKEY_WORKER_COUNT,
        );

        for worker_index in 0..super::ASYNC_COMMAND_WORKER_COUNT {
            let queue = Arc::clone(&command_queue);
            let shutdown_worker = Arc::clone(&shutdown);
            let worker = thread::Builder::new()
                .name(format!("wayexpand-expansion-pool-{worker_index}"))
                .spawn(move || run_pool_worker(queue, shutdown_worker));
            match worker {
                Ok(worker) => workers.push(worker),
                Err(_) => {
                    shutdown.store(true, Ordering::Release);
                    command_queue.notify_all();
                    hotkey_queue.notify_all();
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return None;
                }
            }
        }
        for worker_index in 0..super::ASYNC_HOTKEY_WORKER_COUNT {
            let queue = Arc::clone(&hotkey_queue);
            let shutdown_worker = Arc::clone(&shutdown);
            let worker = thread::Builder::new()
                .name(format!("wayexpand-hotkey-pool-{worker_index}"))
                .spawn(move || run_pool_worker(queue, shutdown_worker));
            match worker {
                Ok(worker) => workers.push(worker),
                Err(_) => {
                    shutdown.store(true, Ordering::Release);
                    command_queue.notify_all();
                    hotkey_queue.notify_all();
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return None;
                }
            }
        }

        Some(Self {
            command_queue,
            hotkey_queue,
            shutdown,
            workers,
        })
    }

    fn command_workers(&self) -> usize {
        super::ASYNC_COMMAND_WORKER_COUNT
    }

    fn hotkey_workers(&self) -> usize {
        super::ASYNC_HOTKEY_WORKER_COUNT
    }
}

impl Drop for SharedWorkerPool {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.command_queue.notify_all();
        self.hotkey_queue.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn run_pool_worker(queue: Arc<CommandQueue<QueuedPoolTask>>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        let Some(task) = queue.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        // The standard panic hook reports the panic without exposing task
        // contents; catching it here keeps one bad task from shrinking the
        // process-wide pool.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task.run()));
    }
}

fn shared_worker_pool() -> Option<Arc<SharedWorkerPool>> {
    static POOL: OnceLock<Mutex<Weak<SharedWorkerPool>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(Weak::new()));
    let mut current = pool
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(shared) = current.upgrade() {
        return Some(shared);
    }
    let shared = Arc::new(SharedWorkerPool::start()?);
    *current = Arc::downgrade(&shared);
    Some(shared)
}

struct ActiveJobs {
    state: Mutex<ActiveJobsState>,
    idle: Condvar,
}

struct ActiveJobsState {
    count: usize,
    closed: bool,
}

impl ActiveJobsState {
    fn new() -> Self {
        Self {
            count: 0,
            closed: false,
        }
    }

    /// Production and scheduler models share this admission transition.
    fn try_enter(&mut self) -> bool {
        if self.closed {
            return false;
        }
        self.count += 1;
        true
    }

    fn close(&mut self) {
        self.closed = true;
    }

    fn leave(&mut self) {
        self.count = self
            .count
            .checked_sub(1)
            .expect("active-job count cannot underflow");
    }

    fn is_idle(&self) -> bool {
        self.count == 0
    }
}

impl ActiveJobs {
    fn new() -> Self {
        Self {
            state: Mutex::new(ActiveJobsState::new()),
            idle: Condvar::new(),
        }
    }

    fn enter(self: &Arc<Self>) -> Option<ActiveJobGuard> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.try_enter() {
            return None;
        }
        Some(ActiveJobGuard(Arc::clone(self)))
    }

    fn close_and_wait(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.close();
        while !state.is_idle() {
            state = self
                .idle
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    #[cfg(test)]
    fn is_closed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
    }
}

struct ActiveJobGuard(Arc<ActiveJobs>);

impl Drop for ActiveJobGuard {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.leave();
        if state.is_idle() {
            self.0.idle.notify_all();
        }
    }
}

/// Send a worker completion without making runtime shutdown dependent on a
/// receiver draining the old configuration's queue. Normal operation keeps
/// backpressure when the bounded completion channel is full; once shutdown is
/// requested, the completion can be discarded because the owning engine is
/// being replaced and must not wait indefinitely for stale output.
enum CompletionSendError<T> {
    Full(T),
    Disconnected(T),
}

/// Shared completion-delivery state transition. `try_send` must return the
/// original value when it cannot enqueue it; that lets the bounded retry
/// loop preserve ownership while checking shutdown. The notifier runs only
/// after a completion has been accepted by the queue.
fn deliver_completion_or_shutdown<T>(
    mut completion: T,
    mut is_shutdown: impl FnMut() -> bool,
    mut try_send: impl FnMut(T) -> Result<(), CompletionSendError<T>>,
    mut wait_for_capacity: impl FnMut(),
    notify: impl FnOnce(),
) -> bool {
    loop {
        if is_shutdown() {
            return false;
        }
        match try_send(completion) {
            Ok(()) => {
                notify();
                return true;
            }
            Err(CompletionSendError::Disconnected(_)) => return false,
            Err(CompletionSendError::Full(value)) => {
                completion = value;
                wait_for_capacity();
            }
        }
    }
}

#[cfg(test)]
pub(super) fn send_completion_or_shutdown<T>(
    sender: &mpsc::SyncSender<T>,
    completion: T,
    shutdown: &AtomicBool,
) -> bool {
    deliver_completion_or_shutdown(
        completion,
        || shutdown.load(Ordering::Acquire),
        |completion| {
            sender.try_send(completion).map_err(|error| match error {
                mpsc::TrySendError::Full(value) => CompletionSendError::Full(value),
                mpsc::TrySendError::Disconnected(value) => CompletionSendError::Disconnected(value),
            })
        },
        || thread::sleep(Duration::from_millis(5)),
        || {},
    )
}

fn send_completion_and_notify<T>(
    sender: &mpsc::SyncSender<T>,
    completion: T,
    shutdown: &AtomicBool,
    notifier: &RwLock<Option<CompletionNotifier>>,
) -> bool {
    deliver_completion_or_shutdown(
        completion,
        || shutdown.load(Ordering::Acquire),
        |completion| {
            sender.try_send(completion).map_err(|error| match error {
                mpsc::TrySendError::Full(value) => CompletionSendError::Full(value),
                mpsc::TrySendError::Disconnected(value) => CompletionSendError::Disconnected(value),
            })
        },
        || thread::sleep(Duration::from_millis(5)),
        || notify_completion(notifier),
    )
}

impl Drop for AsyncCommandRuntime {
    fn drop(&mut self) {
        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _start_gate = self
            .command_start_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.shutdown.store(true, Ordering::Release);
        drop(_start_gate);
        drop(_lifecycle);

        // Remove queued jobs immediately so sensitive payloads do not wait
        // behind another context's work. A task already dequeued observes this
        // runtime's shutdown token; active_jobs waits for it to acknowledge.
        self.pool.command_queue.remove_owner(self.owner);
        self.pool.hotkey_queue.remove_owner(self.owner);
        self.pool.command_queue.notify_all();
        self.pool.hotkey_queue.notify_all();
        self.active_jobs.close_and_wait();
    }
}

static NEXT_POOL_OWNER: AtomicU64 = AtomicU64::new(1);

impl AsyncCommandRuntime {
    #[cfg(test)]
    pub(super) fn for_test(engine: &ExpansionEngine, command_queue_slots: usize) -> Option<Self> {
        let pool = Arc::new(SharedWorkerPool::start()?);
        let runtime = Self::with_pool(
            pool,
            Arc::clone(&engine.shared_input_generation),
            Arc::clone(&engine.command_start_gate),
            Arc::clone(&engine.completion_notifier),
            Arc::clone(&engine.expansion_metrics),
            Arc::clone(&engine.hotkey_metrics),
        );
        runtime
            .command_queue_slots
            .store(command_queue_slots, Ordering::Release);
        Some(runtime)
    }

    pub(super) fn start(
        shared_input_generation: Arc<AtomicU64>,
        command_start_gate: Arc<Mutex<()>>,
        completion_notifier: Arc<RwLock<Option<CompletionNotifier>>>,
        expansion_metrics: Arc<CommandMetricsState>,
        hotkey_metrics: Arc<CommandMetricsState>,
    ) -> Option<Self> {
        let pool = shared_worker_pool()?;
        Some(Self::with_pool(
            pool,
            shared_input_generation,
            command_start_gate,
            completion_notifier,
            expansion_metrics,
            hotkey_metrics,
        ))
    }

    fn with_pool(
        pool: Arc<SharedWorkerPool>,
        shared_input_generation: Arc<AtomicU64>,
        command_start_gate: Arc<Mutex<()>>,
        completion_notifier: Arc<RwLock<Option<CompletionNotifier>>>,
        expansion_metrics: Arc<CommandMetricsState>,
        hotkey_metrics: Arc<CommandMetricsState>,
    ) -> Self {
        let (completion_sender, completion_receiver) =
            mpsc::sync_channel(super::ASYNC_COMMAND_QUEUE_CAPACITY);
        let (hotkey_completion_sender, hotkey_completion_receiver) =
            mpsc::sync_channel(super::ASYNC_COMMAND_QUEUE_CAPACITY);
        Self {
            pool,
            owner: NEXT_POOL_OWNER.fetch_add(1, Ordering::Relaxed),
            completion_sender,
            hotkey_completion_sender,
            receiver: completion_receiver,
            hotkey_receiver: hotkey_completion_receiver,
            expansion_metrics,
            hotkey_metrics,
            shutdown: Arc::new(AtomicBool::new(false)),
            command_start_gate,
            lifecycle_gate: Mutex::new(()),
            active_jobs: Arc::new(ActiveJobs::new()),
            command_queue_slots: Arc::new(AtomicUsize::new(0)),
            hotkey_queue_slots: Arc::new(AtomicUsize::new(0)),
            shared_input_generation,
            completion_notifier,
        }
    }

    pub(super) fn command_worker_threads(&self) -> usize {
        self.pool.command_workers()
    }

    pub(super) fn hotkey_worker_threads(&self) -> usize {
        self.pool.hotkey_workers()
    }

    #[cfg(test)]
    pub(super) fn shares_pool_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.pool, &other.pool)
    }

    pub(super) fn try_send_command(
        &self,
        job: AsyncCommandJob,
    ) -> Result<(), super::QueueSendError> {
        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.shutdown.load(Ordering::Acquire) {
            return Err(super::QueueSendError::Unavailable);
        }
        if !reserve_queue_slot(
            &self.command_queue_slots,
            super::ASYNC_COMMAND_QUEUE_CAPACITY,
        ) {
            self.expansion_metrics
                .queue_rejected_total
                .fetch_add(1, Ordering::Relaxed);
            return Err(super::QueueSendError::Full);
        }
        self.expansion_metrics
            .queue_depth
            .fetch_add(1, Ordering::Relaxed);
        let slot = QueueSlotGuard {
            metrics: Arc::clone(&self.expansion_metrics),
            slots: Arc::clone(&self.command_queue_slots),
            queued: true,
        };
        let context = CommandWorkerContext {
            shutdown: Arc::clone(&self.shutdown),
            input_generation: Arc::clone(&self.shared_input_generation),
            command_start_gate: Arc::clone(&self.command_start_gate),
            completion_sender: self.completion_sender.clone(),
            notifier: Arc::clone(&self.completion_notifier),
            metrics: Arc::clone(&self.expansion_metrics),
            active_jobs: Arc::clone(&self.active_jobs),
        };
        self.enqueue_task(
            &self.pool.command_queue,
            self.expansion_metrics.as_ref(),
            slot,
            Box::new(move |slot| process_command_job(job, context, slot)),
        )
    }

    pub(super) fn try_send_hotkey(
        &self,
        action: HotkeyResult,
    ) -> Result<(), super::QueueSendError> {
        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.shutdown.load(Ordering::Acquire) {
            return Err(super::QueueSendError::Unavailable);
        }
        if !reserve_queue_slot(
            &self.hotkey_queue_slots,
            super::ASYNC_COMMAND_QUEUE_CAPACITY,
        ) {
            self.hotkey_metrics
                .queue_rejected_total
                .fetch_add(1, Ordering::Relaxed);
            return Err(super::QueueSendError::Full);
        }
        self.hotkey_metrics
            .queue_depth
            .fetch_add(1, Ordering::Relaxed);
        let slot = QueueSlotGuard {
            metrics: Arc::clone(&self.hotkey_metrics),
            slots: Arc::clone(&self.hotkey_queue_slots),
            queued: true,
        };
        let context = HotkeyWorkerContext {
            shutdown: Arc::clone(&self.shutdown),
            completion_sender: self.hotkey_completion_sender.clone(),
            notifier: Arc::clone(&self.completion_notifier),
            metrics: Arc::clone(&self.hotkey_metrics),
            active_jobs: Arc::clone(&self.active_jobs),
        };
        self.enqueue_task(
            &self.pool.hotkey_queue,
            self.hotkey_metrics.as_ref(),
            slot,
            Box::new(move |slot| process_hotkey_job(action, context, slot)),
        )
    }

    fn enqueue_task(
        &self,
        queue: &CommandQueue<QueuedPoolTask>,
        metrics: &CommandMetricsState,
        slot: QueueSlotGuard,
        run: PoolTask,
    ) -> Result<(), super::QueueSendError> {
        let task = QueuedPoolTask {
            owner: self.owner,
            run: Some(run),
            _slot: slot,
        };
        match queue.try_send(task) {
            Ok(()) => Ok(()),
            Err(CommandQueueSendError::Full(task)) => {
                drop(task);
                metrics.queue_rejected_total.fetch_add(1, Ordering::Relaxed);
                Err(super::QueueSendError::Full)
            }
            Err(CommandQueueSendError::Unavailable(task)) => {
                drop(task);
                metrics.queue_rejected_total.fetch_add(1, Ordering::Relaxed);
                Err(super::QueueSendError::Unavailable)
            }
        }
    }
}

fn reserve_queue_slot(slots: &AtomicUsize, capacity: usize) -> bool {
    slots
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current < capacity).then_some(current + 1)
        })
        .is_ok()
}

struct CommandWorkerContext {
    shutdown: Arc<AtomicBool>,
    input_generation: Arc<AtomicU64>,
    command_start_gate: Arc<Mutex<()>>,
    completion_sender: mpsc::SyncSender<AsyncCommandCompletion>,
    notifier: Arc<RwLock<Option<CompletionNotifier>>>,
    metrics: Arc<CommandMetricsState>,
    active_jobs: Arc<ActiveJobs>,
}

struct HotkeyWorkerContext {
    shutdown: Arc<AtomicBool>,
    completion_sender: mpsc::SyncSender<AsyncHotkeyCompletion>,
    notifier: Arc<RwLock<Option<CompletionNotifier>>>,
    metrics: Arc<CommandMetricsState>,
    active_jobs: Arc<ActiveJobs>,
}

struct InFlightGuard(Arc<CommandMetricsState>);

impl InFlightGuard {
    fn new(metrics: Arc<CommandMetricsState>) -> Self {
        metrics.in_flight.fetch_add(1, Ordering::Relaxed);
        Self(metrics)
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

fn process_command_job(
    job: AsyncCommandJob,
    context: CommandWorkerContext,
    mut slot: QueueSlotGuard,
) {
    slot.started();
    let Some(_active) = context.active_jobs.enter() else {
        return;
    };
    if context.shutdown.load(Ordering::Acquire) {
        return;
    }
    match job {
        AsyncCommandJob::Form {
            config_index,
            additional_max_size,
            template,
            fields,
            command,
            context: mut template_context,
            title,
            origin,
            mut result,
        } => {
            let _in_flight = InFlightGuard::new(Arc::clone(&context.metrics));
            let kubernetes_context = template.contains("--context {{shellq:context");
            let mut form_fields = fields.as_ref().clone();
            let active_context = if kubernetes_context {
                let context_probe = CommandConfig {
                    action: None,
                    program: "kubectl".into(),
                    args: vec!["config".into(), "current-context".into()],
                    timeout_ms: 2_000,
                    cache_ms: 0,
                    environment: CommandEnvironment::Minimal,
                    pass_env: vec!["KUBECONFIG".into()],
                };
                super::command_runtime::run_command_with_parameters(
                    &context_probe,
                    &std::collections::HashMap::new(),
                    Some(&context.shutdown),
                )
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| {
                    !value.is_empty()
                        && value.chars().count() <= 253
                        && !value.chars().any(char::is_control)
                })
            } else {
                None
            };
            if kubernetes_context {
                if let Some(field) = form_fields.iter_mut().find(|field| field.key == "context") {
                    if let (Some(active), FormFieldKind::Text { default }) =
                        (&active_context, &mut field.kind)
                    {
                        *default = active.clone();
                    }
                }
            }
            let context_notice = if kubernetes_context {
                Some(match active_context.as_deref() {
                    Some(active) => {
                        format!("Active kubectl context: {active}. Verify it before inserting.")
                    }
                    None => "Active kubectl context could not be read. Enter the intended context explicitly.".into(),
                })
            } else {
                None
            };
            let preview_template = (command.is_none() && form_template_uses_only_fields(&template))
                .then_some(template.as_ref());
            let output = match super::command_runtime::run_form_helper_with_preview(
                &title,
                &form_fields,
                preview_template,
                context_notice.as_deref(),
                &context.shutdown,
            ) {
                Ok(values) => {
                    if let Some(command) = &command {
                        match super::command_runtime::normalize_form_action_parameters(&values) {
                            Ok(parameters) => super::command_runtime::run_command_with_parameters(
                                command,
                                &parameters,
                                Some(&context.shutdown),
                            ),
                            Err(error) => Err(error),
                        }
                    } else {
                        template_context.fields = Arc::new(values);
                        match crate::render_template_with_cursor(&template, &template_context) {
                            Ok((text, cursor_offset)) => {
                                result.cursor_offset = cursor_offset;
                                Ok(text)
                            }
                            Err(_) => Err(CommandError::TemplateRenderFailed),
                        }
                    }
                }
                Err(error) => Err(error),
            };
            let _ = send_completion_and_notify(
                &context.completion_sender,
                AsyncCommandCompletion {
                    config_index,
                    generation: 0,
                    cache_ms: 0,
                    additional_max_size,
                    result,
                    output: output.map(Into::into),
                    form: Some(origin),
                    _slot: slot,
                },
                &context.shutdown,
                &context.notifier,
            );
        }
        AsyncCommandJob::Expansion {
            config_index,
            generation,
            additional_max_size,
            command,
            result,
        } => {
            let cache_ms = command.cache_ms;
            if !input_generation_matches(
                generation,
                context.input_generation.load(Ordering::Acquire),
            ) {
                context.metrics.record_error(false, true);
                let _ = send_completion_and_notify(
                    &context.completion_sender,
                    AsyncCommandCompletion {
                        config_index,
                        generation,
                        cache_ms,
                        additional_max_size,
                        result,
                        output: Err(CommandError::StaleInput),
                        form: None,
                        _slot: slot,
                    },
                    &context.shutdown,
                    &context.notifier,
                );
                return;
            }
            let _in_flight = InFlightGuard::new(Arc::clone(&context.metrics));
            let output = super::command_runtime::run_command_with_generation(
                &command,
                Some(&context.shutdown),
                &context.input_generation,
                &context.command_start_gate,
                generation,
            );
            if let Err(error) = &output {
                context.metrics.record_error(
                    matches!(error, CommandError::Timeout),
                    matches!(error, CommandError::StaleInput),
                );
            }
            let _ = send_completion_and_notify(
                &context.completion_sender,
                AsyncCommandCompletion {
                    config_index,
                    generation,
                    cache_ms,
                    additional_max_size,
                    result,
                    output: output.map(Into::into),
                    form: None,
                    _slot: slot,
                },
                &context.shutdown,
                &context.notifier,
            );
        }
    }
}

fn process_hotkey_job(
    action: HotkeyResult,
    context: HotkeyWorkerContext,
    mut slot: QueueSlotGuard,
) {
    slot.started();
    let Some(_active) = context.active_jobs.enter() else {
        return;
    };
    if context.shutdown.load(Ordering::Acquire) {
        return;
    }
    let _in_flight = InFlightGuard::new(Arc::clone(&context.metrics));
    let output = ExpansionEngine::execute_hotkey_with_shutdown(&action, Some(&context.shutdown));
    if let Err(error) = &output {
        context.metrics.record_error(
            matches!(error, HotkeyError::Timeout(_)),
            matches!(
                error,
                HotkeyError::CommandFailure(super::CommandErrorKind::StaleInput)
            ),
        );
    }
    let _ = send_completion_and_notify(
        &context.completion_sender,
        AsyncHotkeyCompletion {
            action,
            output,
            _slot: slot,
        },
        &context.shutdown,
        &context.notifier,
    );
}

pub(super) enum AsyncCommandJob {
    Expansion {
        config_index: usize,
        generation: u64,
        additional_max_size: usize,
        command: Arc<CommandConfig>,
        result: ExpansionResult,
    },
    Form {
        config_index: usize,
        additional_max_size: usize,
        template: Arc<str>,
        fields: Arc<Vec<crate::FormField>>,
        command: Option<Arc<CommandConfig>>,
        context: Box<crate::TemplateContext>,
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
    pub(super) output: Result<super::SensitiveText, CommandError>,
    pub(super) form: Option<FormOrigin>,
    _slot: QueueSlotGuard,
}

pub(super) struct AsyncHotkeyCompletion {
    pub(super) action: HotkeyResult,
    pub(super) output: Result<(), HotkeyError>,
    _slot: QueueSlotGuard,
}

#[cfg(test)]
mod tests {
    use super::{ActiveJobs, CommandQueue, CommandQueueSendError};
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn active_job_shutdown_closes_admission_before_waiting_for_running_work() {
        let jobs = Arc::new(ActiveJobs::new());
        let running = jobs.enter().expect("job admitted before shutdown");
        let closing = Arc::clone(&jobs);
        let (finished_sender, finished_receiver) = std::sync::mpsc::channel();
        let closer = thread::spawn(move || {
            closing.close_and_wait();
            let _ = finished_sender.send(());
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while !jobs.is_closed() {
            assert!(
                Instant::now() < deadline,
                "shutdown did not close admission"
            );
            thread::yield_now();
        }
        assert!(jobs.enter().is_none(), "closed runtime admitted new work");
        assert!(finished_receiver.try_recv().is_err());
        drop(running);
        finished_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("shutdown should finish after active work acknowledges");
        closer.join().expect("shutdown waiter panicked");
    }

    #[test]
    fn command_queue_recovers_a_poisoned_lock_only_when_its_bound_holds() {
        let queue = Arc::new(CommandQueue::new(2));
        let poisoned_queue = Arc::clone(&queue);
        assert!(thread::spawn(move || {
            let mut items = poisoned_queue.items.lock().unwrap();
            items.push_back(1);
            panic!("simulate a worker panic while holding the queue mutex");
        })
        .join()
        .is_err());

        assert_eq!(queue.try_send(2), Ok(()));
        assert_eq!(queue.recv_timeout(Duration::ZERO), Some(1));
        assert_eq!(queue.recv_timeout(Duration::ZERO), Some(2));
        assert!(!queue.items.is_poisoned());
    }

    #[test]
    fn command_queue_fails_closed_if_a_poisoned_queue_exceeds_its_bound() {
        let queue = Arc::new(CommandQueue::new(1));
        let poisoned_queue = Arc::clone(&queue);
        assert!(thread::spawn(move || {
            let mut items = poisoned_queue.items.lock().unwrap();
            items.extend([1, 2]);
            panic!("simulate an invalid queue state");
        })
        .join()
        .is_err());

        assert_eq!(
            queue.try_send(3),
            Err(CommandQueueSendError::Unavailable(3))
        );
        assert_eq!(queue.recv_timeout(Duration::ZERO), None);
        assert!(queue.items.is_poisoned());
    }

    #[test]
    fn command_queue_distinguishes_saturation_from_unavailable_state() {
        let queue = CommandQueue::new(1);
        assert_eq!(queue.try_send(1), Ok(()));
        assert_eq!(queue.try_send(2), Err(CommandQueueSendError::Full(2)));
    }

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

#[cfg(all(test, feature = "loom-tests"))]
mod loom_models {
    use loom::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use loom::sync::{Arc, Mutex};
    use loom::thread;

    use super::super::input_generation_matches;
    use super::{deliver_completion_or_shutdown, CompletionSendError};

    /// The command worker must publish before waking the reactor. The reactor
    /// may drain between those two operations; that produces a harmless wake,
    /// while a completion still queued at worker return must have a wake
    /// pending so a poll-based reactor cannot sleep past it.
    #[test]
    fn completion_delivery_and_reactor_wakeup_have_no_lost_completion() {
        loom::model(|| {
            let slot = Arc::new(Mutex::new(None));
            let notified = Arc::new(AtomicBool::new(false));
            let consumed = Arc::new(loom::sync::atomic::AtomicUsize::new(0));

            let worker_slot = Arc::clone(&slot);
            let worker_notified = Arc::clone(&notified);
            let worker = thread::spawn(move || {
                deliver_completion_or_shutdown(
                    1_usize,
                    || false,
                    |completion| {
                        let mut queued = worker_slot.lock().unwrap();
                        if queued.is_some() {
                            Err(CompletionSendError::Full(completion))
                        } else {
                            *queued = Some(completion);
                            Ok(())
                        }
                    },
                    thread::yield_now,
                    || worker_notified.store(true, Ordering::Release),
                )
            });

            let reactor_slot = Arc::clone(&slot);
            let reactor_consumed = Arc::clone(&consumed);
            let reactor = thread::spawn(move || {
                thread::yield_now();
                if reactor_slot.lock().unwrap().take().is_some() {
                    reactor_consumed.fetch_add(1, Ordering::Relaxed);
                }
            });

            assert!(worker.join().unwrap());
            reactor.join().unwrap();
            let queued = slot.lock().unwrap().is_some();
            let consumed = consumed.load(Ordering::Relaxed);
            assert_eq!(usize::from(queued) + consumed, 1);
            if queued {
                assert!(notified.load(Ordering::Acquire));
            }
            assert!(notified.load(Ordering::Acquire));
        });
    }

    /// A full completion queue must not make worker teardown wait forever.
    /// Shutdown releases the producer from backpressure without replacing the
    /// queued completion or falsely waking the reactor for a dropped one.
    #[test]
    fn shutdown_releases_completion_delivery_waiting_on_full_queue() {
        loom::model(|| {
            let slot = Arc::new(Mutex::new(Some(0_usize)));
            let shutdown = Arc::new(AtomicBool::new(false));
            let notified = Arc::new(AtomicBool::new(false));

            let worker_slot = Arc::clone(&slot);
            let worker_shutdown = Arc::clone(&shutdown);
            let worker_notified = Arc::clone(&notified);
            let worker = thread::spawn(move || {
                deliver_completion_or_shutdown(
                    1_usize,
                    || worker_shutdown.load(Ordering::Acquire),
                    |completion| {
                        let mut queued = worker_slot.lock().unwrap();
                        if queued.is_some() {
                            Err(CompletionSendError::Full(completion))
                        } else {
                            *queued = Some(completion);
                            Ok(())
                        }
                    },
                    thread::yield_now,
                    || worker_notified.store(true, Ordering::Release),
                )
            });

            let shutdown_flag = Arc::clone(&shutdown);
            let stopper = thread::spawn(move || {
                thread::yield_now();
                shutdown_flag.store(true, Ordering::Release);
            });

            assert!(!worker.join().unwrap());
            stopper.join().unwrap();
            assert!(shutdown.load(Ordering::Acquire));
            assert_eq!(*slot.lock().unwrap(), Some(0));
            assert!(!notified.load(Ordering::Acquire));
        });
    }

    /// Models the worker's generation check racing with input invalidation
    /// and completion publication. The engine performs a second generation
    /// check when draining the completion; if input has already advanced the
    /// context, output must not be accepted even if the worker published it.
    #[test]
    fn stale_completion_is_rejected_after_focus_moves_away_and_back() {
        loom::model(|| {
            let generation = Arc::new(AtomicU64::new(7));
            let focused_window = Arc::new(loom::sync::atomic::AtomicUsize::new(1));
            let completion_ready = Arc::new(AtomicBool::new(false));

            let worker_generation = Arc::clone(&generation);
            let worker_window = Arc::clone(&focused_window);
            let worker_completion = Arc::clone(&completion_ready);
            let worker = thread::spawn(move || {
                if input_generation_matches(7, worker_generation.load(Ordering::Acquire))
                    && worker_window.load(Ordering::Acquire) == 1
                {
                    // Command execution and queue publication are asynchronous
                    // with respect to the input reactor.
                    thread::yield_now();
                    worker_completion.store(true, Ordering::Release);
                }
            });

            let input_generation = Arc::clone(&generation);
            let input_window = Arc::clone(&focused_window);
            let input = thread::spawn(move || {
                // Model an ABA focus transition: the user leaves target A,
                // types or otherwise changes context in target B, then
                // returns to A before the old worker result is drained. The
                // target identity alone is equal again, so the monotonically
                // advanced input generation must still reject that result.
                input_window.store(2, Ordering::Release);
                input_generation.store(8, Ordering::Release);
                thread::yield_now();
                input_window.store(1, Ordering::Release);
                input_generation.store(9, Ordering::Release);
            });

            worker.join().expect("modeled command worker panicked");
            input.join().expect("modeled input reactor panicked");

            let published = completion_ready.load(Ordering::Acquire);
            let still_current = input_generation_matches(7, generation.load(Ordering::Acquire));
            let same_target = focused_window.load(Ordering::Acquire) == 1;
            let may_apply = published && still_current && same_target;
            assert!(
                !may_apply,
                "a completion from an earlier visit to the same window must not be applied"
            );
            assert_eq!(focused_window.load(Ordering::Acquire), 1);
            assert_eq!(generation.load(Ordering::Acquire), 9);
        });
    }

    /// Mirrors the production command-start gate: target invalidation and
    /// runtime shutdown serialize with the final freshness check and spawn.
    /// The command is admitted only if its start linearizes first.
    #[test]
    fn command_start_linearizes_before_target_change_or_shutdown() {
        loom::model(|| {
            let gate = Arc::new(loom::sync::Mutex::new(()));
            let generation = Arc::new(AtomicU64::new(0));
            let shutdown = Arc::new(AtomicBool::new(false));
            let sequence = Arc::new(loom::sync::atomic::AtomicUsize::new(0));
            let attempt_order = Arc::new(loom::sync::atomic::AtomicUsize::new(0));
            let invalidation_order = Arc::new(loom::sync::atomic::AtomicUsize::new(0));
            let started = Arc::new(AtomicBool::new(false));

            let worker_gate = Arc::clone(&gate);
            let worker_generation = Arc::clone(&generation);
            let worker_shutdown = Arc::clone(&shutdown);
            let worker_sequence = Arc::clone(&sequence);
            let worker_attempt_order = Arc::clone(&attempt_order);
            let worker_started = Arc::clone(&started);
            let worker = thread::spawn(move || {
                let _admission = worker_gate.lock().unwrap();
                let order = worker_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                worker_attempt_order.store(order, Ordering::Release);
                if worker_generation.load(Ordering::Acquire) == 0
                    && !worker_shutdown.load(Ordering::Acquire)
                {
                    worker_started.store(true, Ordering::Release);
                }
            });

            let invalidator_gate = Arc::clone(&gate);
            let invalidator_generation = Arc::clone(&generation);
            let invalidator_shutdown = Arc::clone(&shutdown);
            let invalidator_sequence = Arc::clone(&sequence);
            let invalidator_order = Arc::clone(&invalidation_order);
            let invalidator = thread::spawn(move || {
                let _admission = invalidator_gate.lock().unwrap();
                invalidator_generation.store(1, Ordering::Release);
                invalidator_shutdown.store(true, Ordering::Release);
                let order = invalidator_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                invalidator_order.store(order, Ordering::Release);
            });

            worker.join().unwrap();
            invalidator.join().unwrap();
            assert_eq!(
                started.load(Ordering::Acquire),
                attempt_order.load(Ordering::Acquire) < invalidation_order.load(Ordering::Acquire),
                "a command may start only when its admission linearizes before invalidation"
            );
        });
    }
}

#[cfg(all(test, feature = "loom-tests"))]
mod active_jobs_loom {
    use super::ActiveJobsState;
    use loom::sync::atomic::{AtomicBool, Ordering};
    use loom::sync::{Arc, Mutex};
    use loom::thread;

    /// Uses the same state transitions as ActiveJobs::enter/close_and_wait to
    /// exhaust the race between a dequeued worker registering and shutdown
    /// closing admission. Shutdown may finish only after every admitted job
    /// has left, and no job can be admitted after closure.
    #[test]
    fn shutdown_waits_for_admitted_work_and_refuses_late_workers() {
        loom::model(|| {
            let state = Arc::new(Mutex::new(ActiveJobsState::new()));
            let shutdown_finished = Arc::new(AtomicBool::new(false));
            let worker_state = Arc::clone(&state);
            let worker_shutdown_finished = Arc::clone(&shutdown_finished);
            let worker = thread::spawn(move || {
                let admitted = worker_state.lock().unwrap().try_enter();
                if admitted {
                    thread::yield_now();
                    assert!(
                        !worker_shutdown_finished.load(Ordering::Acquire),
                        "shutdown completed while an admitted job was active"
                    );
                    worker_state.lock().unwrap().leave();
                }
            });

            let shutdown_state = Arc::clone(&state);
            let shutdown_done = Arc::clone(&shutdown_finished);
            let shutdown = thread::spawn(move || {
                {
                    let mut state = shutdown_state.lock().unwrap();
                    state.close();
                    assert!(!state.try_enter(), "closed runtime admitted a worker");
                }
                loop {
                    if shutdown_state.lock().unwrap().is_idle() {
                        shutdown_done.store(true, Ordering::Release);
                        break;
                    }
                    thread::yield_now();
                }
            });

            worker.join().unwrap();
            shutdown.join().unwrap();
            let state = state.lock().unwrap();
            assert!(state.closed);
            assert!(state.is_idle());
            assert!(shutdown_finished.load(Ordering::Acquire));
        });
    }
}
