use super::command_runtime::{
    configure_command_environment, configure_process_group, run_broker_hotkey, ChildSupervisor,
};
use super::*;
use std::{sync::atomic::AtomicBool, thread};

const HOTKEY_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

impl ExpansionEngine {
    /// Queue a hotkey action for bounded asynchronous execution. The caller
    /// must drain completions periodically; neither enqueueing nor draining
    /// waits for the child process.
    pub fn queue_hotkey(&self, action: &HotkeyResult) -> Result<(), HotkeyError> {
        if self.command_execution_disabled(&action.command) {
            return Err(HotkeyError::PolicyBlocked);
        }
        let Some(runtime) = self.async_commands.as_ref() else {
            return Err(HotkeyError::WorkerUnavailable);
        };
        runtime
            .try_send_hotkey(action.clone())
            .map_err(|error| match error {
                QueueSendError::Full => HotkeyError::QueueFull,
                QueueSendError::Disconnected | QueueSendError::Unavailable => {
                    HotkeyError::WorkerUnavailable
                }
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
        let Some(index) = self.validated.hotkeys.get(chord) else {
            return Vec::new();
        };
        let Some(binding): Option<&HotkeyConfig> = self.config.hotkey.get(*index) else {
            return Vec::new();
        };
        vec![HotkeyResult {
            chord: chord.clone(),
            description: binding.description.clone(),
            command: binding.command.clone(),
        }]
    }

    /// Execute one validated hotkey action without invoking a shell. Output
    /// is discarded and the process is bounded by the configured timeout.
    /// This is intentionally synchronous for explicit, user-initiated
    /// callers; the daemon must use [`Self::queue_hotkey`] so its capture loop
    /// never waits for a child process.
    pub fn execute_hotkey(&self, result: &HotkeyResult) -> Result<(), HotkeyError> {
        if self.command_execution_disabled(&result.command) {
            return Err(HotkeyError::PolicyBlocked);
        }
        Self::execute_hotkey_with_shutdown(result, None)
    }

    pub(super) fn execute_hotkey_with_shutdown(
        result: &HotkeyResult,
        shutdown: Option<&AtomicBool>,
    ) -> Result<(), HotkeyError> {
        if result.command.action.is_some() {
            return run_broker_hotkey(&result.command, shutdown).map_err(|error| match error {
                CommandError::Timeout => HotkeyError::Timeout(result.command.timeout_ms),
                error => HotkeyError::CommandFailure(error.kind()),
            });
        }
        let mut command = Command::new(&result.command.program);
        configure_command_environment(&mut command, &result.command);
        command
            .args(&result.command.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        // Hotkey actions get the same per-command cgroup as expansion
        // commands, so descendants that call setsid() are also cleaned up.
        let cgroup = wayexpand_process_supervisor::ChildCgroup::prepare(&mut command)
            .map_err(HotkeyError::Spawn)?;
        let child = command.spawn().map_err(HotkeyError::Spawn)?;
        // The supervisor observes exit with waitid(WNOWAIT) on Linux, so the
        // leader stays a zombie and its PGID cannot be recycled until the
        // group kill below has run. Dropping it kills and reaps the group on
        // every early return.
        let mut guard = ChildSupervisor::new_with_cgroup(child, cgroup);
        let deadline = Instant::now() + Duration::from_millis(result.command.timeout_ms);
        wait_for_hotkey_exit(&mut guard, deadline, shutdown, result.command.timeout_ms)?;

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

/// Wait for hotkey child completion while preserving the supervisor's
/// waitid(WNOWAIT) and descendant-cleanup sequence. Returns the number of
/// readiness polls to make the current 5 ms strategy measurable.
fn wait_for_hotkey_exit(
    guard: &mut ChildSupervisor,
    deadline: Instant,
    shutdown: Option<&AtomicBool>,
    timeout_ms: u64,
) -> Result<usize, HotkeyError> {
    let mut polls = 0;
    loop {
        if shutdown.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(HotkeyError::Timeout(timeout_ms));
        }
        polls += 1;
        match guard.has_exited() {
            Ok(true) => return Ok(polls),
            Ok(false) if Instant::now() < deadline => thread::sleep(HOTKEY_EXIT_POLL_INTERVAL),
            Ok(false) => return Err(HotkeyError::Timeout(timeout_ms)),
            Err(error) => return Err(HotkeyError::Spawn(error)),
        }
    }
}

#[cfg(test)]
mod polling_benchmark {
    use super::*;
    use std::process::Command;
    use wayexpand_process_supervisor::ChildSupervisor;

    /// Run with `cargo test -p wayexpand-core benchmark_hotkey_polling_wait -- --ignored --nocapture`.
    /// This measures the existing 5 ms child-exit polling loop; it does not
    /// claim end-to-end command latency or change containment semantics.
    #[test]
    #[ignore = "manual hotkey child-exit polling benchmark"]
    fn benchmark_hotkey_polling_wait() {
        let mut elapsed = Vec::with_capacity(100);
        let mut blocking_wait_elapsed = Vec::with_capacity(100);
        let mut total_polls = 0usize;
        for _ in 0..100 {
            let child = Command::new("/bin/sleep")
                .arg("0.025")
                .spawn()
                .expect("spawn benchmark child");
            let mut supervisor = ChildSupervisor::new(child);
            let started = Instant::now();
            total_polls += wait_for_hotkey_exit(
                &mut supervisor,
                started + Duration::from_secs(1),
                None,
                1_000,
            )
            .expect("benchmark child should exit");
            supervisor.kill_group();
            assert!(supervisor.reap().expect("reap benchmark child").success());
            elapsed.push(started.elapsed());

            let mut child = Command::new("/bin/sleep")
                .arg("0.025")
                .spawn()
                .expect("spawn blocking-wait comparison child");
            let started = Instant::now();
            assert!(child.wait().expect("wait for comparison child").success());
            blocking_wait_elapsed.push(started.elapsed());
        }
        elapsed.sort_unstable();
        blocking_wait_elapsed.sort_unstable();
        let p50 = elapsed[49].as_micros();
        let p95 = elapsed[94].as_micros();
        let blocking_p50 = blocking_wait_elapsed[49].as_micros();
        let blocking_p95 = blocking_wait_elapsed[94].as_micros();
        eprintln!(
            "hotkey exit polling: samples=100 interval_ms={} total_polls={} mean_polls_per_child={:.2} p50_wait_us={} p95_wait_us={}; uncontained blocking-wait reference p50_wait_us={} p95_wait_us={}",
            HOTKEY_EXIT_POLL_INTERVAL.as_millis(),
            total_polls,
            total_polls as f64 / elapsed.len() as f64,
            p50,
            p95,
            blocking_p50,
            blocking_p95
        );
    }
}
