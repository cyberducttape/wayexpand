//! Library → Sync: run `wayexpand sync` for a Git-tracked library.

use std::{
    io::Read,
    os::fd::AsRawFd,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SYNC_OUTPUT_BYTES: usize = 128 * 1024;

use crate::*;

impl GuiApp {
    /// Whether the library is tracked with `wayexpand sync init`.
    pub(crate) fn library_is_synchronized(&self) -> bool {
        self.path
            .parent()
            .is_some_and(|directory| directory.join(".git").is_dir())
    }

    pub(crate) fn start_sync(&mut self) {
        if self.sync_task.is_some() {
            return;
        }
        if self.draft_is_dirty() {
            self.status = Status::warning(self.strings.sync_save_first());
            return;
        }
        let cli = env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("wayexpand")))
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("wayexpand"));
        let config = self.path.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("wayexpand-sync".into())
            .spawn(move || {
                let result = run_sync_command(cli, config);
                let _ = sender.send(result);
            });
        match spawned {
            Ok(_) => {
                self.sync_task = Some(receiver);
                self.status = Status::info(self.strings.sync_running());
            }
            Err(error) => self.status = Status::error(error.to_string()),
        }
    }

    pub(crate) fn poll_sync(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.sync_task else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(summary)) => {
                self.sync_task = None;
                self.status = Status::success(summary);
                // Show what the merge brought in.
                self.perform_reload();
            }
            Ok(Err(error)) => {
                self.sync_task = None;
                self.status = Status::error(error);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            Err(mpsc::TryRecvError::Disconnected) => self.sync_task = None,
        }
    }
}

fn run_sync_command(cli: std::path::PathBuf, config: std::path::PathBuf) -> Result<String, String> {
    let mut command = Command::new(cli);
    configure_process_group(&mut command);
    let mut child = command
        .args(["sync"])
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "sync stdout was not captured".to_owned())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "sync stderr was not captured".to_owned())?;
    let mut supervisor = ChildSupervisor::new(child);
    if let Err(error) = set_nonblocking(&stdout).and_then(|_| set_nonblocking(&stderr)) {
        supervisor.kill_group();
        let _ = supervisor.reap();
        return Err(error.to_string());
    }
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let deadline = Instant::now() + SYNC_TIMEOUT;
    let status = loop {
        let stdout_eof =
            drain_sync_output(&mut stdout, &mut stdout_bytes).map_err(|error| error.to_string())?;
        let stderr_eof =
            drain_sync_output(&mut stderr, &mut stderr_bytes).map_err(|error| error.to_string())?;
        if stdout_bytes.len() > MAX_SYNC_OUTPUT_BYTES || stderr_bytes.len() > MAX_SYNC_OUTPUT_BYTES
        {
            supervisor.kill_group();
            let _ = supervisor.reap();
            return Err("sync output exceeded the safety limit".to_owned());
        }
        match supervisor.has_exited() {
            Ok(true) => {
                supervisor.kill_group();
                let status = supervisor.reap().map_err(|error| error.to_string())?;
                let drain_deadline = Instant::now() + Duration::from_millis(100);
                let mut stdout_eof = stdout_eof;
                let mut stderr_eof = stderr_eof;
                while !(stdout_eof && stderr_eof) && Instant::now() < drain_deadline {
                    stdout_eof = drain_sync_output(&mut stdout, &mut stdout_bytes)
                        .map_err(|error| error.to_string())?;
                    stderr_eof = drain_sync_output(&mut stderr, &mut stderr_bytes)
                        .map_err(|error| error.to_string())?;
                    if stdout_bytes.len() > MAX_SYNC_OUTPUT_BYTES
                        || stderr_bytes.len() > MAX_SYNC_OUTPUT_BYTES
                    {
                        return Err("sync output exceeded the safety limit".to_owned());
                    }
                    if !(stdout_eof && stderr_eof) {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                if !stdout_eof {
                    return Err("sync stdout did not complete".to_owned());
                }
                break status;
            }
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(false) | Err(_) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return Err("sync timed out or could not be monitored".to_owned());
            }
        }
    };
    let text = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .lines()
            .last()
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    if status.success() {
        Ok(text(&stdout_bytes))
    } else {
        Err(text(&stderr_bytes))
    }
}

fn set_nonblocking<R: AsRawFd>(stream: &R) -> Result<(), String> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(format!(
            "could not make sync output nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn drain_sync_output<R: Read>(reader: &mut R, output: &mut Vec<u8>) -> std::io::Result<bool> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..64 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => output.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::drain_sync_output;

    #[test]
    fn sync_output_drain_reaches_eof_without_a_blocking_join() {
        let mut output = Vec::new();
        let mut input = std::io::Cursor::new(b"sync complete\n".to_vec());
        assert!(drain_sync_output(&mut input, &mut output).unwrap());
        assert_eq!(output, b"sync complete\n");
    }
}
