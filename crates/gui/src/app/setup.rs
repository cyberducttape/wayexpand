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
    // A system component that invokes one user's home-directory binary is
    // registered, but is not a valid shared installation. Keep the explicit
    // administrator-integration step available for this unsafe legacy state.
    let manifest_scope_valid = state("manifest-scope") == Some("pass");
    Some(!(component_valid && manifest_valid && manifest_scope_valid))
}

fn configure_cli_task_command(command: &mut std::process::Command, config_path: &std::path::Path) {
    // Setup's restart marker and every preflight must refer to the same config
    // the GUI opened. Otherwise a custom CONFIG launch can prepare a marker in
    // the default config directory that this GUI never sees after login.
    command.env("WAYEXPAND_CONFIG", config_path);
}

pub(super) fn current_guided_setup_backend(
    daemon_reachable: Option<bool>,
    daemon_status: &str,
    route_plan: Option<&wayexpand_backend_selection::RoutePlan>,
) -> Option<&'static str> {
    if daemon_reachable != Some(true) {
        return None;
    }
    let status = wayexpand_core::DaemonStatus::parse(daemon_status);
    if status.status_schema() != Some(wayexpand_core::CONTROL_STATUS_SCHEMA)
        || !status.field_is_unique("source")
    {
        return None;
    }
    let backend = match status.field("source")? {
        "evdev" => "evdev",
        "input-method" | "input-method-v2" => "input-method",
        // The control-only service is not a typing route. IBus readiness and
        // the GNOME source checks determine that path separately.
        "stdin" => return None,
        _ => return None,
    };
    let route_is_available = route_plan?.routes.iter().any(|route| {
        route.contract.setup_backend == backend
            && route.assessment.policy_allowed
            && route.standing != wayexpand_backend_selection::RouteStanding::BlockedByPolicy
            && route.standing != wayexpand_backend_selection::RouteStanding::Unavailable
    });
    route_is_available.then_some(backend)
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
    /// Run setup off the UI thread. The additional IBus flags are included
    /// only after the user explicitly accepts replacement and, when needed,
    /// system-wide manifest registration.
    pub(crate) fn start_setup(&mut self) {
        self.desktop_test_verified = false;
        self.desktop_test_verified_application = None;
        self.start_setup_with_replacement_consent(false, false, None);
    }

    fn start_setup_with_replacement_consent(
        &mut self,
        replacement_consent: bool,
        system_integration_consent: bool,
        backend: Option<&str>,
    ) {
        let mut arguments = vec!["setup".into(), "--yes".into()];
        if let Some(backend) = backend {
            arguments.push("--backend".into());
            arguments.push(backend.into());
        }
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

    pub(crate) fn start_guided_setup(&mut self) {
        let situation = self.situation();
        // Desktop-managed IBus can cover browser/editor fields that a
        // terminal-restricted evdev route cannot. When IBus readiness is the
        // reported blocker, guided setup must preflight that route instead of
        // reopening setup for the currently running terminal backend.
        let ibus_route = self.gnome_setup_resume_pending
            || situation.gnome_ibus
            || situation.ibus_route_expected
            || self
                .recommended_route
                .is_some_and(|route| route.setup_backend() == "ibus");
        let current_backend = current_guided_setup_backend(
            self.daemon_reachable,
            &self.daemon_status,
            self.route_plan.as_ref(),
        );
        if !ibus_route {
            if let Some(backend) = current_backend.filter(|backend| *backend != "ibus") {
                self.guided_setup_waiting_for_route = false;
                self.start_setup_with_replacement_consent(false, false, Some(backend));
                return;
            }
        }
        if ibus_route {
            self.setup_ibus_preflight_report = None;
            self.start_cli_task(
                vec!["ibus".into(), "status".into(), "--json".into()],
                self.strings.setup_checking_ibus().into(),
                self.strings.setup_preflight_complete().into(),
            );
            if self.setup_task.is_some() {
                self.cli_task_kind = Some("setup-preflight".into());
            }
        } else if self.recommended_route.is_none() {
            self.guided_setup_waiting_for_route = true;
            self.refresh_diagnostics(false);
            self.status = Status::info(self.strings.setup_waiting_for_routes());
        } else {
            self.guided_setup_waiting_for_route = false;
            self.start_setup();
        }
    }

    /// Verify the active CLI, daemon and managed service before offering any
    /// restart. This avoids restarting a unit selected only from a stale or
    /// unverifiable route report.
    pub(crate) fn check_upgrade_runtime(&mut self) {
        self.start_cli_task(
            vec!["upgrade".into(), "--check".into(), "--json".into()],
            self.strings.dg_software_versions().into(),
            self.strings.dg_software_versions().into(),
        );
        if self.setup_task.is_some() {
            self.cli_task_kind = Some("upgrade-check".into());
        }
    }

    /// Inspect the installation's explicitly safe repairs before changing
    /// anything. The persistent next-step banner uses this path so users can
    /// review and apply missing user-service links without opening a terminal.
    pub(crate) fn check_installation_repairs(&mut self) {
        if self.setup_task.is_some() {
            return;
        }
        self.start_cli_task(
            vec!["repair".into(), "--check".into(), "--json".into()],
            self.strings.repair_check_running().into(),
            self.strings.repair_check_running().into(),
        );
        if self.setup_task.is_some() {
            self.cli_task_kind = Some("repair-check".into());
        }
    }

    fn finish_installation_repair_check(&mut self, output: &str) -> Status {
        let Ok(report) = serde_json::from_str::<serde_json::Value>(output) else {
            self.diagnostics_open = true;
            return Status::warning(self.strings.repair_check_unreadable());
        };
        let repairs = report.get("repairs").and_then(serde_json::Value::as_array);
        if report.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
            || report.get("mode").and_then(serde_json::Value::as_str) != Some("check")
            || repairs.is_none()
        {
            self.diagnostics_open = true;
            return Status::warning(self.strings.repair_check_unreadable());
        }
        let repairs = repairs.expect("validated repair array");
        if repairs.is_empty() {
            self.check_upgrade_runtime();
            return Status::info(self.strings.no_safe_repairs_available());
        }
        if repairs.iter().any(|repair| {
            repair.get("status").and_then(serde_json::Value::as_str) != Some("available")
                || repair
                    .get("component")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(str::is_empty)
                || repair
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(str::is_empty)
        }) {
            self.diagnostics_open = true;
            return Status::warning(self.strings.repair_check_unreadable());
        }
        self.installation_repair_preview = Some(report);
        self.installation_repair_dialog_open = true;
        Status::info(self.strings.repair_review_ready())
    }

    pub(crate) fn apply_installation_repairs(&mut self) {
        if self.setup_task.is_some() || self.installation_repair_preview.is_none() {
            return;
        }
        self.start_cli_task(
            vec!["repair".into(), "--apply".into(), "--json".into()],
            self.strings.repair_running().into(),
            self.strings.repair_running().into(),
        );
        if self.setup_task.is_some() {
            self.cli_task_kind = Some("repair-apply".into());
        }
    }

    fn finish_installation_repair_apply(&mut self, output: &str) -> Status {
        let Ok(report) = serde_json::from_str::<serde_json::Value>(output) else {
            self.installation_repair_preview = None;
            self.installation_repair_dialog_open = false;
            self.diagnostics_open = true;
            return Status::warning(self.strings.repair_result_unreadable());
        };
        let repairs = report.get("repairs").and_then(serde_json::Value::as_array);
        let valid_apply_report = report.get("schema").and_then(serde_json::Value::as_u64)
            == Some(1)
            && report.get("mode").and_then(serde_json::Value::as_str) == Some("apply")
            && repairs.is_some_and(|repairs| {
                repairs.iter().all(|repair| {
                    repair.get("status").and_then(serde_json::Value::as_str) == Some("repaired")
                })
            });
        let repairs_applied =
            valid_apply_report && repairs.is_some_and(|repairs| !repairs.is_empty());
        let no_repairs_remained = valid_apply_report && repairs.is_some_and(Vec::is_empty);
        self.installation_repair_preview = None;
        self.installation_repair_dialog_open = false;
        if repairs_applied {
            self.refresh_diagnostics(false);
            Status::info(self.strings.repair_completed())
        } else if no_repairs_remained {
            self.refresh_diagnostics(false);
            Status::info(self.strings.repair_already_completed())
        } else {
            self.diagnostics_open = true;
            Status::warning(self.strings.repair_result_unreadable())
        }
    }

    fn finish_upgrade_check(&mut self, output: &str) -> Status {
        let Ok(report) = serde_json::from_str::<serde_json::Value>(output) else {
            self.diagnostics_open = true;
            return Status::error(self.strings.upgrade_check_unreadable());
        };
        let installation_check = report.get("installation_check");
        let installation_check_schema = installation_check
            .and_then(|check| check.get("schema"))
            .and_then(serde_json::Value::as_u64);
        let installation_ready = installation_check
            .and_then(|check| check.get("ready"))
            .and_then(serde_json::Value::as_bool);
        if installation_check_schema != Some(2)
            || installation_check
                .and_then(|check| check.get("level"))
                .and_then(serde_json::Value::as_str)
                != Some("installed")
        {
            self.diagnostics_open = true;
            return Status::error(self.strings.upgrade_check_unreadable());
        }
        if installation_ready != Some(true) {
            self.diagnostics_open = true;
            return Status::warning(self.strings.upgrade_installation_incomplete());
        }
        let active_cli = report
            .get("active_payload_cli")
            .and_then(serde_json::Value::as_str);
        if report
            .get("cli_matches_current_install")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        {
            self.diagnostics_open = true;
            return Status::warning(self.strings.upgrade_cli_mismatch(active_cli));
        }
        match report
            .get("runtime_action")
            .and_then(serde_json::Value::as_str)
        {
            Some("restart_daemon_service") => {
                let unit = report
                    .get("restart_process")
                    .and_then(|process| process.get("unit"))
                    .and_then(serde_json::Value::as_str);
                if let Some(unit) = unit {
                    self.service_restart_unit = Some(unit.to_owned());
                    self.service_restart_confirm_open = true;
                    Status::warning(self.strings.upgrade_restart_available())
                } else {
                    self.diagnostics_open = true;
                    Status::warning(self.strings.upgrade_restart_unmanaged())
                }
            }
            Some("restart_daemon_manually") => {
                self.diagnostics_open = true;
                Status::warning(self.strings.upgrade_restart_unmanaged())
            }
            Some("run_current_install_cli" | "verify_current_install_cli") => {
                self.diagnostics_open = true;
                Status::warning(self.strings.upgrade_cli_mismatch(active_cli))
            }
            Some("none") => {
                self.refresh_diagnostics(false);
                Status::success(self.strings.upgrade_build_current())
            }
            _ => {
                self.diagnostics_open = true;
                Status::warning(self.strings.upgrade_check_unreadable())
            }
        }
    }

    /// Give the Quick Picker the default global shortcut through
    /// `wayexpand shortcut set`, which refuses a combination already in use.
    pub(crate) fn set_picker_shortcut(&mut self) {
        self.picker_shortcut_error = None;
        self.start_cli_task(
            vec!["shortcut".into(), "set".into()],
            self.strings.shortcut_setting().into(),
            self.strings.shortcut_set_done().into(),
        );
        if self.setup_task.is_some() {
            self.cli_task_kind = Some("picker-shortcut".into());
        }
    }

    pub(crate) fn retry_gnome_source_activation(&mut self) {
        self.start_cli_task(
            vec!["repair".into(), "--gnome-input-source".into()],
            self.strings.setup_selecting_gnome().into(),
            self.strings.setup_gnome_selected_refreshing().into(),
        );
        if self.setup_task.is_some() {
            self.cli_task_kind = Some("gnome-source-repair".into());
        }
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
        let worker_config_path = self.path.clone();
        let cli = runtime::cli_executable();
        let spawned = thread::Builder::new()
            .name("wayexpand-cli-task".into())
            .spawn(move || {
                let result = (|| {
                    let mut command = std::process::Command::new(&cli);
                    configure_cli_task_command(&mut command, &worker_config_path);
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
                        || line.starts_with("Problem:")
                        || line.starts_with("Next:")
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

    fn setup_completion_status(
        strings: &Strings,
        output: &str,
        fallback: String,
    ) -> (Status, bool) {
        if output.contains("WX-GNOME-SESSION-RESTART-001")
            || output
                .to_ascii_lowercase()
                .contains("log out of gnome and log back in")
            || output
                .to_ascii_lowercase()
                .contains("restart your gnome session")
        {
            (
                Status::warning(strings.setup_needs_desktop_restart()),
                false,
            )
        } else if output.contains("WX-POLICY-001") {
            (
                Status::warning(strings.setup_replacement_consent_needed()),
                false,
            )
        } else if output
            .lines()
            .any(|line| line.trim() == "DESKTOP TEST RESULT: OUTPUT OBSERVED, AWAITING REVIEW")
        {
            (Status::info(strings.setup_output_observed()), false)
        } else if output.lines().any(|line| {
            matches!(
                line.trim(),
                "DESKTOP TEST RESULT: INPUT NOT OBSERVED"
                    | "DESKTOP TEST RESULT: NO OUTPUT OUTCOME"
                    | "DESKTOP TEST RESULT: UNCERTAIN OUTPUT"
                    | "DESKTOP TEST RESULT: CONTROLLED GTK TEST FAILED"
                    | "DESKTOP TEST RESULT: CANCELLED"
            )
        }) {
            (
                Status::warning(strings.setup_desktop_test_unverified()),
                true,
            )
        } else if output
            .lines()
            .any(|line| line.trim() == "SETUP RESULT: CONFIGURED, AWAITING TEST")
        {
            (Status::info(strings.setup_configured_untested()), true)
        } else if output
            .lines()
            .any(|line| line.trim() == "SETUP RESULT: WORKING")
        {
            (Status::success(strings.setup_working()), false)
        } else if output
            .lines()
            .any(|line| line.trim() == "SETUP RESULT: ACTION REQUIRED")
        {
            (Status::warning(strings.setup_needs_action()), true)
        } else {
            (
                Status::info(strings.setup_unverified_fallback(&fallback)),
                true,
            )
        }
    }

    fn setup_preflight_result(&mut self, output: &str) -> Status {
        let Ok(report) = serde_json::from_str::<serde_json::Value>(output) else {
            self.diagnostics_open = true;
            return Status::error(self.strings.setup_checks_unreadable());
        };
        let Some(registration_needed) = ibus_registration_needed(&report) else {
            self.diagnostics_open = true;
            return Status::error(self.strings.setup_checks_incomplete());
        };
        // The consent dialog must show findings from the preflight that
        // opened it, not a potentially older background diagnostics snapshot.
        self.setup_ibus_preflight_report = Some(report);
        self.setup_system_integration_required = registration_needed;
        self.setup_system_integration_acknowledged = !registration_needed;
        self.setup_consent_acknowledged =
            self.config.settings.compatibility_mode != wayexpand_core::CompatibilityMode::Off;
        self.setup_consent_open = true;
        Status::info(self.strings.setup_review_stages())
    }

    pub(crate) fn select_default_desktop_test(&mut self) -> bool {
        if let Some(expansion) = super::first_run::onboarding_test_snippet(&self.config) {
            self.desktop_test_trigger = expansion.trigger.clone();
            true
        } else {
            self.desktop_test_trigger.clear();
            false
        }
    }

    fn finish_controlled_desktop_test(&mut self) -> Status {
        let Some(mut review) = self.pending_desktop_test.take() else {
            self.diagnostics_open = true;
            return Status::warning(self.strings.setup_desktop_test_unverified());
        };
        review.application_class = "GTK 3 controlled test field".into();
        let tested_scope = crate::setup_record::Scope {
            route: review.route.as_deref(),
            route_instance: review.route_instance.as_deref(),
            config_revision: Some(&review.config_revision),
            desktop: &review.desktop,
            session: review.session.as_deref(),
            build: &review.build,
        };
        let record = crate::setup_record::SetupRecord::machine_verified(
            &review.application_class,
            &tested_scope,
        );
        let route = self.current_route_label();
        let route_instance = self.current_route_instance();
        let desktop = crate::setup_record::current_desktop();
        let session = crate::setup_record::current_session();
        let build = crate::setup_record::current_build();
        let config_revision = self.config_revision.fingerprint();
        let current_scope = crate::setup_record::Scope {
            route: route.as_deref(),
            route_instance: route_instance.as_deref(),
            config_revision: Some(&config_revision),
            desktop: &desktop,
            session: session.as_deref(),
            build: &build,
        };
        if let Some(staleness) = record.staleness(&current_scope) {
            self.desktop_test_verified = false;
            self.desktop_test_verified_application = None;
            return Status::warning(
                self.strings
                    .setup_changed_since_test(&self.strings.setup_change(&staleness)),
            );
        }

        self.desktop_test_verified = true;
        self.desktop_test_verified_application = Some(review.application_class.clone());
        self.setup_record_saved = crate::setup_record::save(&record).is_ok();
        self.setup_record = Some(record);
        let message = crate::lang::fill(
            self.strings.dg_test_field_observed_exact_output_in_this(),
            &[&review.application_class],
        );
        if self.setup_record_saved && (self.desktop_test_dialog_required || self.onboarding_visible)
        {
            self.onboarding_visible = true;
            self.finish_onboarding();
        } else if !self.setup_record_saved && self.desktop_test_dialog_required {
            self.onboarding_visible = true;
        }
        if self.setup_record_saved {
            Status::success(message)
        } else {
            Status::warning(format!(
                "{message} {}",
                self.strings.verification_not_saved()
            ))
        }
    }

    /// Show the matcher preview for the same saved snippet used by the
    /// onboarding desktop test. Preserve unsaved editor work rather than
    /// silently switching the selected snippet underneath the user.
    pub(crate) fn open_onboarding_matcher_preview(&mut self) {
        if self.draft_is_dirty() {
            self.status = Status::warning(self.strings.status_save_before_matcher_preview());
            return;
        }
        let Some(expansion) = super::first_run::onboarding_test_snippet(&self.config) else {
            self.status = Status::warning(self.strings.status_create_test_expansion_first());
            return;
        };
        let Some(index) = self
            .config
            .expansion
            .iter()
            .position(|candidate| candidate.id == expansion.id)
        else {
            self.status = Status::warning(self.strings.status_create_test_expansion_first());
            return;
        };

        self.select(index);
        self.invalidate_preview();
        self.playground.clear();
        self.playground.invalidate();
        self.try_live_open = true;
        self.status = Status::info(self.strings.matcher_preview_scope());
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
        let mut arguments = vec![
            "debug".into(),
            "--test".into(),
            trigger.clone(),
            "--application-class".into(),
            application_class.clone(),
            "--focus-confirmed".into(),
            "--observe-only".into(),
        ];
        // First-run uses the dedicated GTK field, whose contents can be
        // checked exactly without recording them. Diagnostics lets the user
        // choose a browser, terminal, or other application; that test must
        // observe the selected application instead of silently opening GTK.
        if self.desktop_test_dialog_required && !self.active_route_is_terminal_scoped() {
            arguments.push("--controlled-client".into());
        }
        self.pending_desktop_test = Some(DesktopTestReview {
            trigger,
            replacement,
            application_class,
            route: self.current_route_label(),
            route_instance: self.current_route_instance(),
            desktop: crate::setup_record::current_desktop(),
            session: crate::setup_record::current_session(),
            build: crate::setup_record::current_build(),
            config_revision: self.config_revision.fingerprint(),
            capture_activity_observed: false,
            output_activity_observed: false,
        });
        self.start_cli_task(
            arguments,
            self.strings.setup_desktop_test_running().into(),
            self.strings.setup_desktop_test_finished().into(),
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
                        let onboarding_state_error = if is_setup
                            && output.lines().any(|line| {
                                line.trim() == "SETUP RESULT: CONFIGURED, AWAITING TEST"
                            }) {
                            self.require_desktop_test()
                        } else {
                            None
                        };
                        if is_setup {
                            self.gnome_setup_resume_pending =
                                crate::gnome_setup_resume_pending_for_current_session(&self.path);
                        }
                        let task_kind = self.cli_task_kind.take();
                        if task_kind.as_deref() != Some("setup-preflight") {
                            self.last_cli_task_output = Some(output.clone());
                        }
                        if task_kind.as_deref() == Some("setup-preflight") {
                            self.setup_preflight_result(&output)
                        } else if task_kind.as_deref() == Some("upgrade-check") {
                            self.finish_upgrade_check(&output)
                        } else if task_kind.as_deref() == Some("repair-check") {
                            self.finish_installation_repair_check(&output)
                        } else if task_kind.as_deref() == Some("repair-apply") {
                            self.finish_installation_repair_apply(&output)
                        } else if task_kind.as_deref() == Some("picker-shortcut") {
                            self.picker_shortcut_error = None;
                            Status::success(success_message)
                        } else if task_kind.as_deref() == Some("gnome-source-repair") {
                            self.desktop_test_verified = false;
                            self.desktop_test_verified_application = None;
                            if self.desktop_test_dialog_required {
                                Status::info(self.strings.setup_gnome_selected_first_run())
                            } else {
                                Status::info(self.strings.setup_gnome_selected())
                            }
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
                                        == "DESKTOP TEST RESULT: VERIFIED IN CONTROLLED GTK 3 FIELD"
                                }) {
                                    self.desktop_test_review = None;
                                    self.desktop_test_review_open = false;
                                    self.diagnostics_open = false;
                                    self.finish_controlled_desktop_test()
                                } else if output.lines().any(|line| {
                                    line.trim()
                                        == "DESKTOP TEST RESULT: OUTPUT OBSERVED, AWAITING REVIEW"
                                }) {
                                    self.desktop_test_review = self.pending_desktop_test.take();
                                    if let Some(review) = &mut self.desktop_test_review {
                                        if output.lines().any(|line| {
                                            line.contains("Test field [")
                                                && line.contains("GTK 3 controlled test field")
                                        }) {
                                            review.application_class =
                                                "GTK 3 controlled test field".into();
                                        }
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
                                    Status::info(self.strings.setup_activity_uncorrelated())
                                } else {
                                    self.pending_desktop_test = None;
                                    self.diagnostics_open = true;
                                    Self::setup_completion_status(
                                        &self.strings,
                                        &output,
                                        success_message,
                                    )
                                    .0
                                }
                            } else {
                                let (status, open_diagnostics) = Self::setup_completion_status(
                                    &self.strings,
                                    &output,
                                    success_message,
                                );
                                if is_setup
                                    && output.lines().any(|line| {
                                        line.trim() == "SETUP RESULT: CONFIGURED, AWAITING TEST"
                                    })
                                {
                                    self.select_default_desktop_test();
                                    self.test_expansion_dialog_open = true;
                                    self.diagnostics_open = false;
                                    if let Some(error) = onboarding_state_error {
                                        Status::warning(
                                            self.strings
                                                .setup_unverified_state_unsaved(&error.to_string()),
                                        )
                                    } else {
                                        Status::info(self.strings.setup_configured_unverified())
                                    }
                                } else {
                                    self.diagnostics_open |= open_diagnostics;
                                    status
                                }
                            }
                        }
                    }
                    Err(detail) => {
                        let task_kind = self.cli_task_kind.take();
                        self.pending_desktop_test = None;
                        if is_setup {
                            self.gnome_setup_resume_pending =
                                crate::gnome_setup_resume_pending_for_current_session(&self.path);
                            if self.gnome_setup_resume_pending {
                                self.onboarding_visible = true;
                            }
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
                            let outcome = Self::setup_completion_status(
                                &self.strings,
                                &report,
                                self.strings.status_setup_failed(&summary),
                            )
                            .0;
                            let (_, open_diagnostics) = Self::setup_completion_status(
                                &self.strings,
                                &report,
                                self.strings.status_setup_failed(&summary),
                            );
                            self.setup_failure_report = Some(report);
                            self.setup_failure_report_open = true;
                            self.diagnostics_open = open_diagnostics;
                            outcome
                        } else if task_kind.as_deref() == Some("picker-shortcut") {
                            // Usually a combination already in use; the CLI
                            // names it and suggests another.
                            let reason =
                                detail.trim().trim_start_matches("Error:").trim().to_owned();
                            self.picker_shortcut_error = Some(reason.clone());
                            Status::warning(reason)
                        } else if task_kind.as_deref() == Some("setup-preflight") {
                            self.diagnostics_open = true;
                            Status::error(self.strings.setup_could_not_check_stages(&detail))
                        } else if task_kind.as_deref() == Some("upgrade-check") {
                            self.diagnostics_open = true;
                            Status::error(self.strings.upgrade_check_unreadable())
                        } else if task_kind.as_deref() == Some("repair-check") {
                            self.diagnostics_open = true;
                            Status::warning(self.strings.repair_check_unreadable())
                        } else if task_kind.as_deref() == Some("repair-apply") {
                            self.installation_repair_preview = None;
                            self.installation_repair_dialog_open = false;
                            self.diagnostics_open = true;
                            Status::warning(self.strings.repair_result_unreadable())
                        } else if task_kind.as_deref() == Some("gnome-source-repair") {
                            let report = setup_failure_report(
                                &[],
                                detail.as_bytes(),
                                "GNOME input-source activation failed",
                            );
                            let summary = setup_failure_summary(&report);
                            let outcome = Self::setup_completion_status(
                                &self.strings,
                                &report,
                                self.strings.status_setup_failed(&summary),
                            )
                            .0;
                            self.setup_failure_report = Some(report);
                            self.setup_failure_report_open = true;
                            self.diagnostics_open = true;
                            outcome
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
                    self.status = Status::error(self.strings.setup_preflight_stopped());
                } else if task_kind.as_deref() == Some("upgrade-check") {
                    self.diagnostics_open = true;
                    self.status = Status::error(self.strings.upgrade_check_unreadable());
                } else if task_kind.as_deref() == Some("repair-check") {
                    self.diagnostics_open = true;
                    self.status = Status::warning(self.strings.repair_check_unreadable());
                } else if task_kind.as_deref() == Some("repair-apply") {
                    self.installation_repair_preview = None;
                    self.installation_repair_dialog_open = false;
                    self.diagnostics_open = true;
                    self.status = Status::warning(self.strings.repair_result_unreadable());
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
                self.status = Status::info(self.strings.status_restarting_unit(&unit));
            }
            Err(error) => {
                self.status = Status::error(self.strings.status_restart_failed(&error.to_string()))
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
            self.start_guided_setup();
        }
        if running && ui.small_button(self.strings.cancel()).clicked() {
            self.cancel_setup();
        }
    }

    pub(crate) fn render_setup_consent(&mut self, ctx: &egui::Context) {
        if !self.setup_consent_open {
            return;
        }
        let compositor = wayexpand_backend_selection::Compositor::detect();
        let (input_title, input_detail, tracking_title, tracking_detail) = match compositor {
            wayexpand_backend_selection::Compositor::Gnome => (
                self.strings.setup_gnome_input(),
                self.strings.setup_gnome_input_detail(),
                self.strings.setup_gnome_tracking(),
                self.strings.setup_gnome_tracking_detail(),
            ),
            wayexpand_backend_selection::Compositor::KdePlasma => (
                self.strings.setup_kde_input(),
                self.strings.setup_kde_input_detail(),
                self.strings.setup_kde_tracking(),
                self.strings.setup_tracking_detail(),
            ),
            compositor => (
                self.strings.setup_other_input(),
                self.strings.setup_other_input_detail(),
                self.strings.setup_other_tracking(),
                if compositor == wayexpand_backend_selection::Compositor::Unknown {
                    self.strings.setup_unknown_tracking_detail()
                } else {
                    self.strings.setup_other_tracking_detail()
                },
            ),
        };
        let show_tracking_step = compositor == wayexpand_backend_selection::Compositor::Gnome
            || self
                .config
                .expansion
                .iter()
                .any(|expansion| expansion.enabled && !expansion.app_filter.is_empty());
        let mut checks = vec![
            ("tools", self.strings.setup_check_tools()),
            ("component", self.strings.setup_check_component()),
            ("manifest", self.strings.setup_check_manifest()),
            ("manifest-scope", self.strings.setup_check_manifest_scope()),
            ("registered", self.strings.setup_check_registered()),
        ];
        if compositor == wayexpand_backend_selection::Compositor::Gnome {
            checks.extend([
                ("gnome-session", self.strings.setup_check_gnome_bridge()),
                (
                    "gnome-source-registered",
                    self.strings.setup_check_gnome_registered(),
                ),
                (
                    "gnome-source-active",
                    self.strings.setup_check_gnome_active(),
                ),
            ]);
        } else {
            checks.extend([
                ("desktop", self.strings.setup_check_desktop()),
                ("selected", self.strings.setup_check_selected()),
            ]);
        }
        checks.extend([
            ("engine", self.strings.setup_check_engine()),
            ("consent", self.strings.setup_check_consent()),
        ]);
        let mut attention = self
            .setup_ibus_preflight_report
            .as_ref()
            .and_then(|report| report.get("checks"))
            .and_then(serde_json::Value::as_array)
            .map(|checks| {
                checks
                    .iter()
                    .filter_map(|check| {
                        let state = check.get("state").and_then(serde_json::Value::as_str)?;
                        if !matches!(state, "fail" | "warn" | "unknown") {
                            return None;
                        }
                        Some((
                            match state {
                                "fail" => "✗",
                                "warn" => "!",
                                _ => "?",
                            },
                            check
                                .get("problem_id")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                            check
                                .get("title")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("IBus check")
                                .to_owned(),
                            check
                                .get("detail")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("The check did not provide details.")
                                .to_owned(),
                            check
                                .get("fix")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        attention.sort_by_key(|(symbol, ..)| match *symbol {
            "✗" => 0,
            "!" => 1,
            _ => 2,
        });
        // Put one prioritized action in the default view. The complete list
        // remains available in the expandable checks below for support work.
        attention.truncate(1);
        egui::Modal::new(egui::Id::new("ibus_replacement_consent")).show(ctx, |ui| {
            ui.set_width(430.0);
            ui.heading(self.strings.setup_title());
            if !attention.is_empty() {
                ui.add_space(6.0);
                ui.group(|ui| {
                    ui.label(RichText::new(self.strings.setup_review_stages()).strong());
                    for (symbol, problem_id, title, detail, fix) in &attention {
                        let id = if problem_id.is_empty() {
                            String::new()
                        } else {
                            format!(" [{problem_id}]")
                        };
                        ui.label(format!("{symbol} {title}{id}"));
                        if problem_id == "WX-IBUS-MULTIUSER-001" {
                            ui.label(
                                RichText::new(self.strings.setup_system_registration_reason())
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                            ui.label(
                                RichText::new(self.strings.setup_system_registration_action())
                                    .small(),
                            );
                        } else {
                            ui.label(
                                RichText::new(detail)
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                            if let Some(fix) = fix {
                                ui.label(
                                    RichText::new(format!(
                                        "{} {fix}",
                                        self.strings.setup_check_action()
                                    ))
                                    .small(),
                                );
                            }
                        }
                    }
                });
            }
            ui.add_space(8.0);
            egui::CollapsingHeader::new(self.strings.setup_checks())
                .default_open(false)
                .show(ui, |ui| {
                    for (id, label) in checks {
                        let preflight_check = self
                            .setup_ibus_preflight_report
                            .as_ref()
                            .and_then(|report| report.get("checks"))
                            .and_then(serde_json::Value::as_array)
                            .and_then(|checks| {
                                checks.iter().find(|check| {
                                    check.get("id").and_then(serde_json::Value::as_str) == Some(id)
                                })
                            });
                        let (state, check_detail, check_fix) = if self
                            .setup_ibus_preflight_report
                            .is_some()
                        {
                            match preflight_check {
                                Some(check) => {
                                    let state = match check
                                        .get("state")
                                        .and_then(serde_json::Value::as_str)
                                    {
                                        Some("pass") => Some(
                                            wayexpand_backend_ibus::chain::IbusCheckState::Pass,
                                        ),
                                        Some("warn") => Some(
                                            wayexpand_backend_ibus::chain::IbusCheckState::Warn,
                                        ),
                                        Some("fail") => Some(
                                            wayexpand_backend_ibus::chain::IbusCheckState::Fail,
                                        ),
                                        Some("skip") => Some(
                                            wayexpand_backend_ibus::chain::IbusCheckState::Skip,
                                        ),
                                        Some("unknown") => Some(
                                            wayexpand_backend_ibus::chain::IbusCheckState::Unknown,
                                        ),
                                        _ => None,
                                    };
                                    (
                                        state,
                                        check
                                            .get("detail")
                                            .and_then(serde_json::Value::as_str)
                                            .unwrap_or(
                                            "The preflight did not include details for this check.",
                                        ),
                                        check.get("fix").and_then(serde_json::Value::as_str),
                                    )
                                }
                                None => (
                                    Some(wayexpand_backend_ibus::chain::IbusCheckState::Unknown),
                                    "The preflight did not report this check.",
                                    None,
                                ),
                            }
                        } else if let Some(check) =
                            self.ibus_route.as_ref().and_then(|report| report.check(id))
                        {
                            (
                                Some(check.state),
                                check.detail.as_str(),
                                check.fix.as_deref(),
                            )
                        } else {
                            (
                                Some(wayexpand_backend_ibus::chain::IbusCheckState::Unknown),
                                "No current IBus diagnostic is available for this check.",
                                None,
                            )
                        };
                        let (symbol, detail) = match state {
                            Some(wayexpand_backend_ibus::chain::IbusCheckState::Pass) => {
                                ("✓", self.strings.setup_state_ready())
                            }
                            Some(wayexpand_backend_ibus::chain::IbusCheckState::Warn) => {
                                ("!", self.strings.setup_state_warn())
                            }
                            Some(wayexpand_backend_ibus::chain::IbusCheckState::Fail) => {
                                ("✗", self.strings.setup_state_fail())
                            }
                            Some(wayexpand_backend_ibus::chain::IbusCheckState::Skip) => {
                                ("·", self.strings.setup_state_skip())
                            }
                            _ => ("·", self.strings.setup_state_unknown()),
                        };
                        ui.label(format!("{symbol} {label}: {detail}"));
                        if matches!(
                            state,
                            Some(
                                wayexpand_backend_ibus::chain::IbusCheckState::Warn
                                    | wayexpand_backend_ibus::chain::IbusCheckState::Fail
                                    | wayexpand_backend_ibus::chain::IbusCheckState::Skip
                                    | wayexpand_backend_ibus::chain::IbusCheckState::Unknown
                            )
                        ) {
                            ui.indent(("ibus-check-detail", id), |ui| {
                                ui.label(
                                    RichText::new(check_detail)
                                        .small()
                                        .color(ui.visuals().weak_text_color()),
                                );
                                if let Some(fix) = check_fix {
                                    ui.label(
                                        RichText::new(format!(
                                            "{} {fix}",
                                            self.strings.setup_check_action()
                                        ))
                                        .small(),
                                    );
                                }
                            });
                        }
                    }
                });
            ui.add_space(8.0);
            let mut steps = vec![
                (
                    "1",
                    self.strings.setup_step_prepare(),
                    self.strings.setup_step_prepare_detail(),
                ),
                ("2", input_title, input_detail),
                (
                    "3",
                    self.strings.setup_step_activate(),
                    self.strings.setup_step_activate_detail(),
                ),
            ];
            if show_tracking_step {
                steps.push(("4", tracking_title, tracking_detail));
            }
            for (number, title, detail) in steps {
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
                ui.label(self.strings.setup_admin_needed());
                ui.checkbox(
                    &mut self.setup_system_integration_acknowledged,
                    self.strings.setup_admin_checkbox(),
                );
            } else {
                ui.add_space(8.0);
                ui.label(self.strings.setup_admin_not_needed());
            }
            ui.add_space(8.0);
            ui.label(self.strings.non_atomic_setup_warning());
            if self.config.settings.compatibility_mode == wayexpand_core::CompatibilityMode::Off {
                ui.checkbox(
                    &mut self.setup_consent_acknowledged,
                    self.strings.setup_consent_checkbox(),
                );
            } else {
                ui.add_space(8.0);
                ui.label(
                    self.strings
                        .setup_consent_already(self.config.settings.compatibility_mode),
                );
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        (!self.setup_system_integration_required
                            || self.setup_system_integration_acknowledged)
                            && self.setup_consent_acknowledged
                            && self.setup_task.is_none(),
                        egui::Button::new(self.strings.setup_continue()),
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
                        Some("ibus"),
                    );
                }
                if ui.button(self.strings.cancel()).clicked() {
                    self.setup_consent_open = false;
                    self.setup_consent_acknowledged = false;
                    self.setup_system_integration_required = false;
                    self.setup_system_integration_acknowledged = false;
                }
            });
        });
    }

    pub(crate) fn render_installation_repair_dialog(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
    ) {
        if !self.installation_repair_dialog_open {
            return;
        }
        let Some(report) = self.installation_repair_preview.as_ref() else {
            self.installation_repair_dialog_open = false;
            return;
        };
        let repairs = report
            .get("repairs")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut open = self.installation_repair_dialog_open;
        let mut close_requested = false;
        egui::Window::new(self.strings.repair_title())
            .id(egui::Id::new("installation_safe_repair"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(430.0);
                ui.label(self.strings.repair_explainer());
                ui.add_space(8.0);
                for repair in &repairs {
                    let component = repair
                        .get("component")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("user service");
                    let path = repair
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("installation path unavailable");
                    ui.label(RichText::new(component).strong());
                    ui.label(RichText::new(path).small().color(palette.muted));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.setup_task.is_none(),
                            egui::Button::new(self.strings.repair_apply_button()),
                        )
                        .clicked()
                    {
                        self.apply_installation_repairs();
                    }
                    if ui.button(self.strings.repair_not_now()).clicked() {
                        close_requested = true;
                    }
                });
            });
        self.installation_repair_dialog_open = open && !close_requested;
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
    fn first_run_consent_explains_interleaving_partial_writes_and_no_retry() {
        for required_warning in [
            "If you keep typing",
            "interleave with the snippet",
            "partial or corrupted text",
            "cannot be rolled back",
            "will not retry an uncertain result",
            "Headless tests do not certify physical-session target integrity",
            "no accepted physical-session certification for this route",
        ] {
            assert!(
                crate::lang::Strings::new(crate::lang::Language::English)
                    .non_atomic_setup_warning()
                    .contains(required_warning),
                "missing consent warning: {required_warning}"
            );
        }
    }

    #[test]
    fn gui_cli_tasks_use_the_active_config_for_restart_resume() {
        let config_path = std::path::Path::new("/tmp/wayexpand-custom/expansions.toml");
        let mut command = std::process::Command::new("wayexpand");
        configure_cli_task_command(&mut command, config_path);
        assert!(command.get_envs().any(|(name, value)| {
            name == "WAYEXPAND_CONFIG" && value == Some(config_path.as_os_str())
        }));
    }

    #[test]
    fn system_registration_consent_requires_a_multiuser_safe_component() {
        let registered = serde_json::json!({
            "checks": [
                {"id": "component", "state": "pass"},
                {"id": "manifest", "state": "pass"},
                {"id": "manifest-scope", "state": "pass"}
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

        let legacy_shared_manifest = serde_json::json!({
            "checks": [
                {"id": "component", "state": "pass"},
                {"id": "manifest", "state": "pass"},
                {"id": "manifest-scope", "state": "warn"}
            ]
        });
        assert_eq!(
            ibus_registration_needed(&legacy_shared_manifest),
            Some(true)
        );

        let older_report_without_scope_evidence = serde_json::json!({
            "checks": [
                {"id": "component", "state": "pass"},
                {"id": "manifest", "state": "pass"}
            ]
        });
        assert_eq!(
            ibus_registration_needed(&older_report_without_scope_evidence),
            Some(true)
        );

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
        let output = b"WayExpand setup\nSETUP RESULT: CONFIGURED, AWAITING TEST\nNext: run wayexpand debug --test ';custom'.\n";
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
    fn setup_output_retains_real_test_result_even_if_followup_steps_follow() {
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
            &crate::lang::Strings::new(crate::lang::Language::English),
            "SETUP RESULT: CONFIGURED, AWAITING TEST",
            "generic success".into(),
        );
        assert_eq!(
            status.text(),
            "Configured, untested: complete Desktop Integration Test to verify a real insertion in a target application."
        );
        assert!(!status.is_error());
        assert!(open_diagnostics);
    }

    #[test]
    fn setup_status_reports_working_only_after_a_confirmed_test() {
        let (status, open_diagnostics) = GuiApp::setup_completion_status(
            &crate::lang::Strings::new(crate::lang::Language::English),
            "SETUP RESULT: WORKING",
            "generic success".into(),
        );
        assert_eq!(
            status.text(),
            "Working: a real insertion was confirmed in the tested application."
        );
        assert!(!open_diagnostics);
    }

    #[test]
    fn setup_status_explains_restart_and_replacement_authorization_states() {
        let (restart, open_diagnostics) = GuiApp::setup_completion_status(
            &crate::lang::Strings::new(crate::lang::Language::English),
            "WX-GNOME-SESSION-RESTART-001: log out of GNOME and log back in",
            "setup failed".into(),
        );
        assert!(restart.text().contains("Needs desktop restart"));
        assert!(!open_diagnostics);

        let (permission, open_diagnostics) = GuiApp::setup_completion_status(
            &crate::lang::Strings::new(crate::lang::Language::English),
            "WX-POLICY-001: replacement is disabled",
            "setup incomplete".into(),
        );
        assert!(permission.text().contains("Replacement consent needed"));
        assert!(!open_diagnostics);
    }

    #[test]
    fn generic_setup_completion_is_never_presented_as_verified_success() {
        let (status, open_diagnostics) = GuiApp::setup_completion_status(
            &crate::lang::Strings::new(crate::lang::Language::English),
            "IBus component installed",
            "Setup finished.".into(),
        );
        assert!(!status.is_error());
        assert!(status
            .text()
            .contains("not enough to verify a real application insertion"));
        assert!(open_diagnostics);
    }

    #[test]
    fn desktop_test_without_activity_opens_diagnostics_not_success() {
        let (status, open_diagnostics) = GuiApp::setup_completion_status(
            &crate::lang::Strings::new(crate::lang::Language::English),
            "DESKTOP TEST RESULT: INPUT NOT OBSERVED",
            "generic success".into(),
        );
        assert!(status.text().contains("Desktop test was not verified"));
        assert!(open_diagnostics);
    }
}
