//! Filesystem ownership and path validation for broker policy values.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// systemd user services that use `ReadWritePaths=` may run in a mount/user
/// namespace where host-root-owned ancestors appear as UID 65534 (unmapped
/// nobody). Treat that representation as root only when `/proc` confirms that
/// namespace UID 0 is unmapped.
pub fn is_root_owner(uid: u32) -> bool {
    uid == 0
        || (uid == 65_534
            && fs::read_to_string("/proc/self/uid_map")
                .ok()
                .is_some_and(|mapping| {
                    !mapping.lines().any(|line| {
                        let mut fields = line.split_whitespace();
                        let Some(namespace_start) =
                            fields.next().and_then(|v| v.parse::<u64>().ok())
                        else {
                            return false;
                        };
                        let Some(_host_start) = fields.next() else {
                            return false;
                        };
                        let Some(length) = fields.next().and_then(|v| v.parse::<u64>().ok()) else {
                            return false;
                        };
                        namespace_start == 0 && length > 0
                    })
                }))
}

pub fn is_user_or_root_owner(uid: u32, current_uid: u32) -> bool {
    uid == current_uid || is_root_owner(uid)
}

pub(crate) fn validate_working_directory(label: &str, directory: &str) -> Result<(), String> {
    let path = Path::new(directory);
    if !path.is_absolute() {
        return Err(format!("{} must be an absolute path", label));
    }
    let resolved = fs::canonicalize(path)
        .map_err(|error| format!("{} '{}' cannot be resolved: {}", label, directory, error))?;
    validate_path_ancestors(&resolved, label, false)?;
    let metadata = fs::metadata(&resolved)
        .map_err(|error| format!("{} '{}' cannot be inspected: {}", label, directory, error))?;
    if !metadata.is_dir() {
        return Err(format!("{} '{}' is not a directory", label, directory));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "{} '{}' is writable by group or other users",
            label, directory
        ));
    }
    let uid = rustix::process::geteuid().as_raw();
    if !is_user_or_root_owner(metadata.uid(), uid) {
        return Err(format!(
            "{} '{}' is not owned by the current user or root",
            label, directory
        ));
    }
    Ok(())
}

pub(crate) fn validate_audit_path(path: &str) -> Result<(), String> {
    let path = resolve_audit_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| "audit_path has no parent directory".to_string())?;
    let parent = fs::canonicalize(parent)
        .map_err(|error| format!("audit_path parent cannot be resolved: {error}"))?;
    validate_path_ancestors(&parent, "audit_path", false)?;
    let parent_metadata = fs::metadata(&parent)
        .map_err(|error| format!("audit_path parent cannot be inspected: {error}"))?;
    let uid = rustix::process::geteuid().as_raw();
    if !parent_metadata.is_dir()
        || (parent_metadata.mode() & 0o022 != 0)
        || !is_user_or_root_owner(parent_metadata.uid(), uid)
    {
        return Err("audit_path parent directory is not private and trusted".to_string());
    }
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.is_file() {
            return Err("audit_path must name a regular file".to_string());
        }
        if !is_user_or_root_owner(metadata.uid(), uid) {
            return Err("audit_path is not owned by the current user or root".to_string());
        }
        let mode = metadata.mode() & 0o777;
        if (metadata.uid() == uid && mode != 0o600)
            || (is_root_owner(metadata.uid()) && mode & 0o022 != 0)
        {
            return Err("audit_path permissions are insecure".to_string());
        }
    }
    Ok(())
}

pub(crate) fn resolve_audit_path(path: &str) -> Result<PathBuf, String> {
    let path = if let Some(suffix) = path.strip_prefix("$XDG_STATE_HOME") {
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| Path::new(&home).join(".local/state").into_os_string())
            })
            .ok_or_else(|| {
                "audit_path uses $XDG_STATE_HOME but neither XDG_STATE_HOME nor HOME is set"
                    .to_string()
            })?;
        Path::new(&state_home).join(suffix.trim_start_matches('/'))
    } else if let Some(suffix) = path.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| "audit_path uses ~ but HOME is not set".to_string())?;
        Path::new(&home).join(suffix)
    } else {
        Path::new(path).to_path_buf()
    };
    if !path.is_absolute() {
        return Err(
            "audit_path must be an absolute path or use $XDG_STATE_HOME/ or ~/".to_string(),
        );
    }
    Ok(path)
}

pub(crate) fn validate_absolute_program(action_id: &str, program: &str) -> Result<(), String> {
    let path = Path::new(program);
    if !path.is_absolute() {
        return Err(format!(
            "action '{}': program must be absolute path when require_absolute_paths=true",
            action_id
        ));
    }
    let resolved = fs::canonicalize(path).map_err(|error| {
        format!(
            "action '{}': program '{}' cannot be inspected: {}",
            action_id, program, error
        )
    })?;
    validate_path_ancestors(&resolved, &format!("action '{}' program", action_id), true)?;
    let metadata = fs::metadata(&resolved).map_err(|error| {
        format!(
            "action '{}': program '{}' cannot be inspected: {}",
            action_id, program, error
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "action '{}': program '{}' is not a regular file",
            action_id, program
        ));
    }
    if metadata.mode() & 0o111 == 0 {
        return Err(format!(
            "action '{}': program '{}' is not executable",
            action_id, program
        ));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "action '{}': program '{}' is writable by group or other users",
            action_id, program
        ));
    }
    let uid = rustix::process::geteuid().as_raw();
    if !is_user_or_root_owner(metadata.uid(), uid) {
        return Err(format!(
            "action '{}': program '{}' is not owned by the current user or root",
            action_id, program
        ));
    }
    Ok(())
}

fn validate_path_ancestors(
    path: &Path,
    label: &str,
    allow_trusted_sticky: bool,
) -> Result<(), String> {
    let uid = rustix::process::geteuid().as_raw();
    let mut current = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", label))?;
    loop {
        let metadata = fs::metadata(current).map_err(|error| {
            format!(
                "{} ancestor '{}' cannot be inspected: {}",
                label,
                current.display(),
                error
            )
        })?;
        if !metadata.is_dir() {
            return Err(format!(
                "{} ancestor '{}' is not a directory",
                label,
                current.display()
            ));
        }
        if !is_user_or_root_owner(metadata.uid(), uid) {
            return Err(format!(
                "{} ancestor '{}' is not owned by the current user or root",
                label,
                current.display()
            ));
        }
        if metadata.mode() & 0o022 != 0 {
            let trusted_sticky = allow_trusted_sticky
                && metadata.mode() & 0o1000 != 0
                && is_user_or_root_owner(metadata.uid(), uid);
            if !trusted_sticky {
                return Err(format!(
                    "{} ancestor '{}' is writable by group or other users",
                    label,
                    current.display()
                ));
            }
        }
        if current == Path::new("/") {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| format!("{} ancestor traversal failed", label))?;
    }
    Ok(())
}
