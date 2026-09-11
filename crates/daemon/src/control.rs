use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

const MAX_COMMAND_BYTES: usize = 128;
const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(2);

pub struct ControlServer {
    pub reload_requested: Arc<AtomicBool>,
    pub stop_requested: Arc<AtomicBool>,
    pub pause_requested: Arc<AtomicBool>,
    status: Arc<Mutex<String>>,
    path: Option<PathBuf>,
    socket_identity: Option<(u64, u64)>,
}

impl ControlServer {
    pub fn start() -> Result<Self> {
        let Some(requested_path) = socket_path() else {
            return Ok(Self {
                reload_requested: Arc::new(AtomicBool::new(false)),
                stop_requested: Arc::new(AtomicBool::new(false)),
                pause_requested: Arc::new(AtomicBool::new(false)),
                status: Arc::new(Mutex::new("starting".into())),
                path: None,
                socket_identity: None,
            });
        };
        let path = secure_socket_path(&requested_path)?;
        validate_socket_parent(&path)?;
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            match UnixStream::connect(&path) {
                Ok(_) => bail!(
                    "another WayExpand daemon is already using {}",
                    path.display()
                ),
                Err(_) => {
                    let owner = rustix::process::geteuid().as_raw();
                    if !is_owned_socket(&metadata, owner) {
                        if !metadata.file_type().is_socket() {
                            bail!(
                                "refusing to remove non-socket control path {}",
                                path.display()
                            );
                        }
                        bail!(
                            "refusing to remove control socket {} owned by uid {}",
                            path.display(),
                            metadata.uid()
                        );
                    }
                    let identity = (metadata.dev(), metadata.ino());
                    let current = fs::symlink_metadata(&path)
                        .with_context(|| format!("rechecking stale socket {}", path.display()))?;
                    if !is_original_socket(&current, identity, owner) {
                        bail!(
                            "control path {} changed while checking stale socket",
                            path.display()
                        );
                    }
                    fs::remove_file(&path)
                        .with_context(|| format!("removing stale socket {}", path.display()))?
                }
            }
        }
        // A restrictive umask closes the permission window between bind and
        // chmod. Restore the caller's mask immediately after bind so this
        // process does not change unrelated file-creation behavior.
        let previous_umask = rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
        let listener_result = UnixListener::bind(&path);
        rustix::process::umask(previous_umask);
        let listener = listener_result
            .with_context(|| format!("binding control socket {}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&path)?;
        let socket_identity = Some((metadata.dev(), metadata.ino()));
        let reload_requested = Arc::new(AtomicBool::new(false));
        let stop_requested = Arc::new(AtomicBool::new(false));
        let pause_requested = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new("starting".into()));
        let reload_flag = Arc::clone(&reload_requested);
        let stop_flag = Arc::clone(&stop_requested);
        let pause_flag = Arc::clone(&pause_requested);
        let status_flag = Arc::clone(&status);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                if handle_request(stream, &reload_flag, &stop_flag, &pause_flag, &status_flag)
                    .is_err()
                {
                    // The control socket is best-effort and must never take
                    // down the keyboard/expansion loop.
                    continue;
                }
                if stop_flag.load(Ordering::Acquire) {
                    break;
                }
            }
        });
        Ok(Self {
            reload_requested,
            stop_requested,
            pause_requested,
            status,
            path: Some(path),
            socket_identity,
        })
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn set_status(&self, status: impl Into<String>) {
        if let Ok(mut current) = self.status.lock() {
            *current = status.into();
        }
    }
}

fn is_owned_socket(metadata: &std::fs::Metadata, uid: rustix::process::RawUid) -> bool {
    metadata.file_type().is_socket() && metadata.uid() == uid
}

fn validate_socket_parent(path: &Path) -> Result<()> {
    secure_socket_path(path).map(|_| ())
}

fn secure_socket_path(path: &Path) -> Result<PathBuf> {
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
        if metadata.uid() != current_uid && metadata.uid() != 0 {
            bail!(
                "control socket directory {} is not owned by the current user or root",
                current.display()
            );
        }
        let mode = metadata.mode() & 0o7777;
        // A root-owned sticky directory such as /tmp protects entry removal;
        // it is safe as an ancestor, but not as the immediate socket parent.
        let root_sticky = metadata.uid() == 0 && mode & 0o1000 != 0;
        if mode & 0o022 != 0 && (!root_sticky || current == resolved_parent) {
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

fn is_original_socket(
    metadata: &std::fs::Metadata,
    identity: (u64, u64),
    uid: rustix::process::RawUid,
) -> bool {
    is_owned_socket(metadata, uid) && (metadata.dev(), metadata.ino()) == identity
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        if let (Some(path), Some(identity)) = (&self.path, self.socket_identity) {
            if let Ok(metadata) = fs::symlink_metadata(path) {
                let uid = rustix::process::geteuid().as_raw();
                if is_original_socket(&metadata, identity, uid) {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
}

fn handle_request(
    mut stream: UnixStream,
    reload: &AtomicBool,
    stop: &AtomicBool,
    pause: &AtomicBool,
    status: &Mutex<String>,
) -> Result<()> {
    stream.set_read_timeout(Some(CONTROL_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_IO_TIMEOUT))?;
    let mut command_bytes = Vec::with_capacity(MAX_COMMAND_BYTES);
    let mut chunk = [0_u8; 64];
    let mut oversized = false;
    loop {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let remaining = MAX_COMMAND_BYTES.saturating_sub(command_bytes.len());
        let retained = count.min(remaining);
        command_bytes.extend_from_slice(&chunk[..retained]);
        oversized |= retained != count;
        if chunk[..count].contains(&b'\n') {
            break;
        }
    }
    let command = if oversized {
        String::new()
    } else {
        String::from_utf8_lossy(&command_bytes).into_owned()
    };
    let response = match command.trim() {
        "status" => format!(
            "running\n{}\n",
            status
                .lock()
                .map_err(|_| anyhow::anyhow!("status lock poisoned"))?
        ),
        "reload" => {
            reload.store(true, Ordering::Release);
            "reload scheduled\n".to_string()
        }
        "stop" => {
            stop.store(true, Ordering::Release);
            "stopping\n".to_string()
        }
        "pause" => {
            pause.store(true, Ordering::Release);
            "paused\n".to_string()
        }
        "resume" => {
            pause.store(false, Ordering::Release);
            "resumed\n".to_string()
        }
        _ => "unknown command; expected status, reload, pause, resume, or stop\n".to_string(),
    };
    stream.write_all(response.as_bytes())?;
    Ok(())
}

pub fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WAYEXPAND_SOCKET") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::Ordering;

    fn request(
        command: &str,
        reload: &Arc<AtomicBool>,
        stop: &Arc<AtomicBool>,
        pause: &Arc<AtomicBool>,
        status: &Arc<Mutex<String>>,
    ) -> String {
        let (mut client, server) = UnixStream::pair().unwrap();
        let reload_worker = Arc::clone(reload);
        let stop_worker = Arc::clone(stop);
        let pause_worker = Arc::clone(pause);
        let status_worker = Arc::clone(status);
        let join = thread::spawn(move || {
            handle_request(
                server,
                &reload_worker,
                &stop_worker,
                &pause_worker,
                &status_worker,
            )
            .unwrap();
        });
        client.write_all(command.as_bytes()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        join.join().unwrap();
        response
    }

    #[test]
    fn lifecycle_commands_set_flags_and_reply() {
        let reload = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new("source=stdin\nbackend=none".into()));
        assert_eq!(
            request("status\n", &reload, &stop, &pause, &status),
            "running\nsource=stdin\nbackend=none\n"
        );
        assert_eq!(
            request("reload\n", &reload, &stop, &pause, &status),
            "reload scheduled\n"
        );
        assert!(reload.load(Ordering::Acquire));
        assert_eq!(
            request("pause\n", &reload, &stop, &pause, &status),
            "paused\n"
        );
        assert!(pause.load(Ordering::Acquire));
        assert_eq!(
            request("resume\n", &reload, &stop, &pause, &status),
            "resumed\n"
        );
        assert!(!pause.load(Ordering::Acquire));
        assert_eq!(
            request("stop\n", &reload, &stop, &pause, &status),
            "stopping\n"
        );
        assert!(stop.load(Ordering::Acquire));
    }

    #[test]
    fn unknown_command_is_nonfatal() {
        let reload = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new("starting".into()));
        assert_eq!(
            request("bogus\n", &reload, &stop, &pause, &status),
            "unknown command; expected status, reload, pause, resume, or stop\n"
        );
        assert!(!reload.load(Ordering::Acquire));
        assert!(!stop.load(Ordering::Acquire));
    }

    #[test]
    fn oversized_command_is_bounded_and_nonfatal() {
        let reload = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new("starting".into()));
        let command = format!("{}\n", "x".repeat(MAX_COMMAND_BYTES + 1024));
        assert_eq!(
            request(&command, &reload, &stop, &pause, &status),
            "unknown command; expected status, reload, pause, resume, or stop\n"
        );
        assert!(!reload.load(Ordering::Acquire));
        assert!(!stop.load(Ordering::Acquire));
    }

    #[test]
    fn stale_socket_policy_requires_socket_and_matching_owner() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-control-test-{}", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        let uid = rustix::process::geteuid().as_raw();
        assert!(is_owned_socket(&metadata, uid));
        let identity = (metadata.dev(), metadata.ino());
        assert!(is_original_socket(&metadata, identity, uid));
        assert!(!is_original_socket(&metadata, (0, 0), uid));
        assert!(!is_owned_socket(&metadata, uid.saturating_add(1)));
        drop(listener);
        fs::remove_file(path).unwrap();

        let regular =
            std::env::temp_dir().join(format!("wayexpand-control-regular-{}", std::process::id()));
        fs::write(&regular, b"not a socket").unwrap();
        let metadata = fs::metadata(&regular).unwrap();
        assert!(!is_owned_socket(&metadata, uid));
        fs::remove_file(regular).unwrap();
    }

    #[test]
    fn socket_parent_must_be_private_and_user_owned() {
        let parent =
            std::env::temp_dir().join(format!("wayexpand-control-parent-{}", std::process::id()));
        fs::create_dir(&parent).unwrap();
        let socket = parent.join("wayexpand.sock");
        assert!(validate_socket_parent(&socket).is_ok());

        fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_socket_parent(&socket).is_err());
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn socket_ancestors_must_be_trusted() {
        if rustix::process::geteuid().as_raw() != 0 {
            return;
        }
        let root =
            std::env::temp_dir().join(format!("wayexpand-control-ancestor-{}", std::process::id()));
        let untrusted = root.join("untrusted");
        let inner = untrusted.join("inner");
        fs::create_dir_all(&inner).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&untrusted, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o700)).unwrap();
        rustix::fs::chown(&untrusted, Some(rustix::fs::Uid::from_raw(65_534)), None).unwrap();

        assert!(validate_socket_parent(&inner.join("wayexpand.sock")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn socket_parent_symlink_is_resolved_before_binding() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-control-symlink-{}", std::process::id()));
        let target = root.join("target");
        let link = root.join("link");
        fs::create_dir_all(&target).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert_eq!(
            secure_socket_path(&link.join("wayexpand.sock")).unwrap(),
            target.join("wayexpand.sock")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
