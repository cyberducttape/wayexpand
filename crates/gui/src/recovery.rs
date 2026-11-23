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
                self.status = Some(self.strings.recovery_relaunch_failed(error.to_string()))
            }
        }
    }

    fn open(&mut self, target: &Path) {
        if let Err(error) = std::process::Command::new("xdg-open").arg(target).spawn() {
            self.status = Some(self.strings.recovery_open_failed(error.to_string()));
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
                                Some(self.strings.recovery_action_failed(error.to_string()))
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
                                Some(self.strings.recovery_action_failed(error.to_string()))
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
        if read_valid_backup(&path).is_ok() {
            valid += 1;
            shown.push(Backup { path, valid: true });
        } else if invalid < MAX_BACKUPS_SHOWN {
            invalid += 1;
            shown.push(Backup { path, valid: false });
        }
    }
    shown
}

/// Read a backup once and validate exactly those bytes with the
/// configuration rules, returning them for installation. Unlike
/// `Config::load`, this never rewrites the backup (legacy ID migration) or
/// leaves a write lock beside it.
fn read_valid_backup(backup: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let limit = wayexpand_core::limits::MAX_CONFIG_BYTES as u64;
    let mut contents = Vec::new();
    fs::File::open(backup)?
        .take(limit + 1)
        .read_to_end(&mut contents)?;
    let invalid = |detail: String| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("the backup is not a valid configuration: {detail}"),
        )
    };
    let text = std::str::from_utf8(&contents).map_err(|_| invalid("not UTF-8".into()))?;
    wayexpand_core::Config::parse(text).map_err(|error| invalid(error.safe_summary()))?;
    Ok(contents)
}

/// Replace the broken configuration with a backup.
///
/// The live path is never absent: the broken file is preserved under a new
/// hard link, and the validated replacement is then renamed over it in one
/// atomic step, so a crash at any point leaves either the old or the new
/// configuration in place. The bytes that are validated are the bytes that
/// get installed (the staged copy, not the backup path, which may have
/// changed since the screen listed it). The library lock, shared with
/// `wayexpand sync`, keeps two recovery windows or a sync from interleaving,
/// and each restore stages in a file only it created and only it removes.
fn restore_backup(path: &Path, backup: &Path) -> std::io::Result<()> {
    restore_backup_with(path, backup, || Ok(()))
}

/// [`restore_backup`] with a hook run after the broken file is preserved and
/// before the replacement is renamed into place, for fault-injection tests.
fn restore_backup_with(
    path: &Path,
    backup: &Path,
    before_rename: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let _lock = LibraryLock::acquire(path)?;
    let contents = read_valid_backup(backup)?;
    let staged = StagedFile::create(path)?;
    staged.write(&contents)?;
    let kept = preserve_broken(path)?;
    let installed = before_rename().and_then(|()| staged.install(path));
    if installed.is_err() {
        // The live file was never touched; drop the duplicate link to it.
        if let Some(kept) = kept {
            let _ = fs::remove_file(kept);
        }
    }
    installed
}

/// The library lock `wayexpand sync` also takes (`.wayexpand-sync.lock`
/// beside the configuration), held for one restore.
struct LibraryLock {
    // Held only for its flock, released when the descriptor closes.
    _file: fs::File,
}

impl LibraryLock {
    fn acquire(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::io::AsRawFd;
        let folder = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "configuration path has no folder",
            )
        })?;
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(folder.join(".wayexpand-sync.lock"))?;
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the library lock has an unsafe type, owner, or link count",
            ));
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            // SAFETY: the descriptor is open for the duration of the call.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self { _file: file });
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(error);
            }
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "another recovery or `wayexpand sync` is using the library; try again",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// A staging file beside the configuration that this restore created with
/// `create_new`, under a name no other restore can choose; it is removed on
/// drop unless it was installed.
struct StagedFile {
    path: PathBuf,
    file: Option<fs::File>,
}

impl StagedFile {
    fn create(path: &Path) -> std::io::Result<Self> {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        for _ in 0..100 {
            let mut staged = path.as_os_str().to_owned();
            staged.push(format!(
                ".restoring-{}-{}-{}",
                std::process::id(),
                now.as_nanos(),
                SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let staged = PathBuf::from(staged);
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&staged)
            {
                Ok(file) => {
                    return Ok(Self {
                        path: staged,
                        file: Some(file),
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "no free name to stage the restored configuration under",
        ))
    }

    /// Write the validated bytes, owner-only, and flush them to disk.
    fn write(&self, contents: &[u8]) -> std::io::Result<()> {
        let mut file = self.file.as_ref().expect("staged file is open");
        file.write_all(contents)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.sync_all()
    }

    /// Atomically rename the staged file over `path` and make the rename
    /// durable by syncing the folder.
    fn install(mut self, path: &Path) -> std::io::Result<()> {
        self.file.take();
        fs::rename(&self.path, path)?;
        self.path = PathBuf::new();
        if let Some(folder) = path.parent() {
            fs::File::open(folder)?.sync_all()?;
        }
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Preserve the broken configuration under a new
/// `<name>.broken-<seconds>-<nanoseconds>` hard link, leaving the original in
/// place. Returns the link, or `None` if there is no configuration file.
///
/// A hard link fails instead of overwriting when the name exists; a taken
/// name (two recoveries within the same instant, or a leftover file) moves on
/// to a numbered variant.
fn preserve_broken(path: &Path) -> std::io::Result<Option<PathBuf>> {
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
            Ok(()) => return Ok(Some(kept)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free name to keep the broken configuration under",
    ))
}

/// Move the broken configuration aside to start a fresh library: preserve it
/// under a `.broken-*` link, then remove the original. The original is
/// removed only after the link succeeds, so it is never lost.
fn keep_broken(path: &Path) -> std::io::Result<Option<PathBuf>> {
    let kept = preserve_broken(path)?;
    if let Some(kept) = &kept {
        if let Err(error) = fs::remove_file(path) {
            let _ = fs::remove_file(kept);
            return Err(error);
        }
    }
    Ok(kept)
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

    const VALID: &str = "[[expansion]]\ntrigger = \";ok\"\nreplacement = \"fine\"\n";

    fn write_private(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn staging_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".restoring"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_live_configuration_exists_throughout_a_restore() {
        let dir = scratch("crash-window");
        let config = dir.join("expansions.toml");
        write_private(&config, "this is = = not toml");
        let backup = dir.join("expansions.toml.bak");
        write_private(&backup, VALID);

        // Stop at the last moment before the rename, as a crash or kill
        // would: the active configuration must still be there, unchanged.
        let error = restore_backup_with(&config, &backup, || {
            assert_eq!(
                fs::read_to_string(&config).unwrap(),
                "this is = = not toml",
                "the live configuration is present after the broken copy is linked"
            );
            Err(std::io::Error::other("simulated termination"))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "simulated termination");
        assert_eq!(fs::read_to_string(&config).unwrap(), "this is = = not toml");
        assert!(staging_files(&dir).is_empty(), "the staged copy is removed");

        restore_backup(&config, &backup).unwrap();
        assert_eq!(fs::read_to_string(&config).unwrap(), VALID);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_backup_damaged_after_listing_is_not_installed() {
        let dir = scratch("stale");
        let config = dir.join("expansions.toml");
        write_private(&config, "broken");
        let backup = dir.join("expansions.toml.bak");
        write_private(&backup, VALID);
        assert!(find_backups(&config)[0].valid);

        write_private(&backup, "truncat");
        let error = restore_backup(&config, &backup).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&config).unwrap(), "broken");
        assert!(staging_files(&dir).is_empty());
        let kept = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".broken-"))
            .count();
        assert_eq!(kept, 0, "nothing is moved aside for a refused restore");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_restore_never_touches_another_restores_staging_file() {
        let dir = scratch("foreign-stage");
        let config = dir.join("expansions.toml");
        write_private(&config, "broken");
        let backup = dir.join("expansions.toml.bak");
        write_private(&backup, VALID);
        let legacy = dir.join("expansions.toml.restoring");
        let other = dir.join("expansions.toml.restoring-1-2-3");
        write_private(&legacy, "someone else's");
        write_private(&other, "someone else's");

        restore_backup(&config, &backup).unwrap();
        assert_eq!(fs::read_to_string(&legacy).unwrap(), "someone else's");
        assert_eq!(fs::read_to_string(&other).unwrap(), "someone else's");
        assert_eq!(
            staging_files(&dir),
            [
                "expansions.toml.restoring",
                "expansions.toml.restoring-1-2-3"
            ]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_restores_run_one_at_a_time() {
        use std::sync::{mpsc, Arc, Mutex};
        let dir = scratch("concurrent");
        let config = dir.join("expansions.toml");
        write_private(&config, "broken");
        let first = dir.join("expansions.toml.bak.1");
        let second = dir.join("expansions.toml.bak.2");
        write_private(&first, VALID);
        let second_contents = VALID.replace("fine", "second");
        write_private(&second, &second_contents);

        let order = Arc::new(Mutex::new(Vec::new()));
        let (entered, inside) = mpsc::channel();
        let holder = {
            let (config, first, order) = (config.clone(), first.clone(), order.clone());
            std::thread::spawn(move || {
                restore_backup_with(&config, &first, || {
                    entered.send(()).unwrap();
                    // Give the second restore time to run if the lock did
                    // not hold it back.
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    order.lock().unwrap().push("first-finishing");
                    Ok(())
                })
            })
        };
        inside.recv().unwrap();
        restore_backup(&config, &second).unwrap();
        order.lock().unwrap().push("second-done");
        holder.join().unwrap().unwrap();

        assert_eq!(*order.lock().unwrap(), ["first-finishing", "second-done"]);
        assert_eq!(fs::read_to_string(&config).unwrap(), second_contents);
        assert!(staging_files(&dir).is_empty());
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
