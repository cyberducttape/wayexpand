//! Library → Sync: run `wayexpand sync` for a Git-tracked library.

use std::{
    io::Read,
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
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "sync stdout was not captured".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "sync stderr was not captured".to_owned())?;
    let stdout_reader = thread::spawn(|| read_sync_output(stdout));
    let stderr_reader = thread::spawn(|| read_sync_output(stderr));
    let mut supervisor = ChildSupervisor::new(child);
    let deadline = Instant::now() + SYNC_TIMEOUT;
    let status = loop {
        match supervisor.has_exited() {
            Ok(true) => {
                supervisor.kill_group();
                break supervisor.reap().map_err(|error| error.to_string())?;
            }
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(false) | Err(_) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("sync timed out or could not be monitored".to_owned());
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "sync stdout reader failed".to_owned())?
        .map_err(|error| error.to_string())?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "sync stderr reader failed".to_owned())?
        .map_err(|error| error.to_string())?;
    let text = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .lines()
            .last()
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    if status.success() {
        Ok(text(&stdout))
    } else {
        Err(text(&stderr))
    }
}

fn read_sync_output<R: Read>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .by_ref()
        .take((MAX_SYNC_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut output)?;
    if output.len() > MAX_SYNC_OUTPUT_BYTES {
        return Err(std::io::Error::other(
            "sync output exceeded the safety limit",
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{read_sync_output, MAX_SYNC_OUTPUT_BYTES};

    #[test]
    fn sync_output_is_bounded() {
        let error = read_sync_output(std::io::Cursor::new(vec![b'x'; MAX_SYNC_OUTPUT_BYTES + 1]))
            .unwrap_err();
        assert!(error.to_string().contains("safety limit"));
    }
}
