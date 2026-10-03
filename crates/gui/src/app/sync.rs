//! Library → Sync: run `wayexpand sync` for a Git-tracked library.

use std::sync::mpsc;

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
                let result = std::process::Command::new(cli)
                    .arg("sync")
                    .arg(config)
                    .stdin(std::process::Stdio::null())
                    .output()
                    .map_err(|error| error.to_string())
                    .and_then(|output| {
                        let text = |bytes: &[u8]| {
                            String::from_utf8_lossy(bytes)
                                .lines()
                                .last()
                                .unwrap_or_default()
                                .trim()
                                .to_owned()
                        };
                        if output.status.success() {
                            Ok(text(&output.stdout))
                        } else {
                            Err(text(&output.stderr))
                        }
                    });
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
