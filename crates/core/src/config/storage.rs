//! Secure configuration storage: atomic replacement, the advisory write lock, and parent-directory ownership and mode checks.

use super::*;

/// Replace a configuration through an opened parent directory. Once the
/// directory descriptor is acquired, an attacker cannot redirect the temp
/// file or final rename by swapping a path component between validation and
/// replacement.
#[cfg(target_os = "linux")]
pub(super) fn save_atomic_serialized_relative(
    resolved: &Path,
    file_name: &str,
    serialized: &[u8],
) -> Result<(), ConfigError> {
    let parent = resolved.parent().unwrap_or_else(|| Path::new("."));
    let parent_fd = open_secure_directory(parent).map_err(|source| ConfigError::Read {
        path: parent.display().to_string(),
        source,
    })?;
    let mut last_error = None;
    for attempt in 0..16 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let temp_name = format!(
            ".{file_name}.tmp.{}.{}.{}",
            std::process::id(),
            nonce,
            attempt
        );
        let temp_fd = match rustix::fs::openat(
            &parent_fd,
            &temp_name,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(0o600),
        ) {
            Ok(fd) => fd,
            Err(error) if error == rustix::io::Errno::EXIST => {
                last_error = Some(ConfigError::Read {
                    path: temp_name,
                    source: error.into(),
                });
                continue;
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: parent.display().to_string(),
                    source: source.into(),
                });
            }
        };
        let temp_path = parent.join(&temp_name);
        let result = (|| -> Result<(), ConfigError> {
            let mut file = fs::File::from(temp_fd);
            file.write_all(serialized)
                .map_err(|source| ConfigError::Read {
                    path: temp_path.display().to_string(),
                    source,
                })?;
            file.sync_all().map_err(|source| ConfigError::Read {
                path: temp_path.display().to_string(),
                source,
            })?;
            Config::validate_save_target_owner(resolved)?;
            rustix::fs::renameat(&parent_fd, &temp_name, &parent_fd, file_name).map_err(
                |source| ConfigError::Read {
                    path: resolved.display().to_string(),
                    source: source.into(),
                },
            )?;
            parent_fd
                .try_clone()
                .and_then(|directory| directory.sync_all())
                .map_err(|source| ConfigError::Read {
                    path: parent.display().to_string(),
                    source,
                })?;
            Ok(())
        })();
        if result.is_err() {
            let _ = rustix::fs::unlinkat(&parent_fd, &temp_name, rustix::fs::AtFlags::empty());
        }
        return result;
    }
    Err(last_error.unwrap_or_else(|| ConfigError::Read {
        path: parent.display().to_string(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "temporary path collision",
        ),
    }))
}

#[cfg(target_os = "linux")]
pub(super) fn open_secure_directory(path: &Path) -> std::io::Result<fs::File> {
    let root = rustix::fs::open(
        "/",
        rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let relative = path.strip_prefix("/").unwrap_or(path);
    let directory = rustix::fs::openat2(
        &root,
        relative,
        rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_SYMLINKS,
    );
    match directory {
        Ok(directory) => Ok(fs::File::from(directory)),
        // Older kernels and some seccomp profiles reject openat2. Walk each
        // component using directory descriptors and O_NOFOLLOW instead; this
        // retains the no-symlink guarantee rather than falling back to a
        // path-based open.
        Err(error)
            if error == rustix::io::Errno::NOSYS
                || error == rustix::io::Errno::PERM
                || error == rustix::io::Errno::INVAL =>
        {
            open_secure_directory_walk(root, relative)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
fn open_secure_directory_walk(root: OwnedFd, relative: &Path) -> std::io::Result<fs::File> {
    let mut current = root;
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "configuration path must not contain dot components",
            ));
        }
        current = rustix::fs::openat(
            &current,
            component.as_os_str(),
            rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
        )?;
    }
    Ok(fs::File::from(current))
}

#[cfg(all(test, target_os = "linux"))]
mod fallback_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn descriptor_walk_accepts_directories_and_rejects_symlinks() {
        let root_path = std::env::temp_dir().join(format!(
            "wayexpand-openat-fallback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real = root_path.join("real");
        fs::create_dir_all(&real).unwrap();
        symlink(&real, root_path.join("linked")).unwrap();

        let root_fd = rustix::fs::open(
            "/",
            rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let relative_root = root_path.strip_prefix("/").unwrap();
        assert!(open_secure_directory_walk(root_fd.try_clone().unwrap(), relative_root).is_ok());
        assert!(open_secure_directory_walk(root_fd, &relative_root.join("linked")).is_err());

        fs::remove_file(root_path.join("linked")).unwrap();
        fs::remove_dir(&real).unwrap();
        fs::remove_dir(root_path).unwrap();
    }
}

pub(super) fn resolve_config_target(path: &Path) -> Result<std::path::PathBuf, ConfigError> {
    match fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let parent = fs::canonicalize(parent).map_err(|source| ConfigError::Read {
                path: parent.display().to_string(),
                source,
            })?;
            let file_name = path.file_name().ok_or_else(|| ConfigError::Read {
                path: path.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "configuration path has no file name",
                ),
            })?;
            Ok(parent.join(file_name))
        }
        Err(source) => Err(ConfigError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

pub(super) struct ConfigWriteLock {
    _file: fs::File,
}

impl ConfigWriteLock {
    pub(super) fn acquire(target: &Path) -> Result<Self, ConfigError> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let name = target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("expansions.toml");
        let lock_path = parent.join(format!(".{name}.wayexpand.lock"));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&lock_path)
            .map_err(|source| ConfigError::Read {
                path: lock_path.display().to_string(),
                source,
            })?;
        let metadata = file.metadata().map_err(|source| ConfigError::Read {
            path: lock_path.display().to_string(),
            source,
        })?;
        let uid = rustix::process::geteuid().as_raw();
        if !metadata.file_type().is_file() {
            return Err(ConfigError::NotRegular {
                path: lock_path.display().to_string(),
            });
        }
        if metadata.uid() != uid {
            return Err(ConfigError::InsecureOwner {
                path: lock_path.display().to_string(),
                uid: metadata.uid(),
            });
        }
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 || metadata.nlink() != 1 {
            return Err(ConfigError::InsecurePermissions {
                path: lock_path.display().to_string(),
                mode,
            });
        }
        let deadline = Instant::now() + CONFIG_LOCK_TIMEOUT;
        loop {
            // SAFETY: `file` remains alive for this guard's lifetime and owns
            // a valid descriptor. flock does not retain the pointer.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result == 0 {
                return Ok(Self { _file: file });
            }
            let source = std::io::Error::last_os_error();
            if source
                .raw_os_error()
                .is_some_and(|error| error == libc::EWOULDBLOCK || error == libc::EAGAIN)
            {
                if Instant::now() >= deadline {
                    return Err(ConfigError::Busy {
                        path: lock_path.display().to_string(),
                    });
                }
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            if source.kind() != std::io::ErrorKind::Interrupted {
                return Err(ConfigError::Read {
                    path: lock_path.display().to_string(),
                    source,
                });
            }
        }
    }
}

pub(super) fn validate_parent_directories(path: &Path) -> Result<(), ConfigError> {
    let mut current = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let current_uid = rustix::process::geteuid().as_raw();
    loop {
        let metadata = fs::metadata(current).map_err(|source| ConfigError::Read {
            path: current.display().to_string(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(ConfigError::Read {
                path: current.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "configuration parent is not a directory",
                ),
            });
        }
        let mode = metadata.permissions().mode() & 0o7777;
        let uid = metadata.uid();
        // Skip ownership check for system directories (/home, /) where UID
        // remapping in containers may cause unexpected ownership. User-owned
        // config directories still validate strictly.
        let is_system_dir = current == Path::new("/") || current == Path::new("/home");
        if !is_system_dir && uid != current_uid && uid != 0 {
            return Err(ConfigError::InsecureParentOwner {
                path: current.display().to_string(),
                uid,
            });
        }
        // Trust is determined by writeability, not ownership. A root-owned
        // directory with group/other write bits is still replaceable by an
        // unprivileged user and must be rejected unless sticky protection is
        // present. The filesystem root is normally 0755, so it needs no
        // special exemption.
        if !parent_mode_is_secure(mode) {
            return Err(ConfigError::InsecureParent {
                path: current.display().to_string(),
                mode,
            });
        }
        // NOTE: We intentionally do NOT stop at the first user-owned directory.
        // While a secure user-owned directory itself cannot be swapped
        // (it requires write access to its parent), a world-writable,
        // non-sticky parent directory can still allow another user to
        // rename/replace that directory entry.
        //
        // Example: /shared is world-writable and non-sticky, /shared/stephan
        // is 0700 and owned by stephan. A different user CAN rename
        // /shared/stephan to /shared/stephan.bak and create a new
        // /shared/stephan pointing to attacker-controlled config.
        //
        // Similarly, a root-owned world-writable parent can be exploited even
        // though the child is root-owned. We validate all ancestors including
        // the root-owned filesystem root ("/"), accepting it as a terminal
        // trust anchor since the filesystem itself is the trust boundary.
        // In containerized/namespaced environments, this prevents false
        // rejections while maintaining protection against directory swaps.
        //
        // Linux save operations additionally open this parent with openat2
        // and perform temp creation/replacement relative to that descriptor.
        // The path walk remains here for portable validation and diagnostics.
        if current == Path::new("/") {
            // Reached filesystem root. Root-owned "/" is a trust anchor.
            // In systemd private namespaces, uid 65534 (overflow) may appear;
            // accept it as validation is constrained to namespace boundary.
            break;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
    Ok(())
}

/// A root-owned config only represents administrator-managed policy when an
/// unprivileged directory owner cannot replace its directory entry. Require
/// every directory in its ancestry to be root-owned; the ordinary parent
/// validator separately checks permissions and sticky-directory semantics.
pub(super) fn validate_root_managed_parent_chain(path: &Path) -> Result<(), ConfigError> {
    let mut current = path.parent().unwrap_or_else(|| Path::new("/"));
    loop {
        let metadata = fs::metadata(current).map_err(|source| ConfigError::Read {
            path: current.display().to_string(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(ConfigError::Read {
                path: current.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "configuration parent is not a directory",
                ),
            });
        }
        if !root_managed_parent_owner_allowed(metadata.uid()) {
            return Err(ConfigError::InsecureParentOwner {
                path: current.display().to_string(),
                uid: metadata.uid(),
            });
        }
        if current == Path::new("/") {
            break;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
    Ok(())
}

pub(super) fn root_managed_parent_owner_allowed(uid: u32) -> bool {
    uid == 0
}

pub(super) fn root_owned_target_requires_admin(target_uid: u32, current_uid: u32) -> bool {
    target_uid == 0 && current_uid != 0
}

pub(super) fn parent_mode_is_secure(mode: u32) -> bool {
    mode & 0o022 == 0 || mode & 0o1000 != 0
}
