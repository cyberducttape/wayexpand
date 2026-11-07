//! Small, content-free continuation marker for setup that spans a desktop login.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const GNOME_IBUS_RESUME_MARKER: &str = "setup-resume-gnome-ibus";
const MARKER_CONTENT: &[u8] = b"wayexpand-setup-resume-v1 gnome-ibus\n";

fn marker_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(GNOME_IBUS_RESUME_MARKER)
}

/// True only for a regular, user-owned marker with the exact known payload.
pub fn gnome_ibus_setup_resume_pending(config_path: &Path) -> bool {
    let path = marker_path(config_path);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return false;
    };
    metadata.file_type().is_file()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.permissions().mode() & 0o077 == 0
        && fs::read(path).is_ok_and(|contents| contents == MARKER_CONTENT)
}

/// Persist only the fixed continuation action. No user choices or consent are stored.
pub fn mark_gnome_ibus_setup_resume(config_path: &Path) -> std::io::Result<()> {
    let path = marker_path(config_path);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(
        ".{GNOME_IBUS_RESUME_MARKER}.{}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(MARKER_CONTENT)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn clear_gnome_ibus_setup_resume(config_path: &Path) -> std::io::Result<()> {
    let path = marker_path(config_path);
    match fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.file_type().is_file() && metadata.uid() == unsafe { libc::geteuid() } =>
        {
            fs::remove_file(path)?;
            let parent = marker_path(config_path);
            let parent = parent.parent().unwrap_or_else(|| Path::new("."));
            fs::File::open(parent)?.sync_all()?;
            Ok(())
        }
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "setup continuation marker is not a regular user-owned file",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
