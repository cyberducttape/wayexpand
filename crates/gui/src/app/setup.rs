//! First-run and backend setup: the guided setup task, turn-on button, and raw-input (evdev) setup.

use crate::*;
use std::{
    io::Read,
    os::fd::AsRawFd,
    process::{ChildStderr, ChildStdout},
};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const MAX_SETUP_OUTPUT_BYTES: usize = 128 * 1024;
const SETUP_OUTPUT_READS_PER_DRAIN: usize = 64;

fn ibus_registration_needed(report: &serde_json::Value) -> Option<bool> {
    let checks = report.get("checks")?.as_array()?;
    let state = |id: &str| {
        checks
            .iter()
            .find(|check| check.get("id").and_then(serde_json::Value::as_str) == Some(id))
            .and_then(|check| check.get("state"))
            .and_then(serde_json::Value::as_str)
    };
    let component_valid = state("component") == Some("pass");
    let manifest_valid = state("manifest") == Some("pass");
    Some(!(component_valid && manifest_valid))
}

fn set_nonblocking<R: AsRawFd>(stream: &R) -> Result<(), String> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(format!(
            "could not make setup output nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn drain_setup_output<R: Read>(reader: &mut R, output: &mut Vec<u8>) -> Result<bool, String> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..SETUP_OUTPUT_READS_PER_DRAIN {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                output.extend_from_slice(&buffer[..count]);
                if output.len() > MAX_SETUP_OUTPUT_BYTES {
                    return Err("setup output exceeded the safety limit".to_owned());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(false)
}

fn redact_setup_text(text: &str) -> String {
    let mut redacted = text.to_owned();
    if let Some(home) = env::var_os("HOME") {
        let home = home.to_string_lossy();
        if !home.is_empty() {
            redacted = redacted.replace(home.as_ref(), "~");
        }
    }
    redacted
}

fn setup_failure_report(stdout: &[u8], stderr: &[u8], reason: &str) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let combined = format!("{stdout}\n{stderr}\n{reason}");
    let summary = stderr
        .lines()
        .rev()
        .find(|line| {
            let line = line.trim();
            !line.is_empty()
                && !line.starts_with("Next:")
                && !line.starts_with("Fix:")
                && !line.starts_with("Remedy:")
        })
        .map(str::trim)
        .filter(|line| !line.starts_with("Usage:"))
        .or_else(|| {
            stdout.lines().rev().find(|line| {
                let line = line.trim();
                line.starts_with("Error:")
                    || line.starts_with("FAIL:")
                    || line.contains(" failed:")
                    || line.starts_with("SETUP RESULT: ACTION REQUIRED")
            })
        })
        .unwrap_or(reason)
        .trim_start_matches("Error: ");
    let stage = stdout
        .lines()
        .rev()
        .find(|line| {
            let line = line.trim();
            !line.is_empty()
                && (line.starts_with("Step ")
                    || line.starts_with("✓")
                    || line.starts_with("•")
                    || line.starts_with("Installing ")
                    || line.starts_with("Checking ")
                    || line.starts_with("Activating "))
        })
        .map(str::trim)
        .unwrap_or("Setup command execution");
    let problem_id = combined
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
        .find(|part| part.starts_with("WX-") && part.len() > 3)
        .unwrap_or("WX-SETUP-001");
    let remedy = combined
        .lines()
        .find(|line| {
            let line = line.trim_start();
            line.starts_with("Next:") || line.starts_with("Fix:") || line.starts_with("Remedy:")
        })
        .map(str::trim)
        .unwrap_or("Run `wayexpand setup --check` and address the reported prerequisite.");
    let lower = combined.to_ascii_lowercase();
    let rollback = if lower.contains("rollback was incomplete")
        || lower.contains("rollback failed")
        || lower.contains("rollback issues")
    {
        "Incomplete; review the remaining changes listed below."
    } else if lower.contains("rollback restored:")
        || lower.contains("rollback completed")
        || lower.contains("no persistent setup changes needed rollback")
    {
        "Succeeded, or no persistent changes required rollback."
    } else {
        "Not reported by the setup command."
    };
    format!(
        "Setup diagnostic report\n\
         Summary: {}\n\
         Failing stage: {}\n\
         Problem ID: {}\n\
         Suggested remedy: {}\n\
         Rollback: {}\n\n\
         --- stdout ---\n{}\n\n\
         --- stderr ---\n{}",
        redact_setup_text(summary.trim()),
        redact_setup_text(stage),
        problem_id,
        redact_setup_text(remedy),
        rollback,
        redact_setup_text(stdout.trim()),
        redact_setup_text(stderr.trim()),
    )
}

fn setup_failure_summary(report: &str) -> String {
    report
        .lines()
        .find_map(|line| line.strip_prefix("Summary: "))
        .unwrap_or("Setup failed; open the diagnostic report for details.")
        .to_owned()
}

fn finish_setup_process(
    supervisor: &mut ChildSupervisor,
    stdout: &mut ChildStdout,
    stderr: &mut ChildStderr,
    stdout_bytes: &mut Vec<u8>,
    stderr_bytes: &mut Vec<u8>,
    mut stdout_eof: bool,
    mut stderr_eof: bool,
) -> Result<std::process::ExitStatus, String> {
    supervisor.kill_group();
    let status = supervisor.reap().map_err(|error| error.to_string())?;
    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while !(stdout_eof && stderr_eof) && Instant::now() < drain_deadline {
        if !stdout_eof {
            stdout_eof = drain_setup_output(stdout, stdout_bytes)?;
        }
        if !stderr_eof {
            stderr_eof = drain_setup_output(stderr, stderr_bytes)?;
        }
        if !(stdout_eof && stderr_eof) {
            thread::sleep(Duration::from_millis(5));
        }
    }
    if !stdout_eof || !stderr_eof {
        return Err("setup output did not complete".to_owned());
    }
    Ok(status)
}

fn run_user_systemctl(
    arguments: &[&str],
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<std::process::ExitStatus, String> {
    let mut command = std::process::Command::new("systemctl");
    configure_process_group(&mut command);
    let child = command
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start systemctl: {error}"))?;
    let mut supervisor = ChildSupervisor::new(child);
    let deadline = Instant::now() + timeout;
    loop {
        if cancelled.load(Ordering::Acquire) {
            supervisor.kill_group();
            let _ = supervisor.reap();
            return Err("Service operation cancelled".into());
        }
        match supervisor.has_exited() {
            Ok(true) => return supervisor.reap().map_err(|error| error.to_string()),
            Ok(false) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(false) => {
                supervisor.kill_group();
                let cleanup = supervisor.reap();
                return Err(format!(
                    "systemctl {} timed out; process cleanup {}",
                    arguments.join(" "),
                    if cleanup.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    }
                ));
            }
            Err(error) => {
                supervisor.kill_group();
                let cleanup = supervisor.reap();
                return Err(format!(
                    "could not monitor systemctl {} ({error}); process cleanup {}",
                    arguments.join(" "),
                    if cleanup.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    }
                ));
            }
        }
    }
}

impl GuiApp {
    /// Run the safe recommended setup off the UI thread. The additional flag
    /// is included only after the user explicitly accepts IBus replacement
    /// and the possible system-wide IBus manifest registration.
    pub(crate) fn start_setup(&mut self) {
        self.start_setup_with_replacement_consent(false, false);
    }

    fn start_setup_with_replacement_consent(
        &mut self,
        replacement_consent: bool,
        system_integration_consent: bool,
    ) {
        let mut arguments = vec!["setup".into(), "--yes".into()];
        if replacement_consent {
            arguments.push("--accept-non-atomic-replacement".into());
        }
        if system_integration_consent {
            arguments.push("--install-system-integration".into());
        }
        self.start_cli_task(
            arguments,
            self.strings.status_setup_running().to_owned(),
            self.strings.status_setup_done().to_owned(),
        );
    }

    pub(crate) fn start_cli_task(
        &mut self,
        arguments: Vec<String>,
        running_message: String,
        success_message: String,
    ) {
        if self.setup_task.is_some() {
            return;
        }
        let is_setup = arguments
            .first()
            .is_some_and(|argument| argument == "setup");
        let is_setup_preflight = arguments
            .iter()
            .map(String::as_str)
            .eq(["ibus", "status", "--json"]);
        if is_setup {
            self.setup_failure_report = None;
            self.setup_failure_report_open = false;
        }
        self.cli_task_kind = None;
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_is_setup = is_setup;
        let worker_is_setup_preflight = is_setup_preflight;
        // Prefer the CLI installed beside this binary (release archives and
        // development builds), else whatever `wayexpand` is on PATH.
        let cli = env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("wayexpand")))
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("wayexpand"));
        let spawned = thread::Builder::new()
            .name("wayexpand-cli-task".into())
            .spawn(move || {
                let result = (|| {
                    let mut command = std::process::Command::new(&cli);
                    configure_process_group(&mut command);
                    let mut child = command
                        .args(&arguments)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|error| {
                            if error.kind() == std::io::ErrorKind::NotFound {
                                format!("could not find the `wayexpand` CLI at {} or on PATH; install WayExpand's CLI package and reopen the GUI", cli.display())
                            } else {
                                format!("could not start WayExpand command: {error}")
                            }
                        })?;
                    let mut stdout = child
                        .stdout
                        .take()
                        .ok_or_else(|| "setup stdout was not captured".to_owned())?;
                    let mut stderr = child
                        .stderr
                        .take()
                        .ok_or_else(|| "setup stderr was not captured".to_owned())?;
                    let mut supervisor = ChildSupervisor::new(child);
                    set_nonblocking(&stdout)?;
                    set_nonblocking(&stderr)?;
                    let mut stdout_bytes = Vec::new();
                    let mut stderr_bytes = Vec::new();
                    let mut stdout_eof = false;
                    let mut stderr_eof = false;
                    let deadline = Instant::now() + SETUP_TIMEOUT;
                    let status = loop {
                        if !stdout_eof {
                            stdout_eof = drain_setup_output(&mut stdout, &mut stdout_bytes)?;
                        }
                        if !stderr_eof {
                            stderr_eof = drain_setup_output(&mut stderr, &mut stderr_bytes)?;
                        }
                        if worker_cancel.load(Ordering::Acquire) {
                            supervisor.kill_group();
                            let _ = supervisor.reap();
                            return Err(if worker_is_setup {
                                setup_failure_report(
                                    &stdout_bytes,
                                    &stderr_bytes,
                                    "Setup cancelled",
                                )
                            } else {
                                "Command cancelled".into()
                            });
                        }
                        if supervisor.has_exited().map_err(|error| error.to_string())? {
                            break finish_setup_process(
                                &mut supervisor,
                                &mut stdout,
                                &mut stderr,
                                &mut stdout_bytes,
                                &mut stderr_bytes,
                                stdout_eof,
                                stderr_eof,
                            )?;
                        }
                        if Instant::now() >= deadline {
                            supervisor.kill_group();
                            let _ = supervisor.reap();
                            let timeout = format!(
                                "Command timed out after {} seconds",
                                SETUP_TIMEOUT.as_secs()
                            );
                            return Err(if worker_is_setup {
                                setup_failure_report(&stdout_bytes, &stderr_bytes, &timeout)
                            } else {
                                timeout
                            });
                        }
                        thread::sleep(Duration::from_millis(50));
                    };
                    if worker_is_setup_preflight {
                        Ok(String::from_utf8_lossy(&stdout_bytes).into_owned())
                    } else if status.success() {
                        Ok(Self::setup_output_text(&stdout_bytes))
                    } else if worker_is_setup {
                        Err(setup_failure_report(
                            &stdout_bytes,
                            &stderr_bytes,
                            &format!("Setup exited with status {}", status),
                        ))
                    } else {
                        Err(Self::setup_output_text(&stderr_bytes))
                    }
                })();
                let _ = sender.send(result);
            });
        match spawned {
            Ok(_) => {
                self.setup_task = Some(SetupTask {
                    receiver,
                    cancel,
                    success_message,
                    is_setup,
                });
                self.status = Status::info(running_message);
            }
            Err(error) => {
                if is_setup {
                    let report = setup_failure_report(
                        &[],
                        error.to_string().as_bytes(),
                        "Could not start the setup worker",
                    );
                    let summary = setup_failure_summary(&report);
                    self.setup_failure_report = Some(report);
                    self.setup_failure_report_open = true;
                    self.diagnostics_open = true;
                    self.status = Status::error(self.strings.status_setup_failed(&summary));
                } else {
                    self.status =
                        Status::error(self.strings.status_setup_failed(&error.to_string()));
                }
            }
        }
    }

    pub(crate) fn setup_output_text(bytes: &[u8]) -> String {
        let output = String::from_utf8_lossy(bytes);
        let lines = output.lines().collect::<Vec<_>>();
        if let Some(result) = lines
            .iter()
            .find(|line| line.trim_start().starts_with("DESKTOP TEST RESULT:"))
        {
            let details = lines
                .iter()
                .filter(|line| {
                    let line = line.trim_start();
                    line.starts_with("Capture [")
                        || line.starts_with("Output activity [")
                        || line.starts_with("Correlation [")
                        || line.starts_with("Test scope:")
                })
                .take(4)
                .map(|line| line.trim())
                .collect::<Vec<_>>();
            return if details.is_empty() {
                result.trim().to_owned()
            } else {
                format!("{}\n{}", result.trim(), details.join("\n"))
            };
        }
        if let Some(result) = lines
            .iter()
            .find(|line| line.trim_start().starts_with("SETUP RESULT:"))
        {
            let detail = lines
                .iter()
                .find(|line| line.trim_start().starts_with("Next:"))
                .or_else(|| {
                    lines.iter().find(|line| {
                        line.trim_start()
                            .starts_with("A real expansion was user-confirmed")
                    })
                });
            return detail.map_or_else(
                || result.trim().to_owned(),
                |detail| format!("{}\n{}", result.trim(), detail.trim()),
            );
        }
        lines
            .into_iter()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .trim_start_matches("Error: ")
            .to_owned()
    }

    fn setup_completion_status(output: &str, fallback: String) -> (Status, bool) {
        if output
            .lines()
            .any(|line| line.trim() == "DESKTOP TEST RESULT: OUTPUT OBSERVED, AWAITING REVIEW")
        {
            (
                Status::info(
                    "Output activity observed. Check the target field and confirm the exact text before recording a result.",
                ),
                false,
            )
        } else if output.lines().any(|line| {
            matches!(
                line.trim(),
                "DESKTOP TEST RESULT: INPUT NOT OBSERVED"
                    | "DESKTOP TEST RESULT: NO OUTPUT OUTCOME"
                    | "DESKTOP TEST RESULT: UNCERTAIN OUTPUT"
                    | "DESKTOP TEST RESULT: CANCELLED"
            )
        }) {
            (
                Status::warning(
                    "Desktop test was not verified. Inspect Diagnostics and the target field; do not retry uncertain output automatically.",
                ),
                true,
            )
        } else if output
            .lines()
            .any(|line| line.trim() == "SETUP RESULT: CONFIGURED, AWAITING TEST")
        {
            (
                Status::info(
                    "Configured, awaiting test: run Desktop Integration Test to verify a real insertion.",
                ),
                true,
            )
        } else if output
            .lines()
            .any(|line| line.trim() == "SETUP RESULT: WORKING")
        {
            (
                Status::success(
                    "Working: a real insertion was confirmed in the tested application.",
                ),
                false,
            )
        } else if output
            .lines()
            .any(|line| line.trim() == "SETUP RESULT: ACTION REQUIRED")
        {
            (
                Status::warning(
                    "Setup needs action. Review Diagnostics for the failing prerequisite.",
                ),
                true,
            )
        } else {
            (
                Status::info(format!(
                    "{fallback} Configuration and service state were not enough to verify a real application insertion; check Diagnostics."
                )),
                true,
            )
        }
    }

    fn setup_preflight_result(&mut self, output: &str) -> Status {
        let Ok(report) = serde_json::from_str::<serde_json::Value>(output) else {
            self.diagnostics_open = true;
            return Status::error(
                "Could not read IBus setup checks. Open Diagnostics, refresh the report, and try setup again.",
            );
        };
        let Some(registration_needed) = ibus_registration_needed(&report) else {
            self.diagnostics_open = true;
            return Status::error(
                "IBus setup checks were incomplete. Open Diagnostics and resolve the IBus registration check before continuing.",
            );
        };
        self.setup_system_integration_required = registration_needed;
        self.setup_system_integration_acknowledged = !registration_needed;
        self.setup_consent_acknowledged =
            self.config.settings.compatibility_mode != wayexpand_core::CompatibilityMode::Off;
        self.setup_consent_open = true;
        Status::info("Review the setup stages and any required consent to continue.")
    }

    pub(crate) fn start_desktop_test(
        &mut self,
        trigger: String,
        replacement: String,
        application_class: String,
    ) {
        if self.setup_task.is_some() {
            return;
        }
        self.desktop_test_verified = false;
        self.desktop_test_verified_application = None;
        let arguments = vec![
            "debug".into(),
            "--test".into(),
            trigger.clone(),
            "--application-class".into(),
            application_class.clone(),
            "--focus-confirmed".into(),
            "--observe-only".into(),
        ];
        self.pending_desktop_test = Some(DesktopTestReview {
            trigger,
            replacement,
            application_class,
            capture_activity_observed: false,
            output_activity_observed: false,
        });
        self.start_cli_task(
            arguments,
            "Desktop test running; type the selected trigger once in the confirmed disposable field."
                .into(),
            "Desktop test finished.".into(),
        );
        if self.setup_task.is_some() {
            self.cli_task_kind = Some("desktop-test".into());
        } else {
            self.pending_desktop_test = None;
            self.diagnostics_open = true;
        }
    }

    pub(crate) fn cancel_setup(&mut self) {
        if let Some(task) = self.setup_task.as_ref() {
            task.cancel.store(true, Ordering::Release);
            self.status = Status::info(self.strings.status_setup_cancelling());
        }
    }

    pub(crate) fn poll_setup(&mut self, ctx: &egui::Context) {
        let Some(task) = self.setup_task.as_ref() else {
            return;
        };
        match task.receiver.try_recv() {
            Ok(result) => {
                let success_message = task.success_message.clone();
                let is_setup = task.is_setup;
                self.setup_task = None;
                self.status = match result {
                    Ok(output) => {
                        let task_kind = self.cli_task_kind.take();
                        if task_kind.as_deref() != Some("setup-preflight") {
                            self.last_cli_task_output = Some(output.clone());
                        }
                        if task_kind.as_deref() == Some("setup-preflight") {
                            self.setup_preflight_result(&output)
                        } else {
                            match task_kind.as_deref() {
                                Some("restore-check") => {
                                    self.restore_preview = serde_json::from_str(&output).ok();
                                    self.restore_confirmed = false;
                                }
                                Some("restore-apply") => {
                                    self.restore_preview = None;
                                    self.restore_confirmed = false;
                                }
                                _ => {}
                            }
                            if task_kind.as_deref() == Some("desktop-test") {
                                if output.lines().any(|line| {
                                    line.trim()
                                        == "DESKTOP TEST RESULT: OUTPUT OBSERVED, AWAITING REVIEW"
                                }) {
                                    self.desktop_test_review = self.pending_desktop_test.take();
                                    if let Some(review) = &mut self.desktop_test_review {
                                        review.capture_activity_observed =
                                            output.lines().any(|line| {
                                                line.trim_start().starts_with("Capture [")
                                                    && line.contains("ACTIVITY OBSERVED")
                                            });
                                        review.output_activity_observed =
                                            output.lines().any(|line| {
                                                line.trim_start().starts_with("Output activity [")
                                            });
                                    }
                                    self.desktop_test_review_open =
                                        self.desktop_test_review.is_some();
                                    Status::info("Activity observed but not correlated to the test trigger. Visually verify the exact target text before confirming.")
                                } else {
                                    self.pending_desktop_test = None;
                                    self.diagnostics_open = true;
                                    Self::setup_completion_status(&output, success_message).0
                                }
                            } else {
                                let (status, open_diagnostics) =
                                    Self::setup_completion_status(&output, success_message);
                                self.diagnostics_open |= open_diagnostics;
                                status
                            }
                        }
                    }
                    Err(detail) => {
                        let task_kind = self.cli_task_kind.take();
                        self.pending_desktop_test = None;
                        if is_setup {
                            let report = if detail.starts_with("Setup diagnostic report\n") {
                                detail
                            } else {
                                setup_failure_report(
                                    &[],
                                    detail.as_bytes(),
                                    "Setup could not be started",
                                )
                            };
                            let summary = setup_failure_summary(&report);
                            self.setup_failure_report = Some(report);
                            self.setup_failure_report_open = true;
                            self.diagnostics_open = true;
                            Status::error(self.strings.status_setup_failed(&summary))
                        } else if task_kind.as_deref() == Some("setup-preflight") {
                            self.diagnostics_open = true;
                            Status::error(format!(
                                "Could not check the IBus setup stages: {detail}"
                            ))
                        } else {
                            Status::error(self.strings.status_setup_failed(&detail))
                        }
                    }
                };
                self.refresh_diagnostics(false);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let is_setup = task.is_setup;
                let task_kind = self.cli_task_kind.take();
                self.setup_task = None;
                if is_setup {
                    let report = setup_failure_report(
                        &[],
                        b"Setup worker stopped unexpectedly",
                        "Setup worker stopped",
                    );
                    let summary = setup_failure_summary(&report);
                    self.setup_failure_report = Some(report);
                    self.setup_failure_report_open = true;
                    self.diagnostics_open = true;
                    self.status = Status::error(self.strings.status_setup_failed(&summary));
                } else if task_kind.as_deref() == Some("setup-preflight") {
                    self.diagnostics_open = true;
                    self.status = Status::error(
                        "IBus setup preflight stopped unexpectedly. Open Diagnostics and retry.",
                    );
                } else {
                    self.status = Status::error(self.strings.status_setup_failed("worker stopped"));
                }
            }
        }
    }

    pub(crate) fn start_service_restart(&mut self, unit: String) {
        if self.setup_task.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_unit = unit.clone();
        let spawned = thread::Builder::new()
            .name("wayexpand-service-restart".into())
            .spawn(move || {
                let result = (|| {
                    let active = run_user_systemctl(
                        &["--user", "is-active", "--quiet", worker_unit.as_str()],
                        Duration::from_secs(5),
                        &worker_cancel,
                    )?;
                    if !active.success() {
                        return Err(format!(
                            "{worker_unit} is not active; refusing to start a different route implicitly"
                        ));
                    }
                    let status = run_user_systemctl(
                        &["--user", "restart", worker_unit.as_str()],
                        Duration::from_secs(60),
                        &worker_cancel,
                    )?;
                    if status.success() {
                        Ok(worker_unit)
                    } else {
                        Err(format!("systemctl exited with {status}"))
                    }
                })();
                let _ = sender.send(result);
            });
        match spawned {
            Ok(_) => {
                self.setup_task = Some(SetupTask {
                    receiver,
                    cancel,
                    success_message: format!("{unit} restarted; refreshing diagnostics."),
                    is_setup: false,
                });
                self.status = Status::info(format!("Restarting {unit}…"));
            }
            Err(error) => {
                self.status = Status::error(format!("Could not start service restart: {error}"))
            }
        }
    }

    /// The "Turn on WayExpand" call to action, shown while no daemon runs.
    pub(crate) fn turn_on_button(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let running = self.setup_task.is_some();
        let response = ui
            .add_enabled_ui(!running, |ui| {
                theme::primary_button(
                    ui,
                    palette,
                    if running {
                        self.strings.turning_on()
                    } else {
                        self.strings.turn_on()
                    },
                )
            })
            .inner
            .on_hover_text(self.strings.turn_on_tooltip());
        if response.clicked() {
            let ibus_route = self
                .recommended_route
                .is_some_and(|route| route.setup_backend() == "ibus");
            if ibus_route {
                self.start_cli_task(
                    vec!["ibus".into(), "status".into(), "--json".into()],
                    "Checking whether IBus registration or replacement consent is needed…".into(),
                    "IBus preflight complete.".into(),
                );
                if self.setup_task.is_some() {
                    self.cli_task_kind = Some("setup-preflight".into());
                }
            } else if self.recommended_route.is_none() {
                self.refresh_diagnostics(false);
                self.status = Status::info(
                    "Checking the recommended input route. Try setup again when that check finishes.",
                );
            } else {
                self.start_setup();
            }
        }
        if running && ui.small_button(self.strings.cancel()).clicked() {
            self.cancel_setup();
        }
    }

    pub(crate) fn render_setup_consent(&mut self, ctx: &egui::Context) {
        if !self.setup_consent_open {
            return;
        }
        let (input_title, input_detail, tracking_title, tracking_detail) =
            match wayexpand_backend_selection::Compositor::detect() {
                wayexpand_backend_selection::Compositor::Gnome => (
                    "Register and select the IBus source",
                    "Keep your existing keyboard sources and layout in GNOME.",
                    "Enable GNOME window tracking",
                    "Used for app-specific snippets; ordinary snippets do not depend on it. No administrator access is needed.",
                ),
                wayexpand_backend_selection::Compositor::KdePlasma => (
                    "Connect IBus to the KDE virtual keyboard",
                    "WayExpand will preserve your configured keyboard layout.",
                    "Use KWin window tracking",
                    "Used for app-specific snippets; ordinary snippets do not depend on it.",
                ),
                compositor => (
                    "Activate the IBus input source",
                    "Your existing keyboard configuration is left in place.",
                    "Check application window tracking",
                    if compositor == wayexpand_backend_selection::Compositor::Unknown {
                        "App-specific snippets need a supported window tracker; Diagnostics will show whether one is available."
                    } else {
                        "App-specific snippets need a supported window tracker; ordinary snippets do not depend on it."
                    },
                ),
            };
        egui::Modal::new(egui::Id::new("ibus_replacement_consent")).show(ctx, |ui| {
                ui.set_width(430.0);
                ui.heading("Set up WayExpand");
                ui.add_space(8.0);
                for (number, title, detail) in [
                    (
                        "1",
                        "Prepare WayExpand",
                        "Use the installed files and create your configuration if needed.",
                    ),
                    (
                        "2",
                        input_title,
                        input_detail,
                    ),
                    (
                        "3",
                        "Activate and check the input engine",
                        "Then test a real expansion in a text field.",
                    ),
                    (
                        "4",
                        tracking_title,
                        tracking_detail,
                    ),
                ] {
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new(number).strong());
                        ui.vertical(|ui| {
                            ui.label(RichText::new(title).strong());
                            ui.label(detail);
                        });
                    });
                }
                ui.add_space(8.0);
                if self.setup_system_integration_required {
                    ui.add_space(8.0);
                    ui.label("IBus does not yet have a valid WayExpand component registration. WayExpand will install it only if still missing; administrator authorization is requested only if the component directory requires it.");
                    ui.checkbox(
                        &mut self.setup_system_integration_acknowledged,
                        "Allow WayExpand to request administrator authorization for IBus registration if needed.",
                    );
                } else {
                    ui.add_space(8.0);
                    ui.label("IBus registration is already valid. No administrator authorization is needed for this step.");
                }
                if self.config.settings.compatibility_mode
                    == wayexpand_core::CompatibilityMode::Off
                {
                    ui.add_space(8.0);
                    ui.label("Text replacement is not atomic: WayExpand removes a recognized trigger and then inserts its snippet. If the application loses focus or insertion is interrupted, already-sent text cannot be rolled back.");
                    ui.checkbox(
                        &mut self.setup_consent_acknowledged,
                        "I understand and want to enable tracked-focus text replacement.",
                    );
                } else {
                    ui.add_space(8.0);
                    ui.label("Tracked-focus text replacement is already enabled.");
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            (!self.setup_system_integration_required
                                || self.setup_system_integration_acknowledged)
                                && self.setup_consent_acknowledged
                                && self.setup_task.is_none(),
                            egui::Button::new("Continue setup"),
                        )
                        .clicked()
                    {
                        self.setup_consent_open = false;
                        let replacement_consent = self.config.settings.compatibility_mode
                            == wayexpand_core::CompatibilityMode::Off;
                        self.start_setup_with_replacement_consent(
                            replacement_consent,
                            self.setup_system_integration_required
                                && self.setup_system_integration_acknowledged,
                        );
                    }
                    if ui.button("Cancel").clicked() {
                        self.setup_consent_open = false;
                        self.setup_consent_acknowledged = false;
                        self.setup_system_integration_required = false;
                        self.setup_system_integration_acknowledged = false;
                    }
                });
            });
    }

    pub(crate) fn render_evdev_setup(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.evdev_setup_open {
            return;
        }
        egui::Window::new(self.strings.evdev_setup_title())
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(self.strings.evdev_setup_warning()).color(palette.warning));
                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.evdev_setup_acknowledged,
                    self.strings.evdev_setup_acknowledge(),
                );
                ui.add_space(8.0);
                ui.label(self.strings.evdev_setup_steps());
                if self.evdev_setup_acknowledged {
                    ui.horizontal(|ui| {
                        ui.monospace("wayexpand setup --mode maximum");
                        if ui.button(self.strings.copy()).clicked() {
                            ui.ctx().copy_text("wayexpand setup --mode maximum".into());
                            self.status = Status::info(self.strings.status_preview_copied());
                        }
                    });
                    ui.label(
                        RichText::new(self.strings.command_not_run_by_gui())
                            .small()
                            .color(palette.muted),
                    );
                }
                ui.add_space(8.0);
                if ui.button(self.strings.close()).clicked() {
                    self.evdev_setup_open = false;
                }
            });
    }
}

/// A numbered onboarding step heading, with a check mark once it is done.
pub(crate) fn onboarding_step(
    ui: &mut egui::Ui,
    palette: &Palette,
    number: u8,
    title: &str,
    done: bool,
) {
    ui.horizontal(|ui| {
        let (badge, color) = if done {
            ("✓".to_owned(), palette.success)
        } else {
            (number.to_string(), palette.accent)
        };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 12.0, theme::tint(color, 50));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            badge,
            egui::FontId::proportional(13.0),
            color,
        );
        ui.label(RichText::new(title).strong().size(16.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_registration_consent_is_needed_only_when_component_checks_fail() {
        let registered = serde_json::json!({
            "checks": [
                {"id": "component", "state": "pass"},
                {"id": "manifest", "state": "pass"}
            ]
        });
        assert_eq!(ibus_registration_needed(&registered), Some(false));

        let missing = serde_json::json!({
            "checks": [
                {"id": "component", "state": "fail"},
                {"id": "manifest", "state": "skip"}
            ]
        });
        assert_eq!(ibus_registration_needed(&missing), Some(true));

        assert_eq!(
            ibus_registration_needed(&serde_json::json!({"schema": 2})),
            None
        );
    }

    struct ContinuousOutput {
        reads: usize,
    }

    impl Read for ContinuousOutput {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            buffer[0] = b'x';
            Ok(1)
        }
    }

    #[test]
    fn setup_output_drain_yields_for_continuous_output() {
        let mut reader = ContinuousOutput { reads: 0 };
        let mut output = Vec::new();
        assert_eq!(
            drain_setup_output(&mut reader, &mut output),
            Ok(false),
            "continuously readable output must yield after the per-turn read budget"
        );
        assert_eq!(reader.reads, SETUP_OUTPUT_READS_PER_DRAIN);
        assert_eq!(output.len(), SETUP_OUTPUT_READS_PER_DRAIN);
    }

    #[test]
    fn setup_output_retains_operational_result_and_next_step() {
        let output = b"WayExpand setup\nSETUP RESULT: CONFIGURED, AWAITING TEST\nNext: run wayexpand debug --test ';;hello'.\n";
        let visible = GuiApp::setup_output_text(output);
        assert!(visible.contains("SETUP RESULT: CONFIGURED, AWAITING TEST"));
        assert!(visible.contains("Next: run wayexpand debug"));
    }

    #[test]
    fn failed_setup_report_preserves_both_streams_and_actionable_fields() {
        let stdout = b"Step 3: activating IBus\nSetup rollback was incomplete. Changes remaining: engine source.\n";
        let stderr = b"Error: WX-GNOME-IBUS-RESTART: engine did not become available\nFix: run wayexpand setup --check\n";
        let report = setup_failure_report(stdout, stderr, "Setup exited with status 1");
        assert!(report.contains("Summary: WX-GNOME-IBUS-RESTART"));
        assert!(report.contains("Failing stage: Step 3: activating IBus"));
        assert!(report.contains("Problem ID: WX-GNOME-IBUS-RESTART"));
        assert!(report.contains("Suggested remedy: Fix: run wayexpand setup --check"));
        assert!(report.contains("Rollback: Incomplete"));
        assert!(report.contains("--- stdout ---\nStep 3: activating IBus"));
        assert!(report.contains("--- stderr ---\nError: WX-GNOME-IBUS-RESTART"));
        assert_eq!(
            setup_failure_summary(&report),
            "WX-GNOME-IBUS-RESTART: engine did not become available"
        );
    }

    #[test]
    fn failed_setup_without_cli_metadata_gets_stable_problem_id_and_remedy() {
        let report = setup_failure_report(&[], b"permission denied", "setup command failed");
        assert!(report.contains("Problem ID: WX-SETUP-001"));
        assert!(report.contains("Suggested remedy: Run `wayexpand setup --check`"));
        assert!(report.contains("Rollback: Not reported"));
    }

    #[test]
    fn setup_output_retains_real_test_result_even_if_optional_steps_follow() {
        let output = b"SETUP RESULT: WORKING\nA real expansion was user-confirmed in the tested application.\nOptional terminal coverage is not configured.\n";
        let visible = GuiApp::setup_output_text(output);
        assert!(visible.contains("SETUP RESULT: WORKING"));
        assert!(visible.contains("A real expansion was user-confirmed"));
    }

    #[test]
    fn desktop_test_output_keeps_the_observed_stage_without_input_content() {
        let output = b"For the next 30 seconds, type once.\nCapture [id]: ACTIVITY OBSERVED -- test trigger not independently confirmed.\nOutput activity [id]: uncorrelated completion.\nDESKTOP TEST RESULT: OUTPUT OBSERVED, AWAITING REVIEW\n";
        let visible = GuiApp::setup_output_text(output);
        assert!(visible.contains("DESKTOP TEST RESULT: OUTPUT OBSERVED, AWAITING REVIEW"));
        assert!(visible.contains("Capture [id]: ACTIVITY OBSERVED"));
        assert!(!visible.contains("trigger contents"));
    }

    #[test]
    fn setup_status_does_not_call_an_unverified_route_working() {
        let (status, open_diagnostics) = GuiApp::setup_completion_status(
            "SETUP RESULT: CONFIGURED, AWAITING TEST",
            "generic success".into(),
        );
        assert_eq!(
            status.text(),
            "Configured, awaiting test: run Desktop Integration Test to verify a real insertion."
        );
        assert!(!status.is_error());
        assert!(open_diagnostics);
    }

    #[test]
    fn setup_status_reports_working_only_after_a_confirmed_test() {
        let (status, open_diagnostics) =
            GuiApp::setup_completion_status("SETUP RESULT: WORKING", "generic success".into());
        assert_eq!(
            status.text(),
            "Working: a real insertion was confirmed in the tested application."
        );
        assert!(!open_diagnostics);
    }

    #[test]
    fn generic_setup_completion_is_never_presented_as_verified_success() {
        let (status, open_diagnostics) =
            GuiApp::setup_completion_status("IBus component installed", "Setup finished.".into());
        assert!(!status.is_error());
        assert!(status
            .text()
            .contains("not enough to verify a real application insertion"));
        assert!(open_diagnostics);
    }

    #[test]
    fn desktop_test_without_activity_opens_diagnostics_not_success() {
        let (status, open_diagnostics) = GuiApp::setup_completion_status(
            "DESKTOP TEST RESULT: INPUT NOT OBSERVED",
            "generic success".into(),
        );
        assert!(status.text().contains("Desktop test was not verified"));
        assert!(open_diagnostics);
    }
}
