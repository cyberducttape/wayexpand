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

impl GuiApp {
    /// One-click setup: run `wayexpand setup --yes`, which configures only
    /// the safe Recommended mode (never raw keyboard access), off the UI
    /// thread, then refresh the daemon status.
    pub(crate) fn start_setup(&mut self) {
        if self.setup_task.is_some() {
            return;
        }
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
            .name("wayexpand-setup".into())
            .spawn(move || {
                let result = (|| {
                    let mut command = std::process::Command::new(&cli);
                    configure_process_group(&mut command);
                    let mut child = command
                        .args(["setup", "--yes"])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|error| error.to_string())?;
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
                            return Err("Setup timed out after 30 seconds".to_owned());
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
                self.setup_task = Some(SetupTask { receiver, cancel });
                self.status = Status::info(self.strings.status_setup_running());
            }
            Err(error) => {
                self.status = Status::error(self.strings.status_setup_failed(&error.to_string()))
            }
        }
    }

    pub(crate) fn setup_output_text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes)
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .trim_start_matches("Error: ")
            .to_owned()
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
                self.setup_task = None;
                self.status = match result {
                    Ok(_) => Status::success(self.strings.status_setup_done()),
                    Err(detail) => Status::error(self.strings.status_setup_failed(&detail)),
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
            self.start_setup();
        }
        if running && ui.small_button(self.strings.cancel()).clicked() {
            self.cancel_setup();
        }
    }

    pub(crate) fn render_evdev_setup(&mut self, ctx: &egui::Context, palette: &Palette) {
        if !self.evdev_setup_open {
            return;
        }
        egui::Window::new(self.strings.evdev_setup_title())
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
    use std::process::{Command, Stdio};
    use wayexpand_process_supervisor::configure_process_group;

    #[test]
    fn setup_output_drain_yields_for_continuous_output() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "yes setup"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        let mut child = command.spawn().expect("spawn continuous-output child");
        let mut stdout = child.stdout.take().expect("child stdout");
        set_nonblocking(&stdout).expect("make stdout nonblocking");
        let mut output = Vec::new();
        let started = Instant::now();
        let result = drain_setup_output(&mut stdout, &mut output);
        assert!(result.is_ok());
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = child.kill();
        let _ = child.wait();
    }
}
