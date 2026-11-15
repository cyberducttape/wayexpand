//! Recovery screen shown when the snippet library cannot be loaded.
//!
//! A configuration that fails validation used to stop the GUI before its
//! window opened, turning a graphical repair into a terminal exercise. This
//! screen shows the (content-free) error and offers the repairs that never
//! lose the original file: edit it, restore a validated backup, or start a
//! fresh library. Whatever replaces the file, the broken one is kept beside
//! it as `<name>.broken-<seconds>-<nanoseconds>`.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, RichText};

use crate::lang::Strings;

const MAX_BACKUPS_SHOWN: usize = 5;
/// Validating a backup loads it, so a folder with thousands of them must
/// not stall the recovery screen.
const MAX_BACKUPS_EXAMINED: usize = 200;

struct Backup {
    path: PathBuf,
    valid: bool,
}

pub(crate) struct RecoveryApp {
    path: PathBuf,
    error: String,
    backups: Vec<Backup>,
    strings: Strings,
    status: Option<String>,
}

impl RecoveryApp {
    pub(crate) fn new(path: PathBuf, error: String, strings: Strings) -> Self {
        let backups = find_backups(&path);
        Self {
            path,
            error,
            backups,
            strings,
            status: None,
        }
    }

    fn relaunch(&mut self, ctx: &egui::Context) {
        let started = std::env::current_exe()
            .and_then(|exe| std::process::Command::new(exe).arg(&self.path).spawn());
        match started {
            Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(error) => {
                self.status = Some(self.strings.recovery_relaunch_failed(&error.to_string()))
            }
        }
    }

    fn open(&mut self, target: &Path) {
        if let Err(error) = std::process::Command::new("xdg-open").arg(target).spawn() {
            self.status = Some(self.strings.recovery_open_failed(&error.to_string()));
        }
    }
}

impl eframe::App for RecoveryApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        egui::CentralPanel::default().show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                ui.heading(self.strings.recovery_title());
                ui.add_space(6.0);
                ui.label(self.strings.recovery_intro());
                ui.add_space(6.0);
                ui.label(
                    RichText::new(self.path.display().to_string())
                        .monospace()
                        .small(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&self.error)
                        .monospace()
                        .color(ui.visuals().warn_fg_color),
                );
                ui.add_space(12.0);

                ui.label(RichText::new(self.strings.recovery_fix_heading()).strong());
                ui.horizontal_wrapped(|ui| {
                    if ui.button(self.strings.recovery_open_file()).clicked() {
                        let path = self.path.clone();
                        self.open(&path);
                    }
                    if let Some(folder) = self.path.parent().map(Path::to_path_buf) {
                        if ui.button(self.strings.recovery_open_folder()).clicked() {
                            self.open(&folder);
                        }
                    }
                    if ui.button(self.strings.recovery_try_again()).clicked() {
                        self.relaunch(&ctx);
                    }
                });
                ui.add_space(12.0);

                ui.label(RichText::new(self.strings.recovery_backups_heading()).strong());
                if self.backups.is_empty() {
                    ui.label(self.strings.recovery_no_backups());
                }
                let mut restore = None;
                for (index, backup) in self.backups.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        let name = backup
                            .path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.label(RichText::new(name).monospace());
                        if backup.valid {
                            if ui.button(self.strings.recovery_restore()).clicked() {
                                restore = Some(index);
                            }
                        } else {
                            ui.label(RichText::new(self.strings.recovery_backup_invalid()).weak());
                        }
                    });
                }
                if let Some(index) = restore {
                    let backup = self.backups[index].path.clone();
                    match restore_backup(&self.path, &backup) {
                        Ok(()) => self.relaunch(&ctx),
                        Err(error) => {
                            self.status =
                                Some(self.strings.recovery_action_failed(&error.to_string()))
                        }
                    }
                }
                ui.add_space(12.0);

                ui.label(RichText::new(self.strings.recovery_fresh_heading()).strong());
                ui.label(self.strings.recovery_fresh_detail());
                if ui.button(self.strings.recovery_fresh_button()).clicked() {
                    match keep_broken(&self.path) {
                        Ok(_) => self.relaunch(&ctx),
                        Err(error) => {
                            self.status =
                                Some(self.strings.recovery_action_failed(&error.to_string()))
                        }
                    }
                }
                if let Some(status) = &self.status {
                    ui.add_space(10.0);
                    ui.colored_label(ui.visuals().error_fg_color, status);
                }
            });
        });
    }
}

/// Backups beside the configuration (`<name>.bak`, `<name>.bak.N`), newest
/// first, each checked by loading it the way the GUI would. Candidates are
/// validated until the five newest valid ones are found, so a run of damaged
/// recent backups cannot hide an older good one. Damaged backups newer than
/// those stay listed (at most five) so the user sees why they are skipped.
fn find_backups(path: &Path) -> Vec<Backup> {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.bak", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| (metadata.modified().unwrap_or(UNIX_EPOCH), entry.path()))
        })
        .collect();
    found.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    let mut shown = Vec::new();
    let (mut valid, mut invalid) = (0, 0);
    for (_, path) in found.into_iter().take(MAX_BACKUPS_EXAMINED) {
        if valid == MAX_BACKUPS_SHOWN {
            break;
        }
        if wayexpand_core::Config::load(&path).is_ok() {
            valid += 1;
            shown.push(Backup { path, valid: true });
        } else if invalid < MAX_BACKUPS_SHOWN {
            invalid += 1;
            shown.push(Backup { path, valid: false });
        }
    }
    shown
}

/// Replace the broken configuration with a backup. The backup is staged in
/// a temporary file first, so a failure never leaves the library missing:
/// if the final rename fails, the broken file is put back.
fn restore_backup(path: &Path, backup: &Path) -> std::io::Result<()> {
    let mut staged = path.as_os_str().to_owned();
    staged.push(".restoring");
    let staged = PathBuf::from(staged);
    let _ = fs::remove_file(&staged);
    replace_with(&staged, backup)?;
    let kept = match keep_broken(path) {
        Ok(kept) => kept,
        Err(error) => {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
    };
    if let Err(error) = fs::rename(&staged, path) {
        if let Some(kept) = kept {
            let _ = fs::rename(kept, path);
        }
        let _ = fs::remove_file(&staged);
        return Err(error);
    }
    Ok(())
}

/// Move the broken configuration to `<name>.broken-<seconds>-<nanoseconds>`
/// beside it, never replacing an existing file. Returns where it was kept.
///
/// The name is claimed with a hard link, which fails instead of overwriting
/// when the name exists; a taken name (two recoveries within the same
/// instant, or a leftover file) moves on to a numbered variant. The original
/// is removed only after the link succeeds, so it is never lost.
fn keep_broken(path: &Path) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let stamp = format!("{}-{:09}", now.as_secs(), now.subsec_nanos());
    for attempt in 0..100 {
        let mut kept = path.as_os_str().to_owned();
        if attempt == 0 {
            kept.push(format!(".broken-{stamp}"));
        } else {
            kept.push(format!(".broken-{stamp}-{attempt}"));
        }
        let kept = PathBuf::from(kept);
        match fs::hard_link(path, &kept) {
            Ok(()) => {
                if let Err(error) = fs::remove_file(path) {
                    let _ = fs::remove_file(&kept);
                    return Err(error);
                }
                return Ok(Some(kept));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free name to keep the broken configuration under",
    ))
}

/// Write `source`'s contents to `path` as a new owner-only file.
fn replace_with(path: &Path, source: &Path) -> std::io::Result<()> {
    let contents = fs::read(source)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&contents)?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wayexpand-recovery-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn restoring_a_backup_keeps_the_broken_file() {
        let dir = scratch("restore");
        let config = dir.join("expansions.toml");
        fs::write(&config, "this is = = not toml").unwrap();
        let backup = dir.join("expansions.toml.bak");
        fs::write(
            &backup,
            "[[expansion]]\ntrigger = \";ok\"\nreplacement = \"fine\"\n",
        )
        .unwrap();
        fs::set_permissions(&backup, fs::Permissions::from_mode(0o600)).unwrap();

        let backups = find_backups(&config);
        assert_eq!(backups.len(), 1);
        assert!(backups[0].valid, "a valid backup is offered");

        restore_backup(&config, &backup).unwrap();
        assert!(wayexpand_core::Config::load(&config).is_ok());
        let kept: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".broken-"))
            .collect();
        assert_eq!(kept.len(), 1, "the broken file is kept");
        assert_eq!(
            fs::read_to_string(kept[0].path()).unwrap(),
            "this is = = not toml"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_backups_are_listed_but_not_offered() {
        let dir = scratch("invalid");
        let config = dir.join("expansions.toml");
        fs::write(&config, "broken").unwrap();
        fs::write(dir.join("expansions.toml.bak"), "also broken").unwrap();
        let backups = find_backups(&config);
        assert_eq!(backups.len(), 1);
        assert!(!backups[0].valid);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_valid_backup_behind_five_damaged_ones_is_still_offered() {
        let dir = scratch("sixth");
        let config = dir.join("expansions.toml");
        fs::write(&config, "broken").unwrap();
        let good = dir.join("expansions.toml.bak.6");
        fs::write(
            &good,
            "[[expansion]]\ntrigger = \";ok\"\nreplacement = \"fine\"\n",
        )
        .unwrap();
        fs::set_permissions(&good, fs::Permissions::from_mode(0o600)).unwrap();
        let old = SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&good)
            .unwrap()
            .set_modified(old)
            .unwrap();
        for number in 1..=5 {
            fs::write(dir.join(format!("expansions.toml.bak.{number}")), "damaged").unwrap();
        }
        let backups = find_backups(&config);
        assert!(
            backups
                .iter()
                .any(|backup| backup.valid && backup.path == good),
            "the older valid backup is offered"
        );
        assert_eq!(backups.iter().filter(|backup| !backup.valid).count(), 5);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn keeping_a_broken_file_never_collides_or_overwrites() {
        let dir = scratch("collide");
        let config = dir.join("expansions.toml");
        let mut kept = Vec::new();
        for round in 0..3 {
            fs::write(&config, format!("broken {round}")).unwrap();
            kept.push(keep_broken(&config).unwrap().expect("a file was kept"));
            assert!(!config.exists(), "the original is moved away");
        }
        kept.sort();
        kept.dedup();
        assert_eq!(kept.len(), 3, "three recoveries keep three files");
        let contents: std::collections::BTreeSet<String> = kept
            .iter()
            .map(|path| fs::read_to_string(path).unwrap())
            .collect();
        assert_eq!(contents.len(), 3, "no kept file was overwritten");
        fs::remove_dir_all(dir).unwrap();
    }
}
