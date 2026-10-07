use super::command_runtime::{
    configure_command_environment, configure_process_group, ChildSupervisor,
};
use super::*;
use std::{sync::atomic::AtomicBool, thread};

impl ExpansionEngine {
    /// Queue a hotkey action for bounded asynchronous execution. The caller
    /// must drain completions periodically; neither enqueueing nor draining
    /// waits for the child process.
    pub fn queue_hotkey(&self, action: &HotkeyResult) -> Result<(), HotkeyError> {
        let Some(runtime) = self.async_commands.as_ref() else {
            return Err(HotkeyError::WorkerUnavailable);
        };
        runtime
            .try_send_hotkey(action.clone())
            .map_err(|error| match error {
                QueueSendError::Full => HotkeyError::QueueFull,
                QueueSendError::Disconnected => HotkeyError::WorkerUnavailable,
            })
    }

    /// Return hotkey action completions without waiting for any child
    /// process. Results are intended for logging and operational status; the
    /// action itself has already completed on the worker thread.
    pub fn drain_completed_hotkeys(&mut self) -> Vec<(HotkeyResult, Result<(), HotkeyError>)> {
        let Some(runtime) = self.async_commands.as_ref() else {
            return Vec::new();
        };
        runtime
            .hotkey_receiver
            .try_iter()
            .map(|completion| (completion.action, completion.output))
            .collect()
    }

    /// Resolve a normalized key chord into configured actions. This method is
    /// side-effect free; the daemon or script runtime owns execution policy,
    /// cancellation, and capability checks.
    pub fn process_key(&self, chord: &KeyChord) -> Vec<HotkeyResult> {
        if !self.is_capture_enabled() {
            return Vec::new();
        }
        self.hotkeys
            .iter()
            .filter(|(configured, _)| configured.matches(chord))
            .filter_map(|(_, index)| {
                let binding: &HotkeyConfig = self.config.hotkey.get(*index)?;
                Some(HotkeyResult {
                    chord: chord.clone(),
                    description: binding.description.clone(),
                    command: binding.command.clone(),
                })
            })
            .collect()
    }

    /// Execute one validated hotkey action without invoking a shell. Output
    /// is discarded and the process is bounded by the configured timeout.
    /// This is intentionally synchronous for explicit, user-initiated
    /// callers; the daemon must use [`Self::queue_hotkey`] so its capture loop
    /// never waits for a child process.
    pub fn execute_hotkey(result: &HotkeyResult) -> Result<(), HotkeyError> {
        Self::execute_hotkey_with_shutdown(result, None)
    }

    pub(super) fn execute_hotkey_with_shutdown(
        result: &HotkeyResult,
        shutdown: Option<&AtomicBool>,
    ) -> Result<(), HotkeyError> {
        let mut command = Command::new(&result.command.program);
        configure_command_environment(&mut command, &result.command);
        command
            .args(&result.command.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        let child = command.spawn().map_err(HotkeyError::Spawn)?;
        // The supervisor observes exit with waitid(WNOWAIT) on Linux, so the
        // leader stays a zombie and its PGID cannot be recycled until the
        // group kill below has run. Dropping it kills and reaps the group on
        // every early return.
        let mut guard = ChildSupervisor::new(child);
        let deadline = Instant::now() + Duration::from_millis(result.command.timeout_ms);
        loop {
            if shutdown.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                return Err(HotkeyError::Timeout(result.command.timeout_ms));
            }
            match guard.has_exited() {
                Ok(true) => break,
                Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                Ok(false) => return Err(HotkeyError::Timeout(result.command.timeout_ms)),
                Err(error) => return Err(HotkeyError::Spawn(error)),
            }
        }

        // The leader may exit successfully while ordinary descendants remain in
        // its process group. Kill the group before reaping the leader;
        // otherwise a successful hotkey can leave background work running.
        guard.kill_group();
        let status = guard.reap().map_err(HotkeyError::Spawn)?;

        if status.success() {
            Ok(())
        } else {
            Err(HotkeyError::Failed(status.to_string()))
        }
    }
}
