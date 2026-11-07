//! Small, content-free continuation marker for setup that spans a desktop login.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const GNOME_IBUS_RESUME_MARKER: &str = "setup-resume-gnome-ibus";
const MARKER_VERSION: &str = "wayexpand-setup-resume-v2";
const MAX_RESUME_AGE_SECS: u64 = 14 * 24 * 60 * 60;

fn marker_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(GNOME_IBUS_RESUME_MARKER)
}

/// True only for a current, versioned GNOME Wayland transaction that has not
/// expired. The environment check intentionally uses complete desktop tokens
/// so values such as `KDE:GNOME-compat` cannot accidentally resume it.
pub fn gnome_ibus_setup_resume_pending(config_path: &Path) -> bool {
    let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let is_gnome = desktops
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("gnome"));
    let is_wayland = std::env::var("XDG_SESSION_TYPE")
        .is_ok_and(|session| session.eq_ignore_ascii_case("wayland"));
    gnome_ibus_setup_resume_pending_for_session(config_path, is_gnome, is_wayland)
}

/// Check a resume marker against the detected session. Exposed separately so
/// setup and the GUI can use their compositor probe rather than trusting a
/// marker to choose the backend.
pub fn gnome_ibus_setup_resume_pending_for_session(
    config_path: &Path,
    is_gnome: bool,
    is_wayland: bool,
) -> bool {
    is_gnome && is_wayland && gnome_ibus_setup_resume_transaction_pending(config_path)
}

/// Whether a valid, unexpired transaction exists, regardless of the current
/// desktop. This is for diagnostics and explicit cancellation guidance only;
/// callers must use the session-aware predicate before resuming it.
pub fn gnome_ibus_setup_resume_transaction_pending(config_path: &Path) -> bool {
    let path = marker_path(config_path);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return false;
    };
    if !metadata.file_type().is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return false;
    }
    fs::read_to_string(path).is_ok_and(|contents| is_current_transaction(&contents, unix_now()))
}

/// A private user-owned marker is present, including an expired or
/// unsupported transaction that must not be resumed automatically.
pub fn gnome_ibus_setup_resume_marker_present(config_path: &Path) -> bool {
    let path = marker_path(config_path);
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o077 == 0
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_current_transaction(contents: &str, now: u64) -> bool {
    let mut lines = contents.lines();
    if lines.next() != Some(MARKER_VERSION)
        || lines.next() != Some("desktop=gnome")
        || lines.next() != Some("session=wayland")
    {
        return false;
    }
    let Some(created) = lines
        .next()
        .and_then(|line| line.strip_prefix("created_at_unix="))
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return false;
    };
    lines.next().is_none()
        && created <= now.saturating_add(300)
        && now.saturating_sub(created) <= MAX_RESUME_AGE_SECS
}

/// Persist only the fixed continuation action and its originating session and
/// timestamp. No user choices or consent are stored.
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
        write!(
            file,
            "{MARKER_VERSION}\ndesktop=gnome\nsession=wayland\ncreated_at_unix={}\n",
            unix_now()
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn transaction(created_at: u64) -> String {
        format!("{MARKER_VERSION}\ndesktop=gnome\nsession=wayland\ncreated_at_unix={created_at}\n")
    }

    #[test]
    fn resume_requires_matching_gnome_wayland_session() {
        let now = unix_now();
        let current = transaction(now);
        assert!(is_current_transaction(&current, now));
        assert!(!is_current_transaction(
            &current,
            now + MAX_RESUME_AGE_SECS + 1
        ));
        assert!(!is_current_transaction(&transaction(now + 301), now));
        assert!(!is_current_transaction(
            &format!("{MARKER_VERSION}\ndesktop=kde\nsession=wayland\ncreated_at_unix={now}\n"),
            now
        ));
        assert!(!is_current_transaction(
            "wayexpand-setup-resume-v1 gnome-ibus\n",
            now
        ));
    }

    #[test]
    fn marker_is_versioned_private_and_cancelable() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "wayexpand-setup-resume-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let config = dir.join("expansions.toml");

        mark_gnome_ibus_setup_resume(&config).unwrap();
        assert!(gnome_ibus_setup_resume_marker_present(&config));
        assert!(gnome_ibus_setup_resume_pending_for_session(
            &config, true, true
        ));
        assert!(!gnome_ibus_setup_resume_pending_for_session(
            &config, false, true
        ));
        assert!(!gnome_ibus_setup_resume_pending_for_session(
            &config, true, false
        ));
        let payload = fs::read_to_string(marker_path(&config)).unwrap();
        assert!(payload.starts_with(
            "wayexpand-setup-resume-v2\ndesktop=gnome\nsession=wayland\ncreated_at_unix="
        ));

        clear_gnome_ibus_setup_resume(&config).unwrap();
        assert!(!gnome_ibus_setup_resume_marker_present(&config));
        fs::remove_dir_all(dir).unwrap();
    }
}
