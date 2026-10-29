use crate::*;

impl GuiApp {
    pub(crate) fn start_profile_operation(&mut self, operation: ProfileOperation) {
        if self.profile_receiver.is_some() {
            return;
        }
        let path = self.path.clone();
        let worker_operation = operation.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("wayexpand-profile-operation".into())
            .spawn(move || {
                let mut command = std::process::Command::new(wayexpand_cli());
                command.arg("profile");
                match &worker_operation {
                    ProfileOperation::List => {
                        command.arg("list");
                    }
                    ProfileOperation::Switch(name) => {
                        command.arg("switch").arg(name);
                    }
                    ProfileOperation::Create(name) => {
                        command.arg("create").arg(name);
                    }
                }
                let result = command
                    .arg(path)
                    .output()
                    .map_err(|error| format!("Could not start `wayexpand profile`: {error}"))
                    .and_then(|output| {
                        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                        if output.status.success() {
                            Ok(stdout)
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                            Err(if stderr.is_empty() {
                                format!("Profile command exited with {}", output.status)
                            } else {
                                stderr
                            })
                        }
                    });
                let _ = sender.send((worker_operation, result));
            });
        match spawned {
            Ok(_) => {
                self.profile_error = None;
                self.profile_receiver = Some(receiver);
            }
            Err(error) => {
                self.profile_error = Some(format!("Could not start profile worker: {error}"));
            }
        }
    }

    pub(crate) fn poll_profiles(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.profile_receiver.as_ref() else {
            return;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => Some(completion),
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                None
            }
            Err(mpsc::TryRecvError::Disconnected) => Some((
                ProfileOperation::List,
                Err("Profile worker stopped unexpectedly".into()),
            )),
        };
        let Some((operation, result)) = completion else {
            return;
        };
        self.profile_receiver = None;
        match (operation, result) {
            (ProfileOperation::List, Ok(output)) => {
                (self.profiles, self.active_profile) = parse_profile_list(&output);
                self.profile_loaded = true;
                self.profile_error = None;
            }
            (ProfileOperation::Switch(name), Ok(_)) | (ProfileOperation::Create(name), Ok(_)) => {
                self.active_profile = Some(name.clone());
                self.profile_error = None;
                self.status = Status::success(format!("Active profile: {name}"));
                self.perform_reload();
                self.profile_loaded = false;
                self.start_profile_operation(ProfileOperation::List);
            }
            (operation, Err(error)) => {
                if matches!(operation, ProfileOperation::List) {
                    self.profile_loaded = true;
                }
                self.profile_error = Some(error.clone());
                self.status = Status::error(error);
            }
        }
    }
}

pub(crate) fn wayexpand_cli() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("WAYEXPAND_CLI") {
        return path.into();
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(sibling) = executable
            .parent()
            .map(|parent| parent.join("wayexpand"))
            .filter(|candidate| candidate.is_file())
        {
            return sibling;
        }
    }
    "wayexpand".into()
}

fn parse_profile_list(output: &str) -> (Vec<ProfileEntry>, Option<String>) {
    let mut profiles = Vec::new();
    let mut active_profile = None;
    for line in output.lines() {
        let (active, entry) = if let Some(entry) = line.strip_prefix("* ") {
            (true, entry)
        } else if let Some(entry) = line.strip_prefix("  ") {
            (false, entry)
        } else {
            continue;
        };
        let Some((name, state)) = entry.rsplit_once(" [") else {
            continue;
        };
        let Some(state) = state.strip_suffix(']') else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if active {
            active_profile = Some(name.to_owned());
        }
        profiles.push(ProfileEntry {
            name: name.to_owned(),
            valid: state == "valid",
        });
    }
    (profiles, active_profile)
}

#[cfg(test)]
mod tests {
    use super::parse_profile_list;

    #[test]
    fn parses_cli_profile_rows_and_keeps_invalid_entries_disabled() {
        let (profiles, active) = parse_profile_list(
            "* personal [valid]\n  work [valid]\n  broken [invalid]\nNo profiles yet",
        );
        assert_eq!(active.as_deref(), Some("personal"));
        assert_eq!(profiles.len(), 3);
        assert!(profiles[0].valid && profiles[1].valid);
        assert!(!profiles[2].valid);
        assert_eq!(profiles[2].name, "broken");
    }
}
