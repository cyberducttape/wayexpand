//! Secure control-socket path resolution and stale-entry handling.

use anyhow::{bail, Context, Result};
use std::{
    fs,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
pub(super) fn open_socket_parent(path: &Path) -> Result<(fs::File, PathBuf)> {
    let parent = path.parent().unwrap_or_else(|| Path::new("/"));
    let root = rustix::fs::open(
        "/",
        rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let relative = parent.strip_prefix("/").unwrap_or(parent);
    let (directory, use_proc_fd_path) = match rustix::fs::openat2(
        &root,
        relative,
        rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_SYMLINKS,
    ) {
        Ok(directory) => (directory, true),
        Err(error) if error == rustix::io::Errno::NOSYS => (
            rustix::fs::open(
                parent,
                rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            )?,
            false,
        ),
        Err(error) => return Err(error.into()),
    };
    let guard = fs::File::from(directory);
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("control socket path has no file name"))?;
    let operation_path = if use_proc_fd_path {
        PathBuf::from(format!(
            "/proc/self/fd/{}/{}",
            guard.as_raw_fd(),
            name.to_string_lossy()
        ))
    } else {
        path.to_path_buf()
    };
    Ok((guard, operation_path))
}

pub(super) fn is_owned_socket(metadata: &std::fs::Metadata, uid: rustix::process::RawUid) -> bool {
    metadata.file_type().is_socket() && metadata.uid() == uid
}

pub(super) fn validate_socket_parent(path: &Path) -> Result<()> {
    secure_socket_path(path).map(|_| ())
}

pub(super) fn secure_socket_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    let resolved_parent = fs::canonicalize(parent)
        .with_context(|| format!("resolving control socket directory {}", parent.display()))?;
    let current_uid = rustix::process::geteuid().as_raw();
    let mut current = resolved_parent.as_path();
    loop {
        let metadata = fs::metadata(current)
            .with_context(|| format!("checking control socket directory {}", current.display()))?;
        if !metadata.is_dir() {
            bail!(
                "control socket parent {} is not a directory",
                current.display()
            );
        }
        let is_system_dir = current == Path::new("/")
            || current == Path::new("/home")
            || current == Path::new("/run")
            || current == Path::new("/run/user");
        if !is_system_dir && metadata.uid() != current_uid && metadata.uid() != 0 {
            bail!(
                "control socket directory {} is not owned by the current user or root",
                current.display()
            );
        }
        let mode = metadata.mode() & 0o7777;
        if !socket_parent_mode_is_secure(mode) {
            bail!(
                "control socket directory {} is writable by group or other users",
                current.display()
            );
        }
        if current == Path::new("/") {
            break;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
    let name = path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow::anyhow!("control socket path has no file name"))?;
    Ok(resolved_parent.join(name))
}

pub(super) fn socket_parent_mode_is_secure(mode: u32) -> bool {
    mode & 0o022 == 0 || mode & 0o1000 != 0
}

pub(super) fn is_original_socket(
    metadata: &std::fs::Metadata,
    identity: (u64, u64),
    uid: rustix::process::RawUid,
) -> bool {
    is_owned_socket(metadata, uid) && (metadata.dev(), metadata.ino()) == identity
}
