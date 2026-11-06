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
        self.cli_task_kind = None;
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
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
                            return Err("Setup cancelled".to_owned());
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
                            return Err(format!(
                                "Setup timed out after {} seconds",
                                SETUP_TIMEOUT.as_secs()
                            ));
                        }
                        thread::sleep(Duration::from_millis(50));
                    };
                    if status.success() {
                        Ok(Self::setup_output_text(&stdout_bytes))
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
                });
                self.status = Status::info(running_message);
            }
            Err(error) => {
                self.status = Status::error(self.strings.status_setup_failed(&error.to_string()))
            }
        }
    }

    pub(crate) fn setup_output_text(bytes: &[u8]) -> String {
        let output = String::from_utf8_lossy(bytes);
        let lines = output.lines().collect::<Vec<_>>();
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
            (Status::success(fallback), false)
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
                self.setup_task = None;
                self.status = match result {
                    Ok(output) => {
                        self.last_cli_task_output = Some(output.clone());
                        match self.cli_task_kind.take().as_deref() {
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
                        let (status, open_diagnostics) =
                            Self::setup_completion_status(&output, success_message);
                        self.diagnostics_open |= open_diagnostics;
                        status
                    }
                    Err(detail) => {
                        self.cli_task_kind = None;
                        // Setup's own report names the broken link; show the
                        // same checks here instead of a one-line error.
                        self.diagnostics_open = true;
                        Status::error(self.strings.status_setup_failed(&detail))
                    }
                };
                self.refresh_diagnostics(false);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.setup_task = None;
                self.status = Status::error(self.strings.status_setup_failed("worker stopped"));
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
                self.setup_consent_acknowledged = self.config.settings.compatibility_mode
                    != wayexpand_core::CompatibilityMode::Off;
                self.setup_system_integration_acknowledged = false;
                self.setup_consent_open = true;
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
        egui::Modal::new(egui::Id::new("ibus_replacement_consent")).show(ctx, |ui| {
                ui.set_width(430.0);
                ui.heading("Set up WayExpand with IBus?");
                ui.add_space(8.0);
                ui.label("The IBus engine manifest may need to be registered in the component directory used by your running IBus session. Application files remain in your user installation; administrator authorization is requested only if that directory is not writable by you.");
                ui.checkbox(
                    &mut self.setup_system_integration_acknowledged,
                    "I want to install the IBus integration if it is missing.",
                );
                ui.add_space(8.0);
                ui.label("Alternative: cancel and install WayExpand using your distribution package, which includes the IBus registration, or select another input route.");
                if self.config.settings.compatibility_mode
                    == wayexpand_core::CompatibilityMode::Off
                {
                    ui.add_space(8.0);
                ui.label("WayExpand checks the text before the cursor against your trigger, then deletes the trigger and inserts the snippet in two separate steps. This replacement is not atomic.");
                ui.add_space(8.0);
                ui.label("If focus or input changes during insertion, WayExpand cannot guarantee rollback of text already sent.");
                ui.checkbox(
                    &mut self.setup_consent_acknowledged,
                    "I understand and want to enable verified two-step replacement.",
                );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.setup_system_integration_acknowledged
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
                            true,
                        );
                    }
                    if ui.button("Cancel").clicked() {
                        self.setup_consent_open = false;
                        self.setup_consent_acknowledged = false;
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
    fn setup_output_retains_real_test_result_even_if_optional_steps_follow() {
        let output = b"SETUP RESULT: WORKING\nA real expansion was user-confirmed in the tested application.\nOptional terminal coverage is not configured.\n";
        let visible = GuiApp::setup_output_text(output);
        assert!(visible.contains("SETUP RESULT: WORKING"));
        assert!(visible.contains("A real expansion was user-confirmed"));
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
}
