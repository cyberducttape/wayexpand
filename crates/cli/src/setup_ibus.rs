//! `wayexpand setup --backend ibus [--terminals]`: configure the IBus route
//! end to end and verify every link, so users never wire IBus by hand.
//!
//! Each step is idempotent and checks the live result rather than trusting a
//! command's exit status (`ibus engine` exits 1 after a successful switch to
//! an engine whose layout is `default`).

use crate::services::{ServiceInventory, UnitState, UserService};
use crate::setup::{enable_user_service, evdev_consent_marker, run_setup_command};
use crate::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use wayexpand_backend_ibus::chain::{
    self, kwin_input_method, kwin_input_method_available, IbusCheckState, IbusDaemonProcess,
    IbusDesktop, KWIN_IBUS_INPUT_METHOD,
};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const COMPONENT_TEMPLATE: &str = include_str!("../../../desktop/wayexpand-ibus.xml");
const COMMAND_OUTPUT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_COMMAND_OUTPUT: u64 = 64 * 1024;
const MAX_COMPONENT_BYTES: u64 = 64 * 1024;
const MAX_SHARED_ENGINE_BYTES: u64 = 512 * 1024 * 1024;
const TERMINAL_DROP_IN: &str = "30-terminal-apps-only.conf";
const LEGACY_PLASMA_IBUS_ENVIRONMENT_HEADER: &str =
    "# Written by `wayexpand setup --backend ibus`.";
/// Common terminal app IDs (normalized), used when `terminal_apps` is empty.
pub(crate) const DEFAULT_TERMINAL_APPS: &[&str] = &[
    "org.kde.konsole",
    "org.kde.yakuake",
    "org.gnome.console",
    "org.gnome.terminal",
    "org.gnome.ptyxis",
    "com.mitchellh.ghostty",
    "kitty",
    "alacritty",
    "foot",
    "footclient",
    "org.wezfurlong.wezterm",
    "xterm",
];

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct IbusSetupOptions {
    pub(crate) assume_yes: bool,
    /// Explicit user acceptance of IBus's verified but non-atomic replacement.
    /// This is intentionally separate from `assume_yes`.
    pub(crate) accept_non_atomic_replacement: bool,
    /// Also cover terminals with a terminal-only evdev daemon.
    pub(crate) terminals: bool,
    /// The core IBus route passed the interactive target-app check in this
    /// setup invocation; terminal activation is only allowed after this.
    pub(crate) real_insertion_verified: bool,
    /// Explicit authorization to attempt the one-time privileged component
    /// registration. `--yes` does not imply this authorization.
    pub(crate) install_system_integration: bool,
}

struct PrivateStagingFile(Option<PathBuf>);

impl PrivateStagingFile {
    fn path(&self) -> &Path {
        self.0
            .as_deref()
            .expect("staging path is present until cleanup")
    }

    fn cleanup(&mut self) -> Result<()> {
        let Some(path) = self.0.as_deref() else {
            return Ok(());
        };
        match fs::remove_file(path) {
            Ok(()) => {
                self.0 = None;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.0 = None;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for PrivateStagingFile {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// Run `program` with a deadline and return its stdout when it succeeds.
pub(crate) fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let (status, output) = command_output_with_status(program, arguments)?;
    status.success().then_some(output)
}

/// Whether setup has the client and engine binaries needed to provision an
/// IBus integration. This is intentionally separate from live IBus readiness:
/// setup can register a missing component and refresh an existing session.
pub(crate) fn setup_route_available() -> bool {
    executable_on_path("ibus") && engine_binary().is_some()
}

/// Return bounded stdout and the exit status. Some IBus commands print the
/// active engine successfully but exit nonzero for default-layout engines.
pub(crate) fn command_output_with_status(
    program: &str,
    arguments: &[&str],
) -> Option<(std::process::ExitStatus, String)> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_process_group(&mut command);
    let child = command.spawn().ok()?;
    let mut supervisor = ChildSupervisor::new(child);
    let mut stdout = supervisor.take_stdout()?;
    let reader = thread::spawn(move || {
        let mut output = String::new();
        let _ = (&mut stdout)
            .take(MAX_COMMAND_OUTPUT)
            .read_to_string(&mut output);
        output
    });
    let deadline = Instant::now() + COMMAND_OUTPUT_TIMEOUT;
    loop {
        match supervisor.has_exited() {
            Ok(true) => break,
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return None;
            }
        }
    }
    supervisor.kill_group();
    let status = supervisor.reap().ok()?;
    let output = reader.join().ok()?;
    Some((status, output))
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn step(message: &str) {
    println!("• {message}");
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The system component must never bind to one user's home directory.
pub(crate) fn component_manifest() -> String {
    let engine_path = shared_engine_path();
    COMPONENT_TEMPLATE.replace(
        "<exec>wayexpand-ibus</exec>",
        &format!(
            "<exec>{}</exec>",
            xml_escape(&engine_path.display().to_string())
        ),
    )
}

fn shared_engine_path() -> PathBuf {
    let contract: toml::Value = toml::from_str(include_str!("../../../installer-components.toml"))
        .expect("checked-in installer component contract must be valid TOML");
    PathBuf::from(
        contract["components"]["ibus"]["user_install_exec"]
            .as_str()
            .expect("IBus user_install_exec must be a string"),
    )
}

fn package_engine_path() -> PathBuf {
    let contract: toml::Value = toml::from_str(include_str!("../../../installer-components.toml"))
        .expect("checked-in installer component contract must be valid TOML");
    PathBuf::from(
        contract["components"]["ibus"]["package_install_exec_path"]
            .as_str()
            .expect("IBus package_install_exec_path must be a string"),
    )
}

fn package_engine_is_trusted() -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(package_engine_path()).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && metadata.uid() == 0
            && metadata.permissions().mode() & 0o111 != 0
    })
}

fn manifest_exec(contents: &[u8]) -> Option<&str> {
    let contents = std::str::from_utf8(contents).ok()?;
    let start = contents.find("<exec>")? + "<exec>".len();
    let end = contents[start..].find("</exec>")? + start;
    Some(contents[start..end].trim())
}

fn is_package_manifest_exec(exec: &str) -> bool {
    let contract: toml::Value = toml::from_str(include_str!("../../../installer-components.toml"))
        .expect("checked-in installer component contract must be valid TOML");
    exec == contract["components"]["ibus"]["package_install_exec"]
        .as_str()
        .expect("IBus package_install_exec must be a string")
        || exec
            == contract["components"]["ibus"]["package_install_exec_path"]
                .as_str()
                .expect("IBus package_install_exec_path must be a string")
}

fn preserve_package_registration(
    component_ok: bool,
    manifest_ok: bool,
    package_trusted: bool,
    exec: Option<&str>,
) -> bool {
    exec.is_some_and(is_package_manifest_exec) && component_ok && manifest_ok && package_trusted
}

fn preserve_shared_registration(
    component_ok: bool,
    manifest_ok: bool,
    shared_trusted: bool,
    exec: Option<&str>,
) -> bool {
    exec == shared_engine_path().to_str() && component_ok && manifest_ok && shared_trusted
}

fn shared_engine_is_trusted() -> bool {
    let path = shared_engine_path();
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return false;
    };
    let Some(parent) = path.parent() else {
        return false;
    };
    let ancestors_are_trusted = std::iter::successors(Some(parent), |directory| directory.parent())
        .all(|directory| {
            fs::symlink_metadata(directory).is_ok_and(|metadata| {
                metadata.file_type().is_dir()
                    && metadata.uid() == 0
                    && metadata.permissions().mode() & 0o022 == 0
            })
        });
    metadata.file_type().is_file()
        && metadata.uid() == 0
        && metadata.permissions().mode() & 0o111 != 0
        && ancestors_are_trusted
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComponentFileState {
    bytes: Vec<u8>,
    mode: u32,
    uid: u32,
    gid: u32,
    sha256: String,
}

impl ComponentFileState {
    fn read(path: &Path) -> Result<Option<Self>> {
        Self::read_limited(path, MAX_COMPONENT_BYTES)
    }

    fn read_limited(path: &Path, limit: u64) -> Result<Option<Self>> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_file() || metadata.len() > limit {
            bail!(
                "IBus component {} is not a regular file within the {}-byte transaction limit",
                path.display(),
                limit
            );
        }
        let bytes = fs::read(path)?;
        if bytes.len() as u64 > limit {
            bail!(
                "IBus component {} exceeds the transaction size limit",
                path.display()
            );
        }
        Ok(Some(Self {
            sha256: sha256_hex(&bytes),
            bytes,
            mode: metadata.permissions().mode() & 0o7777,
            uid: metadata.uid(),
            gid: metadata.gid(),
        }))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}

fn run_component_admin_command(
    privileged: bool,
    program: &str,
    arguments: &[&str],
) -> Result<std::process::ExitStatus> {
    if privileged {
        let mut elevated_arguments = Vec::with_capacity(arguments.len() + 1);
        elevated_arguments.push(program);
        elevated_arguments.extend_from_slice(arguments);
        run_setup_command("pkexec", &elevated_arguments)
    } else {
        run_setup_command(program, arguments)
    }
}

/// Check the closest existing parent, so an explicitly configured writable
/// `IBUS_COMPONENT_PATH` can be used without a privilege prompt. The ordinary
/// XDG data directory is not considered unless the running daemon selected it.
fn component_path_writable(target: &Path) -> bool {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let Some(mut parent) = target.parent() else {
        return false;
    };
    while !parent.exists() {
        let Some(ancestor) = parent.parent() else {
            return false;
        };
        parent = ancestor;
    }
    let Ok(path) = CString::new(parent.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: path is a valid NUL-terminated path; access retains no pointer.
    unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

fn current_uid_gid() -> (u32, u32) {
    // SAFETY: geteuid/getegid have no preconditions and only read process state.
    unsafe { (libc::geteuid(), libc::getegid()) }
}

/// Journal the system-wide component file separately from the running-session
/// snapshot so setup can restore a replacement or remove a newly installed
/// manifest if later activation fails.
#[derive(Debug)]
struct ComponentInstallJournal {
    target: PathBuf,
    original: Option<ComponentFileState>,
    installed: Option<ComponentFileState>,
    install_attempted: bool,
    install_used_privilege: bool,
    installed_state_known: bool,
    missing_directories: Vec<PathBuf>,
    max_bytes: u64,
}

impl ComponentInstallJournal {
    fn capture_at(target: PathBuf) -> Result<Self> {
        Self::capture_limited(target, MAX_COMPONENT_BYTES)
    }

    fn capture_limited(target: PathBuf, max_bytes: u64) -> Result<Self> {
        let original = ComponentFileState::read_limited(&target, max_bytes)
            .with_context(|| format!("could not snapshot transaction file {}", target.display()))?;
        let mut missing_directories = Vec::new();
        let mut parent = target.parent();
        while let Some(directory) = parent {
            if directory.exists() {
                break;
            }
            missing_directories.push(directory.to_path_buf());
            parent = directory.parent();
        }
        Ok(Self {
            target,
            original: original.clone(),
            installed: original,
            install_attempted: false,
            install_used_privilege: false,
            installed_state_known: true,
            missing_directories,
            max_bytes,
        })
    }

    fn capture() -> Result<Self> {
        Self::capture_at(chain::component_install_directory().join("wayexpand.xml"))
    }

    fn observe_after_install(&mut self) -> Result<()> {
        self.installed_state_known = false;
        self.installed = ComponentFileState::read_limited(&self.target, self.max_bytes)?;
        self.installed_state_known = true;
        Ok(())
    }

    fn restore(&self) -> Result<bool> {
        if !self.install_attempted {
            return Ok(false);
        }
        let current = ComponentFileState::read_limited(&self.target, self.max_bytes)?;
        let file_changed = self.installed != self.original || current != self.original;
        if file_changed {
            if !self.installed_state_known || current != self.installed {
                bail!(
                    "IBus component {} changed after setup; leaving the newer file untouched",
                    self.target.display()
                );
            }
            if let Some(original) = &self.original {
                let mut staged =
                    write_private_staging_file("ibus-component-restore", &original.bytes)?;
                let mode = format!("{:o}", original.mode);
                let uid = original.uid.to_string();
                let gid = original.gid.to_string();
                let target = self.target.display().to_string();
                let staged_arg = staged.path().display().to_string();
                let result = run_component_admin_command(
                    self.install_used_privilege,
                    "/usr/bin/install",
                    &[
                        "-D",
                        "-o",
                        &uid,
                        "-g",
                        &gid,
                        "-m",
                        &mode,
                        &staged_arg,
                        &target,
                    ],
                );
                let cleanup = staged.cleanup();
                let status = result.context(
                    "could not restore the original IBus component through the administrator prompt",
                )?;
                cleanup.context("could not remove the private staged IBus component backup")?;
                if !status.success() {
                    bail!("administrator command did not restore the original IBus component (exit {status})");
                }
                if ComponentFileState::read_limited(&self.target, self.max_bytes)?.as_ref()
                    != Some(original)
                {
                    bail!("restored IBus component did not match its recorded SHA-256 and mode");
                }
            } else {
                let target = self.target.display().to_string();
                let status = run_component_admin_command(
                    self.install_used_privilege,
                    "/usr/bin/rm",
                    &["-f", "--", &target],
                )
                .context("could not remove the IBus component installed by this setup")?;
                if !status.success() {
                    bail!(
                        "administrator command did not remove the IBus component (exit {status})"
                    );
                }
                if ComponentFileState::read_limited(&self.target, self.max_bytes)?.is_some() {
                    bail!("the IBus component remains after the removal command");
                }
            }
        }
        let mut directories_removed = false;
        for directory in &self.missing_directories {
            if !directory.exists() {
                continue;
            }
            let path = directory.display().to_string();
            let status = run_component_admin_command(
                self.install_used_privilege,
                "/usr/bin/rmdir",
                &["--", &path],
            )
            .context("could not remove an empty system directory created during setup")?;
            if !status.success() {
                bail!("setup-created system directory {} is not empty or could not be removed; leaving it intact", directory.display());
            }
            directories_removed = true;
        }
        Ok(file_changed || directories_removed)
    }
}

#[derive(Debug)]
struct PreparedFileJournal {
    target: PathBuf,
    original: Option<ComponentFileState>,
    written: Option<ComponentFileState>,
    attempted: bool,
    written_state_known: bool,
    created_directories: Vec<PathBuf>,
}

impl PreparedFileJournal {
    fn capture(target: PathBuf) -> Result<Self> {
        let original = ComponentFileState::read(&target)
            .with_context(|| format!("could not snapshot prepared file {}", target.display()))?;
        Ok(Self {
            target,
            original: original.clone(),
            written: original,
            attempted: false,
            written_state_known: true,
            created_directories: Vec::new(),
        })
    }

    fn write(&mut self, contents: &[u8]) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let parent = self
            .target
            .parent()
            .ok_or_else(|| anyhow::anyhow!("prepared file has no parent directory"))?;
        let mut missing = Vec::new();
        let mut current = parent;
        while !current.exists() {
            missing.push(current.to_path_buf());
            current = current
                .parent()
                .ok_or_else(|| anyhow::anyhow!("prepared file has no existing ancestor"))?;
        }
        self.attempted = true;
        for directory in missing.iter().rev() {
            match fs::create_dir(directory) {
                Ok(()) => self.created_directories.push(directory.clone()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !fs::symlink_metadata(directory)?.file_type().is_dir() {
                        bail!(
                            "prepared path ancestor {} is not a directory",
                            directory.display()
                        );
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        let staged = parent.join(format!(
            ".wayexpand-setup-{}-{}.tmp",
            std::process::id(),
            self.target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("dropin")
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&staged)?;
        if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
            let _ = fs::remove_file(&staged);
            self.written = ComponentFileState::read(&self.target)?;
            return Err(error.into());
        }
        self.written_state_known = false;
        if let Err(error) = fs::rename(&staged, &self.target) {
            let _ = fs::remove_file(&staged);
            self.written = ComponentFileState::read(&self.target)?;
            self.written_state_known = true;
            return Err(error.into());
        }
        self.written = ComponentFileState::read(&self.target)?;
        self.written_state_known = true;
        Ok(())
    }

    fn restore(&self) -> Result<bool> {
        if !self.attempted {
            return Ok(false);
        }
        if !self.written_state_known {
            bail!(
                "could not establish the installed state of prepared file {}; leaving it untouched",
                self.target.display()
            );
        }
        let mut restored = false;
        if self.written != self.original && ComponentFileState::read(&self.target)? != self.written
        {
            bail!(
                "prepared file {} changed after setup; leaving the newer file untouched",
                self.target.display()
            );
        }
        if self.written != self.original {
            if let Some(original) = &self.original {
                let parent = self.target.parent().expect("captured target has parent");
                let staged = parent.join(format!(
                    ".wayexpand-rollback-{}-{}.tmp",
                    std::process::id(),
                    self.target
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("dropin")
                ));
                use std::os::unix::fs::OpenOptionsExt;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(original.mode)
                    .open(&staged)?;
                fs::set_permissions(&staged, fs::Permissions::from_mode(original.mode))?;
                if let Err(error) = file
                    .write_all(&original.bytes)
                    .and_then(|()| file.sync_all())
                {
                    let _ = fs::remove_file(&staged);
                    return Err(error.into());
                }
                if let Err(error) = fs::rename(&staged, &self.target) {
                    let _ = fs::remove_file(&staged);
                    return Err(error.into());
                }
                restore_file_owner(&self.target, original.uid, original.gid)?;
                if ComponentFileState::read(&self.target)?.as_ref() != Some(original) {
                    bail!("prepared file restoration did not match its original hash and mode");
                }
            } else {
                fs::remove_file(&self.target)?;
            }
            restored = true;
        }
        for directory in self.created_directories.iter().rev() {
            match fs::remove_dir(directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                    bail!(
                        "setup-created directory {} is no longer empty; leaving it intact",
                        directory.display()
                    );
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(restored || !self.created_directories.is_empty())
    }
}

fn restore_file_owner(path: &Path, uid: u32, gid: u32) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let current = fs::symlink_metadata(path)?;
    if current.uid() == uid && current.gid() == gid {
        return Ok(());
    }
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| anyhow::anyhow!("file path contains a NUL byte"))?;
    // SAFETY: `path` is a valid NUL-terminated path and chown retains no pointers.
    if unsafe { libc::chown(path.as_ptr(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn write_private_staging_file(prefix: &str, bytes: &[u8]) -> Result<PrivateStagingFile> {
    use std::os::unix::fs::OpenOptionsExt;
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| anyhow::anyhow!("XDG_RUNTIME_DIR is not set"))?;
    let path = runtime.join(format!("wayexpand-{prefix}-{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("could not create private staging file {}", path.display()))?;
    if let Err(error) = file.write_all(bytes) {
        let _ = fs::remove_file(&path);
        return Err(error.into());
    }
    Ok(PrivateStagingFile(Some(path)))
}

fn engine_binary() -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join("wayexpand-ibus"))
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleEngineProcess {
    pub(crate) pid: i32,
    pub(crate) executable: PathBuf,
}

/// Find this user's WayExpand IBus engine processes that still map an older
/// engine executable. IBus owns these processes, so restarting a WayExpand
/// systemd unit cannot refresh them.
pub(crate) fn stale_engine_processes() -> Vec<StaleEngineProcess> {
    let Some(installed) = engine_binary().and_then(|path| fs::canonicalize(path).ok()) else {
        return Vec::new();
    };
    stale_engine_processes_at(Path::new("/proc"), unsafe { libc::geteuid() }, &installed)
}

fn stale_engine_processes_at(
    proc_root: &Path,
    uid: u32,
    installed: &Path,
) -> Vec<StaleEngineProcess> {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut processes: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<i32>().ok()?;
            let proc_dir = entry.path();
            let owned = fs::metadata(&proc_dir)
                .is_ok_and(|metadata| std::os::unix::fs::MetadataExt::uid(&metadata) == uid);
            if !owned {
                return None;
            }
            let executable = fs::read_link(proc_dir.join("exe")).ok()?;
            // `comm` is useful, but can differ from argv[0] when an IBus
            // launcher or wrapper starts the engine. Check all three kernel
            // views so the upgrade report still identifies the process that
            // IBus must restart.
            let comm_matches = fs::read_to_string(proc_dir.join("comm"))
                .is_ok_and(|comm| comm.trim() == "wayexpand-ibus");
            let exe_matches = executable
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "wayexpand-ibus" || name == "wayexpand-ibus (deleted)");
            let argv_matches = fs::read(proc_dir.join("cmdline"))
                .ok()
                .and_then(|cmdline| {
                    cmdline
                        .split(|byte| *byte == 0)
                        .find(|argument| !argument.is_empty())
                        .map(|argument| {
                            argument
                                .rsplit(|byte| *byte == b'/')
                                .next()
                                .is_some_and(|name| name == b"wayexpand-ibus")
                        })
                })
                .unwrap_or(false);
            if !(comm_matches || exe_matches || argv_matches) {
                return None;
            }
            (executable != installed).then_some(StaleEngineProcess { pid, executable })
        })
        .collect();
    processes.sort_by_key(|process| process.pid);
    processes
}

/// Install the component where the running ibus-daemon reads components.
/// Writing there needs administrator rights, so this asks through pkexec.
fn install_component(
    journal: &mut ComponentInstallJournal,
    engine_journal: &mut ComponentInstallJournal,
    options: IbusSetupOptions,
) -> Result<()> {
    let report = chain::diagnose();
    let component_ok = report
        .check("component")
        .is_some_and(|check| check.state == IbusCheckState::Pass);
    let manifest_ok = report
        .check("manifest")
        .is_some_and(|check| check.state == IbusCheckState::Pass);
    let original_exec = journal
        .original
        .as_ref()
        .and_then(|state| manifest_exec(&state.bytes))
        .map(str::to_owned);
    if original_exec
        .as_deref()
        .is_some_and(is_package_manifest_exec)
    {
        // Distribution packages own this registration. A per-user source
        // install must never replace the package's shared executable mapping.
        if preserve_package_registration(
            component_ok,
            manifest_ok,
            package_engine_is_trusted(),
            original_exec.as_deref(),
        ) {
            step("The distribution-owned IBus component and engine are already valid; preserving them");
            return Ok(());
        }
        bail!("WX-IBUS-PACKAGE-001: the distribution-owned IBus component is present but its engine is unavailable or untrusted. Repair or reinstall WayExpand with the package manager; a per-user source install will not replace the shared package registration.");
    }
    if preserve_shared_registration(
        component_ok,
        manifest_ok,
        shared_engine_is_trusted(),
        original_exec.as_deref(),
    ) {
        step("The shared IBus component and engine are already valid; preserving the system-wide registration");
        return Ok(());
    }
    if component_ok && manifest_ok && journal.original.is_none() {
        step("The IBus component is installed where IBus reads it");
        return Ok(());
    }
    let engine = engine_binary().ok_or_else(|| {
        anyhow::anyhow!("`wayexpand-ibus` is not on PATH; reinstall WayExpand (user installs need ~/.local/bin on PATH)")
    })?;
    let (uid, gid) = current_uid_gid();
    let original_has_other_owner = journal
        .original
        .as_ref()
        .is_some_and(|original| original.uid != uid || original.gid != gid);
    let engine_has_other_owner = engine_journal
        .original
        .as_ref()
        .is_some_and(|original| original.uid != uid || original.gid != gid);
    let privileged = !component_path_writable(&journal.target)
        || !component_path_writable(&engine_journal.target)
        || original_has_other_owner
        || engine_has_other_owner;
    if privileged && !executable_on_path("pkexec") {
        bail!("WX-IBUS-AUTH-001: no `pkexec` desktop authorization helper is available. WayExpand cannot register the system IBus component nonprivileged because the running IBus daemon does not read the ordinary per-user component directory. Install WayExpand with your distribution package manager, restore `pkexec` and its desktop authorization agent, or choose another supported input route. You can safely retry with `wayexpand setup --backend ibus --install-system-integration` after that is resolved.");
    }
    let system_install_consent = if !privileged {
        step(&format!(
            "The configured IBus component directory {} is writable by this user; registering without administrator privileges",
            journal.target.parent().unwrap_or(&journal.target).display()
        ));
        true
    } else if options.install_system_integration {
        true
    } else if options.assume_yes {
        bail!("WX-IBUS-INTEGRATION-CONSENT: the IBus manifest is not registered. `--yes` does not authorize a system-wide integration change. Review the plan, then rerun `wayexpand setup --backend ibus --install-system-integration`, or install WayExpand through your distribution package manager.");
    } else {
        println!("WayExpand IBus installation plan:");
        println!("  - Application files remain in your user installation.");
        println!(
            "  - IBus must read a component manifest from {}.",
            journal.target.display()
        );
        println!("  - The shared engine executable will be installed at {}; the manifest will not refer to your home directory.", shared_engine_path().display());
        println!("  - Registering it requires a one-time administrator authorization.");
        if original_has_other_owner {
            println!("  - The existing manifest is owned by another account; authorization also preserves its original ownership during rollback.");
        }
        println!("  - No keyboard permissions or replacement consent are granted by this step.");
        println!("Alternative: install the distribution package (which owns the manifest) or choose another supported input route.");
        prompt_yes_no(
            "Install the IBus integration now using the desktop authorization prompt? [y/N] ",
            false,
        )?
    };
    if !system_install_consent {
        bail!("IBus integration was not installed. No session activation was attempted; rerun `wayexpand setup --backend ibus --install-system-integration` when ready, or use a distribution package/another input route.");
    }
    journal.install_used_privilege = privileged;
    engine_journal.install_used_privilege = privileged;
    let manifest = component_manifest();
    let engine_metadata = fs::metadata(&engine)?;
    if !engine_metadata.is_file()
        || engine_metadata.len() > MAX_SHARED_ENGINE_BYTES
        || engine_metadata.permissions().mode() & 0o111 == 0
    {
        bail!("the current wayexpand-ibus executable is not a regular executable within the shared-engine size limit");
    }
    let engine_contents = fs::read(&engine)?;
    let mut staged_engine = write_private_staging_file("ibus-engine", &engine_contents)?;
    let engine_target_arg = engine_journal.target.display().to_string();
    let staged_engine_arg = staged_engine.path().display().to_string();
    engine_journal.install_attempted = true;
    let engine_result = run_component_admin_command(
        privileged,
        "/usr/bin/install",
        &["-D", "-m", "0755", &staged_engine_arg, &engine_target_arg],
    );
    let engine_observation = engine_journal.observe_after_install();
    let engine_cleanup = staged_engine.cleanup();
    let engine_status =
        engine_result.context("could not install the shared IBus engine executable")?;
    engine_cleanup.context("could not remove the staged shared IBus engine")?;
    engine_observation.context("could not verify the shared IBus engine")?;
    if !engine_status.success() {
        bail!(
            "could not install shared IBus engine at {}; setup will roll back",
            engine_journal.target.display()
        );
    }
    let source_engine_hash = sha256_hex(&engine_contents);
    if engine_journal.installed.as_ref().is_none_or(|state| {
        state.sha256 != source_engine_hash || state.mode & 0o111 == 0 || state.uid != 0
    }) {
        bail!(
            "the shared IBus engine did not match the installed user binary or is not root-owned"
        );
    }
    let mut staged = write_private_staging_file("ibus-component", manifest.as_bytes())?;
    step(&format!(
        "Installing the IBus component to {} (IBus never reads per-user component folders; you will be asked for an administrator password)",
        journal.target.display()
    ));
    let staged_arg = staged.path().display().to_string();
    let target_arg = journal.target.display().to_string();
    journal.install_attempted = true;
    let result = run_component_admin_command(
        privileged,
        "/usr/bin/install",
        &["-D", "-m", "0644", &staged_arg, &target_arg],
    );
    let observation = journal.observe_after_install();
    let cleanup = staged.cleanup();
    let status = result.context(if privileged { "could not start IBus component registration through the desktop authorization helper. Check that a polkit authentication agent is running, then retry `wayexpand setup --backend ibus --install-system-integration`; alternatively install the distribution package or select another input route" } else { "could not register the IBus component in the configured user-writable component directory" })?;
    cleanup.context("could not remove the private staged IBus component manifest")?;
    observation.context("could not verify the IBus component after installation")?;
    if !status.success() {
        bail!("WX-IBUS-AUTH-002: IBus component registration did not complete (authorization may have been denied, or {} may be read-only). Setup will roll back any partial file change. Retry with `wayexpand setup --backend ibus --install-system-integration` after resolving authorization, or install the distribution package/use another input route", journal.target.display());
    }
    let installed = journal.installed.as_ref().ok_or_else(|| {
        anyhow::anyhow!("the administrator command succeeded but the IBus component is missing")
    })?;
    if installed.sha256 != sha256_hex(manifest.as_bytes()) || installed.mode & 0o777 != 0o644 {
        bail!("the installed IBus component did not match the requested manifest hash and mode");
    }
    Ok(())
}

fn executable_on_path(program: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|directory| {
            let candidate = directory.join(program);
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
    })
}

/// On KDE Plasma Wayland, make KWin launch IBus as its input method; only
/// then do Wayland apps send keys to IBus.
fn route_kde_to_ibus(snapshot: &mut SessionSnapshot) -> Result<()> {
    let configured =
        kwin_input_method().is_some_and(|value| value.to_ascii_lowercase().contains("ibus"));
    if configured && kwin_input_method_available() == Some(true) {
        step("KWin already sends input to IBus");
        return Ok(());
    }
    if !Path::new(KWIN_IBUS_INPUT_METHOD).exists() {
        bail!(
            "{KWIN_IBUS_INPUT_METHOD} is missing; install the IBus GTK 3 panel (for example the `ibus-gtk3` package) and rerun setup"
        );
    }
    // A hand-started ibus-daemon has no Wayland input-method connection and
    // would keep KWin's own instance from starting.
    if let Some(daemon) = IbusDaemonProcess::find() {
        if !chain::started_by(daemon.pid, "kwin_wayland") {
            step("Stopping an ibus-daemon that was not started by KWin");
            let _ = run_setup_command("ibus", &["exit"]);
            wait_until(Duration::from_secs(5), || {
                IbusDaemonProcess::find().is_none()
            });
        }
    }
    step("Setting KWin's Virtual Keyboard to IBus Wayland");
    // KWin applies the setting from a change notification, and
    // kwriteconfig6 sends none when the stored value is unchanged.
    let kwriteconfig = kwriteconfig().ok_or_else(|| anyhow::anyhow!("kwriteconfig6 is not installed; set System Settings → Keyboard → Virtual Keyboard → \"IBus Wayland\" yourself, then rerun setup"))?;
    snapshot.kwin_input_method = Some(kwin_input_method());
    let base = KWIN_INPUT_METHOD_KEY;
    let mut delete = base.to_vec();
    delete.push("--delete");
    let _ = run_setup_command(kwriteconfig, &delete);
    let mut write = base.to_vec();
    write.push(KWIN_IBUS_INPUT_METHOD);
    if !run_setup_command(kwriteconfig, &write).is_ok_and(|status| status.success()) {
        bail!("could not update kwinrc; set System Settings → Keyboard → Virtual Keyboard → \"IBus Wayland\" yourself");
    }
    if !wait_until(Duration::from_secs(15), || {
        kwin_input_method_available() == Some(true) && IbusDaemonProcess::find().is_some()
    }) {
        bail!("KWin did not start IBus; reselect \"IBus Wayland\" in System Settings → Keyboard → Virtual Keyboard, or log out and back in, then rerun setup");
    }
    Ok(())
}

/// After an upgrade IBus keeps running the old engine binary. Stop engine
/// processes that do not run the installed binary, so selecting the engine
/// starts the current one.
fn restart_stale_engine() {
    for process in stale_engine_processes() {
        step("Stopping an IBus engine from an older installation");
        unsafe {
            libc::kill(process.pid, libc::SIGTERM);
        }
    }
}

/// Parse `gsettings get` output for a string list such as `['a', 'b']` or
/// `@as []`.
pub(crate) fn parse_string_list(output: &str) -> Option<Vec<String>> {
    let trimmed = output.trim().trim_start_matches("@as").trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    inner
        .split(',')
        .map(|item| {
            let item = item.trim();
            item.strip_prefix('\'')
                .and_then(|item| item.strip_suffix('\''))
                .map(str::to_owned)
        })
        .collect()
}

pub(crate) fn format_string_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("'{item}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Add WayExpand to IBus' input methods only when the transaction captured
/// the original setting; verify the update so rollback has a known baseline.
fn add_to_preload_engines(
    original: Option<&str>,
    changed_value: &mut Option<String>,
) -> Result<()> {
    if original.is_none() {
        step("Leaving the IBus preload list unchanged because its original value could not be captured");
        return Ok(());
    }
    let Some(output) = command_output(
        "gsettings",
        &["get", "org.freedesktop.ibus.general", "preload-engines"],
    ) else {
        bail!("could not reread IBus' input-method list; refusing an unjournaled change");
    };
    let Some(mut engines) = parse_string_list(&output) else {
        bail!("IBus returned an invalid preload-engine list; refusing to change it");
    };
    if engines.iter().any(|engine| engine == "wayexpand") {
        return Ok(());
    }
    if engines.is_empty() {
        // An empty list means "the default keyboard"; keep one so switching
        // away from WayExpand still leaves a plain keyboard layout.
        engines.push("xkb:us::eng".into());
    }
    engines.push("wayexpand".into());
    let intended_raw = format_string_list(&engines);
    // Mark the attempted value before invoking gsettings. A failed process or
    // lost reply can still have applied the setting, so rollback must know
    // what value it may safely undo.
    *changed_value = Some(intended_raw.clone());
    step("Adding WayExpand to IBus' input methods");
    let status = run_setup_command(
        "gsettings",
        &[
            "set",
            "org.freedesktop.ibus.general",
            "preload-engines",
            &intended_raw,
        ],
    )?;
    if !status.success() {
        bail!("gsettings failed to update IBus' preload-engine list");
    }
    let verified_raw = command_output(
        "gsettings",
        &["get", "org.freedesktop.ibus.general", "preload-engines"],
    )
    .ok_or_else(|| anyhow::anyhow!("could not verify the updated IBus preload-engine list"))?;
    let verified = parse_string_list(&verified_raw)
        .ok_or_else(|| anyhow::anyhow!("IBus returned an invalid updated preload-engine list"))?;
    if verified != engines {
        bail!("IBus did not retain the updated preload-engine list");
    }
    *changed_value = Some(verified_raw.trim().to_owned());
    Ok(())
}

fn service_active(service: &str) -> bool {
    run_setup_command("systemctl", &["--user", "is-active", "--quiet", service])
        .is_ok_and(|status| status.success())
}

fn companion_readiness_error(service: UserService, status: &serde_json::Value) -> Option<String> {
    let expected_source = match service {
        UserService::Evdev => "evdev",
        UserService::Control => "stdin",
        UserService::InputMethod => return Some("IBus is not a companion service".into()),
    };
    let field = |name: &str| status.get(name).and_then(serde_json::Value::as_str);
    if !crate::doctor::status::status_schema_compatible(status) {
        return Some("daemon status schema is incompatible".into());
    }
    if !crate::doctor::status::daemon_build_matches(field("daemon_commit"), field("daemon_version"))
    {
        return Some("daemon build identity does not match the setup CLI".into());
    }
    if field("source") != Some(expected_source) {
        return Some(format!(
            "expected source {expected_source}, found {:?}",
            field("source")
        ));
    }
    if field("backend") != Some("libei") {
        return Some(format!(
            "expected libei output, found {:?}",
            field("backend")
        ));
    }
    if field("config_state") != Some("ok")
        || status
            .get("config_generation")
            .and_then(serde_json::Value::as_u64)
            .is_none_or(|generation| generation == 0)
        || field("config_active_path") != Some(default_config_path().to_string_lossy().as_ref())
    {
        return Some("expected configuration is not loaded and healthy".into());
    }
    if field("output_connection_state") != Some("connected") || field("output_health") != Some("ok")
    {
        return Some("libei output is not operational".into());
    }
    match service {
        UserService::Evdev
            if field("state") != Some("connected")
                || status.get("route_available") != Some(&serde_json::Value::Bool(true))
                || status.get("matching_available") != Some(&serde_json::Value::Bool(true))
                || field("input_connection_state") != Some("connected") =>
        {
            Some("evdev capture or matching is not operational".into())
        }
        UserService::Control if field("state") != Some("running") => {
            Some("control service is not running".into())
        }
        _ => None,
    }
}

fn companion_status_error(service: UserService) -> Result<()> {
    let response = crate::doctor::status::read_daemon_status()
        .map_err(|error| anyhow::anyhow!("daemon control status is unavailable: {error}"))?;
    let status = crate::doctor::status::status_as_json(&response)?;
    if let Some(problem) = companion_readiness_error(service, &status) {
        bail!("{problem}");
    }
    Ok(())
}

fn terminal_drop_in() -> Option<PathBuf> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(
        config_home
            .join("systemd/user/wayexpand-evdev.service.d")
            .join(TERMINAL_DROP_IN),
    )
}

fn plasma_ibus_environment_path() -> Option<PathBuf> {
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    plasma_ibus_environment_path_from(xdg_config_home.as_deref(), home.as_deref())
}

fn plasma_ibus_environment_path_from(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let config_home = match xdg_config_home {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(_) => return None,
        None => home?.join(".config"),
    };
    config_home
        .is_absolute()
        .then(|| config_home.join("plasma-workspace/env/wayexpand-ibus.sh"))
}

fn is_legacy_plasma_ibus_environment(bytes: &[u8]) -> bool {
    let Ok(contents) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut lines = contents.lines();
    if lines
        .next()
        .is_none_or(|line| line.trim() != LEGACY_PLASMA_IBUS_ENVIRONMENT_HEADER)
    {
        return false;
    }
    let mut gtk = false;
    let mut qt = false;
    let mut xim = false;
    for line in lines.map(str::trim).filter(|line| !line.is_empty()) {
        match line {
            line if line.starts_with('#') => {}
            "export GTK_IM_MODULE=ibus" if !gtk => gtk = true,
            "export QT_IM_MODULE=ibus" if !qt => qt = true,
            "export XMODIFIERS=@im=ibus" if !xim => xim = true,
            _ => return false,
        }
    }
    gtk && qt && xim
}

/// Remove only the obsolete login script written by earlier WayExpand setup
/// versions. KWin's IBus Wayland integration is compositor-managed and must
/// not inherit GTK_IM_MODULE=ibus or QT_IM_MODULE=ibus.
fn remove_legacy_plasma_ibus_environment() -> Result<bool> {
    let Some(path) = plasma_ibus_environment_path() else {
        return Ok(false);
    };
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).context("could not inspect the old Plasma IBus environment file")
        }
    };
    if metadata.len() > 8 * 1024 || !is_legacy_plasma_ibus_environment(&fs::read(&path)?) {
        return Ok(false);
    }
    fs::remove_file(&path)
        .context("could not remove the obsolete WayExpand Plasma IBus environment file")?;

    let Some(manager_environment) = command_output("systemctl", &["--user", "show-environment"])
    else {
        return Ok(true);
    };
    let mut variables = Vec::new();
    for (name, expected) in [
        ("GTK_IM_MODULE", "ibus"),
        ("QT_IM_MODULE", "ibus"),
        ("XMODIFIERS", "@im=ibus"),
    ] {
        if manager_environment
            .lines()
            .any(|line| line == format!("{name}={expected}"))
        {
            variables.push(name);
        }
    }
    if variables.is_empty() {
        return Ok(true);
    }
    let mut arguments = vec!["--user", "unset-environment"];
    arguments.extend(variables);
    let status = run_setup_command("systemctl", &arguments)?;
    if !status.success() {
        bail!("removed the obsolete Plasma IBus environment file, but could not clear its values from the user service manager; log out of Plasma to reset the D-Bus session environment");
    }
    Ok(true)
}

#[derive(Debug, Default)]
struct ConfigMutationJournal {
    path: PathBuf,
    original_mode: wayexpand_core::CompatibilityMode,
    original_terminal_apps: Vec<String>,
    last_written_revision: Option<wayexpand_core::ConfigRevision>,
    write_attempted: bool,
    replacement_authorization_granted: bool,
}

impl ConfigMutationJournal {
    fn capture() -> Result<Self> {
        let path = default_config_path();
        let loaded =
            Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
        Ok(Self {
            path,
            original_mode: loaded.config.settings.compatibility_mode,
            original_terminal_apps: loaded.config.settings.terminal_apps,
            last_written_revision: None,
            write_attempted: false,
            replacement_authorization_granted: false,
        })
    }

    fn note_write(&mut self, revision: wayexpand_core::ConfigRevision) {
        self.last_written_revision = Some(revision);
    }

    fn rollback(&self) -> Result<Vec<String>> {
        if !self.write_attempted {
            return Ok(Vec::new());
        }
        let loaded = Config::load_versioned(&self.path)
            .map_err(|error| config_load_error(&self.path, error))?;
        let mode_changed = loaded.config.settings.compatibility_mode != self.original_mode;
        let terminals_changed = loaded.config.settings.terminal_apps != self.original_terminal_apps;
        if !mode_changed && !terminals_changed {
            return Ok(Vec::new());
        }
        if self
            .last_written_revision
            .as_ref()
            .is_none_or(|expected_revision| loaded.revision != *expected_revision)
        {
            let reason = if self.last_written_revision.is_some() {
                "configuration changed after setup"
            } else {
                "the config write outcome could not be bound to a safe revision"
            };
            let mut remaining = Vec::new();
            if mode_changed {
                remaining.push(format!(
                    "compatibility_mode remains {} because {reason}",
                    loaded.config.settings.compatibility_mode.as_str()
                ));
            }
            if terminals_changed {
                remaining.push(format!(
                    "settings.terminal_apps remains changed because {reason}"
                ));
            }
            bail!("{}", remaining.join("; "));
        }
        let mut config = loaded.config;
        config.settings.compatibility_mode = self.original_mode;
        config
            .settings
            .terminal_apps
            .clone_from(&self.original_terminal_apps);
        config.validate().map_err(|error| {
            config_error(format!(
                "configuration invalid during setup rollback: {}",
                error.safe_summary()
            ))
        })?;
        config
            .save_atomic_if_revision_matches(&self.path, &loaded.revision)
            .map_err(|error| {
                config_error(format!(
                    "could not restore setup configuration changes: {}",
                    error.safe_summary()
                ))
            })?;
        let mut restored = Vec::new();
        if mode_changed {
            restored.push(if self.replacement_authorization_granted {
                "explicit replacement authorization was rolled back with the failed setup".into()
            } else {
                "compatibility_mode was restored".into()
            });
        }
        if terminals_changed {
            restored.push("settings.terminal_apps was restored".into());
        }
        Ok(restored)
    }
}

/// Make sure `settings.terminal_apps` lists terminals; returns the list.
fn ensure_terminal_apps(journal: &mut ConfigMutationJournal) -> Result<Vec<String>> {
    let path = default_config_path();
    let loaded = Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    if !loaded.config.settings.terminal_apps.is_empty() {
        return Ok(loaded.config.settings.terminal_apps);
    }
    let mut config = loaded.config;
    config.settings.terminal_apps = DEFAULT_TERMINAL_APPS
        .iter()
        .map(|app| (*app).to_owned())
        .collect();
    config.validate().map_err(|error| {
        config_error(format!(
            "configuration invalid after edit: {}",
            error.safe_summary()
        ))
    })?;
    journal.write_attempted = true;
    let revision = config
        .save_atomic_if_revision_matches(&path, &loaded.revision)
        .map_err(|error| {
            config_error(format!(
                "could not save configuration: {}",
                error.safe_summary()
            ))
        })?;
    journal.note_write(revision);
    step(&format!(
        "Listed common terminals in settings.terminal_apps ({}); edit that list in {} to add yours",
        DEFAULT_TERMINAL_APPS.len(),
        path.display()
    ));
    Ok(config.settings.terminal_apps)
}

/// IBus replacement is verified but not atomic, which the default
/// `compatibility_mode = "off"` refuses. Ask for that consent explicitly;
/// `--yes` never grants it on the user's behalf.
fn replacement_authorization_required(mode: wayexpand_core::CompatibilityMode) -> bool {
    mode == wayexpand_core::CompatibilityMode::Off
}

fn ensure_replacement_consent(
    options: IbusSetupOptions,
    journal: &mut ConfigMutationJournal,
) -> Result<bool> {
    let path = default_config_path();
    let loaded = Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    if !replacement_authorization_required(loaded.config.settings.compatibility_mode) {
        return Ok(false);
    }
    println!("  IBus checks that the text before the cursor is your trigger, then deletes it and inserts");
    println!("  the snippet as two separate steps. compatibility_mode = \"off\" refuses that, so nothing would expand.");
    if !options.accept_non_atomic_replacement && options.assume_yes {
        println!("  Not changed by --yes. Enable it with compatibility_mode = \"tracked-focus\" under [settings] in {}", path.display());
        return Ok(true);
    }
    if !options.accept_non_atomic_replacement && !prompt_yes_no(
        "  Allow verified two-step replacement (compatibility_mode = \"tracked-focus\")? [Y/n] ",
        true,
    )? {
        println!(
            "  Left compatibility_mode off; IBus will recognize triggers but not replace them."
        );
        return Ok(true);
    }
    let mut config = loaded.config;
    config.settings.compatibility_mode = wayexpand_core::CompatibilityMode::TrackedFocus;
    config.validate().map_err(|error| {
        config_error(format!(
            "configuration invalid after edit: {}",
            error.safe_summary()
        ))
    })?;
    journal.write_attempted = true;
    let revision = config
        .save_atomic_if_revision_matches(&path, &loaded.revision)
        .map_err(|error| {
            config_error(format!(
                "could not save configuration: {}",
                error.safe_summary()
            ))
        })?;
    journal.note_write(revision);
    journal.replacement_authorization_granted = true;
    step("Set compatibility_mode = \"tracked-focus\"");
    Ok(false)
}

/// Runtime state changed during activation. Persistent files and user config
/// are tracked separately by `SetupTransaction`.
#[derive(Debug, Default)]
struct SessionSnapshot {
    /// Whether an IBus daemon existed before setup; a newly-started desktop
    /// daemon may remain after failure and must be reported if not stoppable.
    ibus_daemon_was_running: bool,
    /// `ibus engine` before setup, if IBus was running.
    global_engine: Option<String>,
    /// Raw `gsettings get` value of IBus' input-method list.
    preload_engines: Option<String>,
    /// Value installed by this setup, when it changed the list.
    preload_engines_after: Option<String>,
    /// KWin's previous `[Wayland] InputMethod`, recorded when setup changed it.
    kwin_input_method: Option<Option<String>>,
    /// GNOME's typed input-source list and active source, when applicable.
    gnome_input_sources: Option<crate::gnome_input_sources::Snapshot>,
    /// State before any component installation/restart, used if failure occurs
    /// before the later activation-time GNOME snapshot can be captured.
    gnome_input_sources_original: Option<crate::gnome_input_sources::Snapshot>,
    /// Original service state for every route that setup may affect.
    services: Vec<OriginalServiceState>,
}

#[derive(Debug, Default)]
struct TransactionRollback {
    restored: Vec<String>,
    remaining: Vec<String>,
}

#[derive(Debug)]
struct SetupTransaction {
    component: ComponentInstallJournal,
    shared_engine: ComponentInstallJournal,
    config: ConfigMutationJournal,
    prepared_file: Option<PreparedFileJournal>,
    gnome_input_sources_original: Option<crate::gnome_input_sources::Snapshot>,
    session: Option<SessionSnapshot>,
}

impl SetupTransaction {
    fn capture(terminals: bool) -> Result<Self> {
        let prepared_file = if terminals {
            Some(PreparedFileJournal::capture(
                terminal_drop_in().context("cannot locate the systemd user directory")?,
            )?)
        } else {
            None
        };
        let gnome_input_sources_original = if IbusDesktop::detect() == IbusDesktop::Gnome {
            Some(
                crate::gnome_input_sources::Snapshot::capture()
                    .context("could not snapshot original GNOME input sources before setup")?,
            )
        } else {
            None
        };
        Ok(Self {
            // All persistent snapshots are taken before the first mutation.
            component: ComponentInstallJournal::capture()?,
            shared_engine: ComponentInstallJournal::capture_limited(
                shared_engine_path(),
                MAX_SHARED_ENGINE_BYTES,
            )?,
            config: ConfigMutationJournal::capture()?,
            prepared_file,
            gnome_input_sources_original,
            session: None,
        })
    }

    fn rollback(&self) -> TransactionRollback {
        let mut report = TransactionRollback::default();
        if let Some(session) = &self.session {
            let failures = session.rollback();
            if failures.is_empty() {
                report.restored.push(
                    "active input source, IBus preload list, KWin route, and service states".into(),
                );
            } else {
                report.remaining.extend(failures);
            }
        }
        if let Some(prepared_file) = &self.prepared_file {
            match prepared_file.restore() {
                Ok(true) => report.restored.push(format!(
                    "prepared service drop-in {} restored",
                    prepared_file.target.display()
                )),
                Ok(false) => {}
                Err(error) => report.remaining.push(format!(
                    "prepared service drop-in {}: {error}",
                    prepared_file.target.display()
                )),
            }
        }
        match self.config.rollback() {
            Ok(restored) => report.restored.extend(restored),
            Err(error) => report.remaining.push(error.to_string()),
        }
        match self.component.restore() {
            Ok(true) if self.component.original.is_some() => {
                report.restored.push(format!(
                    "original IBus component restored (SHA-256 {})",
                    self.component
                        .original
                        .as_ref()
                        .map(|state| state.sha256.as_str())
                        .unwrap_or("unknown")
                ));
            }
            Ok(true) => report.restored.push("new IBus component removed".into()),
            Ok(false) => {}
            Err(error) => report.remaining.push(format!(
                "IBus component {}: {error}",
                self.component.target.display()
            )),
        }
        match self.shared_engine.restore() {
            Ok(true) if self.shared_engine.original.is_some() => report
                .restored
                .push("original shared IBus engine restored".into()),
            Ok(true) => report
                .restored
                .push("new shared IBus engine removed".into()),
            Ok(false) => {}
            Err(error) => report.remaining.push(format!(
                "shared IBus engine {}: {error}",
                self.shared_engine.target.display()
            )),
        }
        report
    }

    fn retained_prepared_changes(&self) -> Vec<String> {
        let mut changes = Vec::new();
        if self.session.as_ref().is_some_and(|session| {
            !session.ibus_daemon_was_running && IbusDaemonProcess::find().is_some()
        }) {
            changes.push(
                "the desktop IBus daemon is now running and remains under desktop-session control"
                    .into(),
            );
        }
        if self.component.install_attempted && self.component.installed != self.component.original {
            let state = self
                .component
                .installed
                .as_ref()
                .map(|state| format!("SHA-256 {}", state.sha256))
                .unwrap_or_else(|| "state could not be verified".into());
            changes.push(format!(
                "IBus component at {} was {} by setup ({state})",
                self.component.target.display(),
                if self.component.original.is_some() {
                    "replaced"
                } else {
                    "installed"
                },
            ));
        }
        if self.config.last_written_revision.is_some() {
            changes.push(
                "WayExpand setup configuration changes are retained (inspect compatibility_mode and settings.terminal_apps)".into(),
            );
        }
        if let Some(file) = &self.prepared_file {
            if file.attempted && file.written != file.original {
                changes.push(format!(
                    "prepared service drop-in at {} is retained",
                    file.target.display()
                ));
            }
        }
        changes
    }
}

fn transaction_rollback_summary(report: TransactionRollback) -> String {
    if report.remaining.is_empty() {
        if report.restored.is_empty() {
            "No persistent setup changes needed rollback.".into()
        } else {
            format!("Setup rollback restored: {}.", report.restored.join("; "))
        }
    } else {
        let restored = if report.restored.is_empty() {
            "No changes were confirmed restored.".to_owned()
        } else {
            format!("Restored: {}.", report.restored.join("; "))
        };
        format!(
            "Setup rollback was incomplete. {restored} Changes remaining or requiring review: {}.",
            report.remaining.join("; ")
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OriginalServiceState {
    service: UserService,
    installed: bool,
    enabled: bool,
    active: bool,
}

impl OriginalServiceState {
    fn capture(service: UserService, state: UnitState) -> Self {
        Self {
            service,
            installed: state.installed,
            enabled: state.enabled,
            active: state.active,
        }
    }
}

impl SessionSnapshot {
    fn capture(
        inventory: &ServiceInventory,
        original_gnome_input_sources: Option<crate::gnome_input_sources::Snapshot>,
    ) -> Result<Self> {
        let ibus_daemon_running = IbusDaemonProcess::find().is_some();
        let global_engine = ibus_daemon_running
            .then(|| command_output_with_status("ibus", &["engine"]))
            .flatten()
            .map(|(_, engine)| engine)
            .map(|engine| engine.trim().to_owned())
            .filter(|engine| !engine.is_empty());
        if ibus_daemon_running && global_engine.is_none() {
            bail!("could not capture the active IBus input source; refusing to change the session without a rollback value");
        }
        Ok(Self {
            ibus_daemon_was_running: ibus_daemon_running,
            global_engine,
            preload_engines: command_output(
                "gsettings",
                &["get", "org.freedesktop.ibus.general", "preload-engines"],
            )
            .map(|value| value.trim().to_owned()),
            // Captured immediately before GNOME source activation, after any
            // potentially slow IBus registry refresh.
            gnome_input_sources: None,
            gnome_input_sources_original: original_gnome_input_sources,
            services: UserService::ALL
                .into_iter()
                .map(|service| OriginalServiceState::capture(service, inventory.state(service)))
                .collect(),
            ..Self::default()
        })
    }

    /// Undo runtime session changes. Persistent component and config changes
    /// are restored by the enclosing `SetupTransaction`.
    fn rollback(&self) -> Vec<String> {
        let mut failures = self.rollback_with(
            kwriteconfig(),
            |program, arguments| {
                run_setup_command(program, arguments).is_ok_and(|status| status.success())
            },
            |program, arguments| {
                if program == "ibus" && arguments == ["engine"] {
                    command_output_with_status(program, arguments).map(|(_, output)| output)
                } else {
                    command_output(program, arguments)
                }
            },
        );
        let restored = ServiceInventory::probe();
        for original in &self.services {
            if !original.installed {
                continue;
            }
            let actual = restored.state(original.service);
            if actual.installed != original.installed
                || actual.enabled != original.enabled
                || actual.active != original.active
            {
                failures.push(format!(
                    "{} expected installed={}, enabled={}, active={} but found installed={}, enabled={}, active={}",
                    original.service.unit(),
                    original.installed,
                    original.enabled,
                    original.active,
                    actual.installed,
                    actual.enabled,
                    actual.active
                ));
            }
        }
        if !self.ibus_daemon_was_running && IbusDaemonProcess::find().is_some() {
            failures.push(
                "IBus daemon was not running before setup and remains running after rollback"
                    .into(),
            );
        }
        if !failures.is_empty() {
            eprintln!(
                "Setup rollback could not restore every service state: {}",
                failures.join(", ")
            );
        }
        failures
    }

    fn rollback_with(
        &self,
        kwriteconfig_program: Option<&str>,
        mut run: impl FnMut(&str, &[&str]) -> bool,
        mut read: impl FnMut(&str, &[&str]) -> Option<String>,
    ) -> Vec<String> {
        println!("Setup did not complete; restoring your previous input-method setup…");
        let mut failures = Vec::new();
        for original in &self.services {
            if !original.installed {
                continue;
            }
            let unit = original.service.unit();
            let enable_action = if original.enabled {
                "enable"
            } else {
                "disable"
            };
            if !run("systemctl", &["--user", enable_action, unit]) {
                failures.push(format!("{unit} enabled={}", original.enabled));
            }
            let active_action = if original.active { "start" } else { "stop" };
            if !run("systemctl", &["--user", active_action, unit]) {
                failures.push(format!("{unit} active={}", original.active));
            }
        }
        if let (Some(previous), Some(kwriteconfig)) =
            (&self.kwin_input_method, kwriteconfig_program)
        {
            let mut arguments = KWIN_INPUT_METHOD_KEY.to_vec();
            match previous {
                Some(value) => arguments.push(value),
                None => arguments.push("--delete"),
            }
            if !run(kwriteconfig, &arguments) {
                failures.push("KWin input method".into());
            }
        }
        if let (Some(expected), Some(original)) =
            (&self.preload_engines_after, &self.preload_engines)
        {
            let current = read(
                "gsettings",
                &["get", "org.freedesktop.ibus.general", "preload-engines"],
            )
            .map(|value| value.trim().to_owned());
            if current.as_deref() == Some(expected.as_str()) {
                if !run(
                    "gsettings",
                    &[
                        "set",
                        "org.freedesktop.ibus.general",
                        "preload-engines",
                        original,
                    ],
                ) {
                    failures.push("IBus preloaded engines could not be restored".into());
                }
            } else if current.as_deref() != Some(original.as_str()) {
                failures.push(
                    "IBus preload list changed after setup; leaving the newer value untouched"
                        .into(),
                );
            }
        }
        if let Some(gnome) = self
            .gnome_input_sources
            .as_ref()
            .or(self.gnome_input_sources_original.as_ref())
        {
            if let Err(error) = gnome.restore_with(
                |key, value| {
                    run(
                        "gsettings",
                        &["set", "org.gnome.desktop.input-sources", key, value],
                    )
                },
                || crate::gnome_input_sources::active_source().ok(),
                |source| crate::gnome_input_sources::activate_source(source).is_ok(),
            ) {
                failures.push(format!("GNOME input sources ({error})"));
            }
        }
        if let Some(engine) = self
            .global_engine
            .as_deref()
            .filter(|engine| *engine != "wayexpand")
        {
            let requested = run("ibus", &["engine", engine]);
            let selected = read("ibus", &["engine"]).map(|value| value.trim().to_owned());
            if selected
                .as_deref()
                .is_some_and(|selected| selected != engine)
                || (!requested && selected.as_deref() != Some(engine))
            {
                failures.push("IBus selected engine".into());
            }
        }
        failures
    }
}

/// Return a loaded systemd user unit only when it demonstrably owns the
/// running IBus daemon. IBus unit names differ between distributions, so
/// discovery is based on the manager's live unit list and MainPID.
fn systemd_unit_for_ibus(pid: u32) -> Option<String> {
    let units = command_output(
        "systemctl",
        &[
            "--user",
            "list-units",
            "--type=service",
            "--all",
            "--no-legend",
            "--no-pager",
        ],
    )?;
    units.lines().find_map(|line| {
        let unit = line.split_whitespace().next()?;
        let lower = unit.to_ascii_lowercase();
        if !unit.ends_with(".service")
            || !(lower.starts_with("ibus") || lower.starts_with("org.freedesktop.ibus"))
        {
            return None;
        }
        let main_pid = command_output(
            "systemctl",
            &["--user", "show", unit, "--property=MainPID", "--value"],
        )?;
        (main_pid.trim().parse::<u32>().ok() == Some(pid)).then(|| unit.to_owned())
    })
}

fn gnome_session_connection_state(report: &chain::IbusChainReport) -> &'static str {
    let session = report.check("gnome-session").map(|check| check.state);
    let daemon = report.check("daemon").map(|check| check.state);
    match (session, daemon) {
        (Some(IbusCheckState::Pass), Some(IbusCheckState::Pass)) => "PASS",
        (Some(IbusCheckState::Fail), _) | (_, Some(IbusCheckState::Fail)) => "FAIL",
        _ => "UNKNOWN",
    }
}

fn gnome_activation_failures(report: &chain::IbusChainReport) -> Vec<String> {
    [
        "gnome-session",
        "gnome-source-registered",
        "gnome-source-active",
        "daemon",
        "registered",
        "selected",
    ]
    .iter()
    .filter_map(|id| report.check(id))
    .filter(|check| check.state != IbusCheckState::Pass)
    .map(|check| format!("{}: {}", check.title, check.detail))
    .collect()
}

fn restart_failure(desktop: IbusDesktop, reason: &str) -> anyhow::Error {
    if desktop != IbusDesktop::Gnome {
        return anyhow::anyhow!("could not refresh the IBus component registry: {reason}");
    }
    let report = chain::diagnose();
    let registration = report
        .check("gnome-source-registered")
        .map(|check| check.state)
        .unwrap_or(IbusCheckState::Unknown);
    let registry = if wayexpand_backend_ibus::engine_available() {
        "PASS"
    } else {
        "FAIL"
    };
    anyhow::anyhow!(
        "WX-GNOME-IBUS-RESTART: {reason}\nGNOME input-source registration: {}\nIBus registry: {}\nDesktop session connection: {}\nSetup will preserve or restore the previous input source.",
        registration.label().to_ascii_uppercase(),
        registry,
        gnome_session_connection_state(&report),
    )
}

/// Refresh the IBus registry only when the installed engine is absent from
/// the live daemon. Prefer a discovered systemd unit only when it owns that
/// exact daemon process; otherwise use IBus's own restart command and check
/// its result. GNOME must reconnect both its Shell and IBus session-bus names.
fn refresh_ibus_component_registry(desktop: IbusDesktop) -> Result<()> {
    if wayexpand_backend_ibus::engine_available() {
        return Ok(());
    }
    let daemon = IbusDaemonProcess::find();
    if desktop == IbusDesktop::Gnome {
        let report = chain::diagnose();
        let session_state = report
            .check("gnome-session")
            .map(|check| check.state)
            .unwrap_or(IbusCheckState::Unknown);
        let daemon_state = report
            .check("daemon")
            .map(|check| check.state)
            .unwrap_or(IbusCheckState::Unknown);
        if session_state != IbusCheckState::Pass
            || (daemon.is_some() && daemon_state != IbusCheckState::Pass)
        {
            return Err(restart_failure(
                desktop,
                "the active GNOME/IBus session could not be confirmed; no restart was attempted",
            ));
        }
    }
    let restart_result = if let Some(pid) = daemon.as_ref().map(|process| process.pid) {
        if let Some(unit) = systemd_unit_for_ibus(pid) {
            step(&format!(
                "Refreshing IBus component registry through {unit}"
            ));
            run_setup_command("systemctl", &["--user", "restart", &unit])
        } else {
            step("Refreshing the IBus component registry (the desktop-managed IBus session may pause briefly)");
            run_setup_command("ibus", &["restart"])
        }
    } else {
        step("Starting the desktop IBus session so it can load the WayExpand component");
        run_setup_command("ibus", &["start"])
    };
    let status = restart_result.map_err(|error| restart_failure(desktop, &error.to_string()))?;
    if !status.success() {
        return Err(restart_failure(
            desktop,
            &format!("IBus start/restart command exited with {status}"),
        ));
    }
    let connected_and_registered = || {
        let Some(_daemon) = IbusDaemonProcess::find() else {
            return false;
        };
        let ibus_owner = command_output(
            "busctl",
            &[
                "--user",
                "--timeout=1s",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                "s",
                "org.freedesktop.IBus",
            ],
        )
        .is_some_and(|value| value.trim() == "b true");
        let desktop_connected = if desktop == IbusDesktop::Gnome {
            command_output(
                "busctl",
                &[
                    "--user",
                    "--timeout=1s",
                    "call",
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "NameHasOwner",
                    "s",
                    "org.gnome.Shell",
                ],
            )
            .is_some_and(|value| value.trim() == "b true")
        } else {
            true
        };
        ibus_owner && desktop_connected && wayexpand_backend_ibus::engine_available()
    };
    if !wait_until(Duration::from_secs(20), connected_and_registered) {
        return Err(restart_failure(
            desktop,
            "IBus restarted, but the WayExpand engine or desktop session did not reconnect",
        ));
    }
    step("IBus reconnected and registered the WayExpand engine");
    Ok(())
}

const KWIN_INPUT_METHOD_KEY: [&str; 7] = [
    "--notify",
    "--file",
    "kwinrc",
    "--group",
    "Wayland",
    "--key",
    "InputMethod",
];

fn kwriteconfig() -> Option<&'static str> {
    ["kwriteconfig6", "kwriteconfig5"]
        .into_iter()
        .find(|program| command_output("sh", &["-c", &format!("command -v {program}")]).is_some())
}

/// Which daemon unit runs beside IBus after setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompanionService {
    /// The terminal-only evdev daemon (it also serves the control socket).
    Terminals,
    /// The control-only daemon.
    Control,
    /// No companion unit is installed; IBus still expands.
    None,
}

/// Phase 1: everything setup needs, checked before anything changes.
fn preflight(options: IbusSetupOptions, inventory: &ServiceInventory) -> Result<CompanionService> {
    if command_output("sh", &["-c", "command -v ibus"]).is_none() {
        bail!("IBus is not installed; install your distribution's `ibus` package and rerun setup");
    }
    if engine_binary().is_none() {
        bail!("`wayexpand-ibus` is not on PATH; reinstall WayExpand (user installs need ~/.local/bin on PATH)");
    }
    let path = default_config_path();
    Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    if IbusDesktop::detect() == IbusDesktop::KdeWayland
        && !kwin_input_method().is_some_and(|value| value.to_ascii_lowercase().contains("ibus"))
    {
        if !Path::new(KWIN_IBUS_INPUT_METHOD).exists() {
            bail!(
                "{KWIN_IBUS_INPUT_METHOD} is missing; install the IBus GTK 3 panel (for example the `ibus-gtk3` package) and rerun setup"
            );
        }
        if kwriteconfig().is_none() {
            bail!("kwriteconfig6 is not installed; set System Settings → Keyboard → Virtual Keyboard → \"IBus Wayland\" yourself, then rerun setup");
        }
    }
    // Terminal coverage set up earlier stays, unless --terminals is asked
    // for anew: enabling the control unit would stop it (Conflicts=).
    let keep_terminals = !options.terminals
        && terminal_drop_in().is_some_and(|path| path.exists())
        && inventory.state(UserService::Evdev).enabled;
    if options.terminals || keep_terminals {
        if !inventory.installed(UserService::Evdev) {
            bail!("wayexpand-evdev.service is not installed; reinstall WayExpand, then rerun setup with --terminals");
        }
        let marker = evdev_consent_marker()
            .context("cannot locate the evdev consent marker: HOME is not set")?;
        if !marker.exists() {
            bail!(
                "terminal coverage uses evdev, which reads every keystroke (it only expands in terminal_apps).\n\
                 After reviewing docs/BACKENDS_SENSITIVE_FIELDS.md, accept that with:\n  \
                 install -d -m 0700 {}\n  install -m 0600 /dev/null {}\n\
                 then rerun `wayexpand setup --backend ibus --terminals`. Nothing was changed.",
                marker.parent().unwrap_or(&marker).display(),
                marker.display()
            );
        }
        if keep_terminals {
            step("Keeping the terminal coverage set up earlier (wayexpand-evdev.service)");
        }
        return Ok(CompanionService::Terminals);
    }
    if inventory.installed(UserService::Control) {
        Ok(CompanionService::Control)
    } else {
        println!("  Note: wayexpand-control.service is not installed, so quick insert and hotkeys");
        println!("  will not run beside IBus. Reinstall WayExpand to add it; IBus expansion works without it.");
        Ok(CompanionService::None)
    }
}

/// Phase 2 (part): the terminal list and the drop-in that limits the evdev
/// daemon to it. Inert until the unit is started.
fn prepare_terminals(
    journal: &mut ConfigMutationJournal,
    file_journal: &mut PreparedFileJournal,
) -> Result<()> {
    let apps = ensure_terminal_apps(journal)?;
    file_journal.write(
        b"# Written by `wayexpand setup --backend ibus --terminals`.\n\
         # IBus expands in apps; this evdev daemon expands only in settings.terminal_apps.\n\
         [Service]\nEnvironment=WAYEXPAND_TERMINAL_APPS_ONLY=1\n",
    )?;
    step(&format!(
        "Prepared wayexpand-evdev.service to expand only in terminal_apps ({} app IDs)",
        apps.len()
    ));
    Ok(())
}

/// Enable and (re)start `service`, recording it for rollback when setup is
/// what enabled it.
fn start_companion(service: UserService) -> Result<()> {
    let unit = service.unit();
    enable_user_service(unit)?;
    // Pick up a changed drop-in or binary in an already running unit. A
    // successful ActiveState probe alone does not prove this restart took
    // effect or that the intended daemon build/config/backend is serving.
    let restart = run_setup_command("systemctl", &["--user", "restart", unit])?;
    if !restart.success() {
        bail!("systemd failed to restart {unit}; see `journalctl --user -u {unit} -n 50`");
    }
    let mut readiness_detail = "service has not become active".to_owned();
    if !wait_until(Duration::from_secs(8), || {
        if !service_active(unit) {
            readiness_detail = "service is not active".to_owned();
            return false;
        }
        match companion_status_error(service) {
            Ok(()) => true,
            Err(error) => {
                readiness_detail = error.to_string();
                false
            }
        }
    }) {
        bail!(
            "{unit} restart completed, but daemon readiness verification failed: {readiness_detail}; see `journalctl --user -u {unit} -n 50`{}",
            if service == UserService::Evdev {
                " (it needs /dev/input access: scripts/install-evdev-permissions.sh)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

/// Stop capture daemons that would expand everywhere next to IBus.
fn stop_global_capture(options: IbusSetupOptions, companion: CompanionService) -> Result<()> {
    for service in [UserService::Evdev, UserService::InputMethod] {
        let unit = service.unit();
        if !service_active(unit) {
            continue;
        }
        // The terminal-only evdev daemon is the companion, not a conflict.
        if service == UserService::Evdev && companion == CompanionService::Terminals {
            continue;
        }
        println!(
            "  {unit} captures keys in every app; with IBus active, snippets would expand twice."
        );
        let stop = options.assume_yes
            || prompt_yes_no(&format!("  Stop and disable {unit}? [Y/n] "), true)?;
        if stop {
            let result = run_setup_command("systemctl", &["--user", "disable", "--now", unit])?;
            if !result.success() {
                bail!("could not stop and disable {unit}; refusing to activate IBus beside an active capture route");
            }
            let after = ServiceInventory::probe().state(service);
            if after.active || after.enabled {
                bail!("{unit} did not become inactive and disabled; refusing to activate IBus beside an active capture route");
            }
            step(&format!("Stopped {unit}"));
        } else {
            bail!("{unit} would expand snippets a second time beside IBus; stop it or use `--terminals`");
        }
    }
    Ok(())
}

/// Phases 3 and 4: change the session, then verify it.
fn activate_and_verify(
    options: IbusSetupOptions,
    companion: CompanionService,
    snapshot: &mut SessionSnapshot,
) -> Result<()> {
    let desktop = IbusDesktop::detect();
    if desktop == IbusDesktop::KdeWayland {
        route_kde_to_ibus(snapshot)?;
    }
    // A healthy registry needs no IBus restart. New component manifests do
    // require a refresh, which is controlled and verified before selection.
    refresh_ibus_component_registry(desktop)?;
    add_to_preload_engines(
        snapshot.preload_engines.as_deref(),
        &mut snapshot.preload_engines_after,
    )?;
    restart_stale_engine();
    step("Selecting WayExpand as the active input method");
    if IbusDesktop::detect() == IbusDesktop::Gnome {
        crate::gnome_input_sources::Snapshot::register_and_activate(
            &mut snapshot.gnome_input_sources,
        )
        .context("GNOME input-source settings are unavailable; refusing partial IBus setup")?;
        step("Registered and activated WayExpand in GNOME input sources");
    }
    // The exit status is meaningless for `default`-layout engines; read back.
    let _ = run_setup_command("ibus", &["engine", "wayexpand"]);
    if !wait_until(
        Duration::from_secs(5),
        wayexpand_backend_ibus::global_engine_selected,
    ) {
        bail!("could not select the WayExpand IBus engine; check that `ibus list-engine --name-only` lists `wayexpand`");
    }
    stop_global_capture(options, companion)?;
    match companion {
        CompanionService::Terminals => {
            step("Starting the terminal-only evdev daemon");
            start_companion(UserService::Evdev)?;
        }
        CompanionService::Control => {
            step("Starting wayexpand-control.service for quick insert and hotkeys");
            start_companion(UserService::Control)?;
        }
        CompanionService::None => {}
    }
    // Phase 4: verify with the same checks `wayexpand ibus status` runs.
    let report = chain::diagnose();
    if desktop == IbusDesktop::Gnome {
        let failed = gnome_activation_failures(&report);
        if !failed.is_empty() {
            bail!(
                "WX-GNOME-IBUS-ACTIVATION: WayExpand did not remain active in the GNOME session: {}",
                failed.join("; ")
            );
        }
    }
    if let Some(failure) = report.first_failure() {
        bail!(
            "verification failed at \"{}\": {}{}",
            failure.title,
            failure.detail,
            failure
                .fix
                .as_deref()
                .map(|fix| format!(" (fix: {fix})"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

/// Repair only GNOME's persistent WayExpand source and activate it. This is
/// deliberately separate from full IBus setup and records the old values so
/// selection failure cannot leave a partial GNOME source mutation behind.
pub(crate) fn repair_gnome_input_source() -> Result<()> {
    if IbusDesktop::detect() != IbusDesktop::Gnome {
        bail!("GNOME input-source repair must run inside a GNOME session");
    }
    if !wayexpand_backend_ibus::engine_available() {
        bail!("the WayExpand IBus engine is not registered; run `wayexpand setup --backend ibus` first");
    }
    // `ibus engine` can print the active default-layout engine and still exit
    // nonzero; use the route probe for WayExpand before relying on exit status.
    let previous_engine = if wayexpand_backend_ibus::global_engine_selected() {
        Some("wayexpand".to_owned())
    } else {
        command_output_with_status("ibus", &["engine"])
            .map(|(_, value)| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let mut snapshot = SessionSnapshot {
        ibus_daemon_was_running: true,
        global_engine: previous_engine,
        ..SessionSnapshot::default()
    };
    let activation = (|| -> Result<()> {
        crate::gnome_input_sources::Snapshot::register_and_activate(
            &mut snapshot.gnome_input_sources,
        )?;
        let _ = run_setup_command("ibus", &["engine", "wayexpand"]);
        if !wait_until(
            Duration::from_secs(5),
            wayexpand_backend_ibus::global_engine_selected,
        ) {
            bail!("IBus did not report WayExpand as the active engine");
        }
        let report = chain::diagnose();
        let failed = gnome_activation_failures(&report);
        if !failed.is_empty() {
            bail!(
                "WX-GNOME-IBUS-ACTIVATION: WayExpand did not remain active in the GNOME session: {}",
                failed.join("; ")
            );
        }
        Ok(())
    })();
    if let Err(error) = activation {
        let failures = snapshot.rollback_with(
            None,
            |program, arguments| {
                run_setup_command(program, arguments).is_ok_and(|status| status.success())
            },
            |program, arguments| {
                if program == "ibus" && arguments == ["engine"] {
                    command_output_with_status(program, arguments).map(|(_, output)| output)
                } else {
                    command_output(program, arguments)
                }
            },
        );
        bail!(
            "GNOME source repair failed ({error}); prior settings were restored{}",
            if failures.is_empty() {
                String::new()
            } else {
                format!(" with rollback issues: {}", failures.join(", "))
            }
        );
    }
    Ok(())
}

pub(crate) fn configure_ibus(options: IbusSetupOptions) -> Result<()> {
    let inventory = ServiceInventory::probe();
    println!("Checking what the IBus route needs…");
    let companion = preflight(options, &inventory)?;
    step("Preflight passed");
    if IbusDesktop::detect() == IbusDesktop::KdeWayland && remove_legacy_plasma_ibus_environment()?
    {
        println!("Removed the obsolete WayExpand GTK/Qt overrides for Plasma Wayland; log out and back in before testing so KWin and the D-Bus session restart with clean input-method settings.");
    }

    // Journal installation and persistent configuration before the first
    // mutation; activation state is captured immediately before it changes.
    let mut transaction = SetupTransaction::capture(companion == CompanionService::Terminals)?;
    let preparation = (|| -> Result<bool> {
        install_component(
            &mut transaction.component,
            &mut transaction.shared_engine,
            options,
        )?;
        let authorization_required = ensure_replacement_consent(options, &mut transaction.config)?;
        if companion == CompanionService::Terminals && options.terminals {
            prepare_terminals(
                &mut transaction.config,
                transaction
                    .prepared_file
                    .as_mut()
                    .expect("terminal drop-in journal captured"),
            )?;
        }
        Ok(authorization_required)
    })();
    let replacement_authorization_required = match preparation {
        Ok(required) => required,
        Err(error) => {
            let rollback = transaction.rollback();
            let summary = transaction_rollback_summary(rollback);
            bail!("setup preparation failed: {error}\n{summary}");
        }
    };

    // Capture live session state after inert preparation, just before it
    // changes. The transaction still owns the installation/config snapshots.
    let service_snapshot = ServiceInventory::probe();
    let session_snapshot = SessionSnapshot::capture(
        &service_snapshot,
        transaction.gnome_input_sources_original.clone(),
    );
    transaction.session = match session_snapshot {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            let rollback = transaction.rollback();
            let summary = transaction_rollback_summary(rollback);
            bail!("could not snapshot session state before activation: {error}\n{summary}");
        }
    };
    if let Err(error) = activate_and_verify(
        options,
        companion,
        transaction
            .session
            .as_mut()
            .expect("session snapshot captured"),
    ) {
        let rollback = transaction.rollback();
        let summary = transaction_rollback_summary(rollback);
        bail!("IBus activation failed: {error}\n{summary}");
    }
    println!();
    let report = chain::diagnose();
    let status = crate::doctor::status::read_daemon_status()
        .ok()
        .and_then(|response| crate::doctor::status::status_as_json(&response).ok());
    let overlap = crate::commands::ibus::RouteOverlap::from_status(status.as_ref());
    crate::commands::ibus::print_report(&report, &overlap);
    println!();
    if replacement_authorization_required {
        println!("SETUP INCOMPLETE: WX-POLICY-001");
        println!("IBus was configured and activated, but text replacement is disabled by policy.");
        println!("Required user action: review and authorize compatibility_mode = \"tracked-focus\" under [settings], then rerun `wayexpand setup --backend ibus`.");
        let retained = transaction.retained_prepared_changes();
        if !retained.is_empty() {
            println!("Prepared changes retained by this policy-limited setup:");
            for change in retained {
                println!("  - {change}");
            }
        }
        bail!("WX-POLICY-001: IBus is active, but replacement authorization is required before expansions can work")
    }
    if let Some(problem_id) = setup_route_failure_problem_id(&report, &overlap) {
        println!("SETUP RESULT: ACTION REQUIRED ({problem_id})");
        if let Some(failure) = report.first_failure() {
            println!(
                "IBus configuration was retained, but a required route check failed: {}.",
                failure.title
            );
            if let Some(fix) = &failure.fix {
                println!("Repair: {fix}");
            }
        } else {
            println!("IBus configuration was retained, but another global WayExpand capture route overlaps it.");
            let control_installed = crate::services::ServiceInventory::probe()
                .installed(crate::services::UserService::Control);
            if let Some((_, _, Some(fix))) = overlap.finding(true, control_installed) {
                println!("Repair: {fix}");
            }
        }
        bail!("{problem_id}: IBus is configured but the selected desktop route is not operational")
    }
    println!(
        "IBus is configured and active; actual target-app insertion: {}.",
        if options.real_insertion_verified {
            "user-confirmed for the tested application"
        } else {
            "NOT TESTED"
        }
    );
    println!("To complete the first-run check, run `wayexpand ibus watch`, switch to a test application, and type a harmless trigger.");
    if companion != CompanionService::Terminals {
        println!("Terminal coverage is optional and is offered separately after the real IBus insertion test.");
    }
    Ok(())
}

fn setup_route_failure_problem_id(
    report: &chain::IbusChainReport,
    overlap: &crate::commands::ibus::RouteOverlap,
) -> Option<&'static str> {
    if let Some(check) = report.first_failure() {
        return Some(check.problem_id.unwrap_or("WX-IBUS-SETUP-001"));
    }
    let ibus_active = report
        .check("selected")
        .is_some_and(|check| check.state == chain::IbusCheckState::Pass);
    (ibus_active
        && matches!(
            overlap,
            crate::commands::ibus::RouteOverlap::Everywhere { .. }
        ))
    .then_some("WX-IBUS-SETUP-002")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_staging_manifest_is_removed_when_scope_ends() {
        let path = env::temp_dir().join(format!(
            "wayexpand-staged-manifest-test-{}",
            std::process::id()
        ));
        fs::write(&path, b"<component />").unwrap();
        drop(PrivateStagingFile(Some(path.clone())));
        assert!(!path.exists());
    }

    #[test]
    fn plasma_ibus_environment_path_uses_xdg_home_then_home_fallback() {
        assert_eq!(
            plasma_ibus_environment_path_from(
                Some(Path::new("/tmp/custom-config")),
                Some(Path::new("/home/tester")),
            ),
            Some(PathBuf::from(
                "/tmp/custom-config/plasma-workspace/env/wayexpand-ibus.sh"
            )),
        );
        assert_eq!(
            plasma_ibus_environment_path_from(None, Some(Path::new("/home/tester"))),
            Some(PathBuf::from(
                "/home/tester/.config/plasma-workspace/env/wayexpand-ibus.sh"
            )),
        );
        assert_eq!(
            plasma_ibus_environment_path_from(Some(Path::new("relative")), None),
            None,
        );
    }

    #[test]
    fn legacy_plasma_ibus_environment_is_recognized_for_cleanup() {
        let legacy = b"# Written by `wayexpand setup --backend ibus`.\n\
            # Restart GTK/Qt applications after changing this file.\n\
            export GTK_IM_MODULE=ibus\n\
            export QT_IM_MODULE=ibus\n\
            export XMODIFIERS=@im=ibus\n";
        assert!(is_legacy_plasma_ibus_environment(legacy));
    }

    #[test]
    fn legacy_plasma_environment_cleanup_recognizer_keeps_user_scripts() {
        assert!(!is_legacy_plasma_ibus_environment(
            b"# user-managed content\nexport GTK_IM_MODULE=ibus\n"
        ));
        assert!(!is_legacy_plasma_ibus_environment(
            b"# Written by `wayexpand setup --backend ibus`.\n\
              export GTK_IM_MODULE=ibus\n\
              export QT_IM_MODULE=ibus\n\
              export XMODIFIERS=@im=ibus\n\
              echo user-command\n"
        ));
    }

    #[test]
    fn compatibility_off_is_reported_as_incomplete_setup() {
        assert!(replacement_authorization_required(
            wayexpand_core::CompatibilityMode::Off
        ));
        assert!(!replacement_authorization_required(
            wayexpand_core::CompatibilityMode::TrackedFocus
        ));
    }

    #[test]
    fn setup_route_failures_keep_stable_problem_ids() {
        let report = chain::IbusChainReport {
            desktop: chain::IbusDesktop::KdeWayland,
            checks: vec![chain::IbusCheck {
                id: "desktop",
                title: "KWin sends input to IBus",
                state: chain::IbusCheckState::Fail,
                problem_id: Some("WX-KWIN-IBUS-002"),
                detail: "test".into(),
                fix: None,
            }],
            activity: None,
        };
        let no_overlap = crate::commands::ibus::RouteOverlap::None;
        assert_eq!(
            setup_route_failure_problem_id(&report, &no_overlap),
            Some("WX-KWIN-IBUS-002")
        );

        let generic_failure = chain::IbusChainReport {
            desktop: chain::IbusDesktop::KdeWayland,
            checks: vec![chain::IbusCheck {
                id: "desktop",
                title: "KWin sends input to IBus",
                state: chain::IbusCheckState::Fail,
                problem_id: None,
                detail: "test".into(),
                fix: None,
            }],
            activity: None,
        };
        assert_eq!(
            setup_route_failure_problem_id(&generic_failure, &no_overlap),
            Some("WX-IBUS-SETUP-001")
        );
        let selected_with_overlap = chain::IbusChainReport {
            desktop: chain::IbusDesktop::KdeWayland,
            checks: vec![chain::IbusCheck {
                id: "selected",
                title: "WayExpand is the active input method",
                state: chain::IbusCheckState::Pass,
                problem_id: None,
                detail: "test".into(),
                fix: None,
            }],
            activity: None,
        };
        let everywhere = crate::commands::ibus::RouteOverlap::Everywhere {
            source: "evdev".into(),
        };
        assert_eq!(
            setup_route_failure_problem_id(&selected_with_overlap, &everywhere),
            Some("WX-IBUS-SETUP-002")
        );
        assert_eq!(
            setup_route_failure_problem_id(
                &chain::IbusChainReport {
                    desktop: chain::IbusDesktop::KdeWayland,
                    checks: vec![],
                    activity: None,
                },
                &no_overlap
            ),
            None
        );
    }

    fn healthy_companion_status(source: &str) -> serde_json::Value {
        serde_json::json!({
            "status_schema": crate::CONTROL_STATUS_SCHEMA,
            "daemon_commit": crate::build_info::COMMIT,
            "daemon_version": crate::build_info::VERSION,
            "source": source,
            "backend": "libei",
            "config_state": "ok",
            "config_generation": 1,
            "config_active_path": default_config_path().display().to_string(),
            "output_connection_state": "connected",
            "output_health": "ok",
            "state": if source == "evdev" { "connected" } else { "running" },
            "route_available": source == "evdev",
            "matching_available": source == "evdev",
            "input_connection_state": if source == "evdev" { "connected" } else { "unknown" },
        })
    }

    #[test]
    fn companion_readiness_checks_build_schema_config_backend_and_runtime() {
        let mut status = healthy_companion_status("evdev");
        assert_eq!(companion_readiness_error(UserService::Evdev, &status), None);

        status["daemon_commit"] = "older-build".into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("build identity"));
        status = healthy_companion_status("evdev");
        status["status_schema"] = (crate::CONTROL_STATUS_SCHEMA - 1).into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("schema"));
        status = healthy_companion_status("evdev");
        status["backend"] = "other".into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("expected libei"));
        status = healthy_companion_status("evdev");
        status["config_generation"] = 0.into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("configuration"));
        status = healthy_companion_status("evdev");
        status["matching_available"] = false.into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("capture or matching"));
    }

    #[test]
    fn control_companion_requires_healthy_output_but_not_text_capture() {
        let status = healthy_companion_status("stdin");
        assert_eq!(
            companion_readiness_error(UserService::Control, &status),
            None
        );
    }

    #[test]
    fn gsettings_string_lists_round_trip() {
        assert_eq!(parse_string_list("@as []\n"), Some(Vec::new()));
        assert_eq!(
            parse_string_list("['xkb:us::eng', 'wayexpand']\n"),
            Some(vec!["xkb:us::eng".to_owned(), "wayexpand".to_owned()])
        );
        assert_eq!(parse_string_list("not a list"), None);
        assert_eq!(
            format_string_list(&["xkb:us::eng".into(), "wayexpand".into()]),
            "['xkb:us::eng', 'wayexpand']"
        );
    }

    #[test]
    fn installed_manifest_uses_stable_system_engine_for_multiple_users() {
        let alice_manifest = component_manifest();
        let bob_manifest = component_manifest();
        assert_eq!(alice_manifest, bob_manifest);
        assert!(
            alice_manifest.contains(&format!("<exec>{}</exec>", shared_engine_path().display()))
        );
        let alice_exec = manifest_exec(alice_manifest.as_bytes());
        let bob_exec = manifest_exec(bob_manifest.as_bytes());
        assert!(preserve_shared_registration(true, true, true, alice_exec));
        assert!(preserve_shared_registration(true, true, true, bob_exec));
        assert!(!preserve_shared_registration(
            true,
            true,
            true,
            Some("/home/alice/.local/bin/wayexpand-ibus")
        ));
        assert!(!alice_manifest.contains("/home/alice"));
        assert!(!alice_manifest.contains("/home/bob"));
        assert!(alice_manifest.contains("<name>org.freedesktop.IBus.WayExpand</name>"));
    }

    #[test]
    fn package_owned_ibus_exec_is_recognized_and_never_migrated_as_a_user_manifest() {
        let contract: toml::Value =
            toml::from_str(include_str!("../../../installer-components.toml")).unwrap();
        let package_exec = contract["components"]["ibus"]["package_install_exec"]
            .as_str()
            .unwrap();
        let package_path = contract["components"]["ibus"]["package_install_exec_path"]
            .as_str()
            .unwrap();
        assert_eq!(
            manifest_exec(COMPONENT_TEMPLATE.as_bytes()),
            Some(package_exec)
        );
        let packaged_xml = format!("<component><exec>{package_exec}</exec></component>");
        let absolute_packaged_xml = format!("<component><exec>{package_path}</exec></component>");
        assert_eq!(manifest_exec(packaged_xml.as_bytes()), Some(package_exec));
        assert!(is_package_manifest_exec(package_exec));
        assert!(is_package_manifest_exec(package_path));
        assert!(!is_package_manifest_exec(
            "/home/alice/.local/bin/wayexpand-ibus"
        ));
        assert_eq!(
            manifest_exec(absolute_packaged_xml.as_bytes()),
            Some(package_path)
        );
        assert!(preserve_package_registration(
            true,
            true,
            true,
            Some(package_exec)
        ));
        assert!(!preserve_package_registration(
            true,
            false,
            true,
            Some(package_exec)
        ));
        assert!(!preserve_package_registration(
            true,
            true,
            false,
            Some(package_exec)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn stale_engine_scan_lists_only_older_wayexpand_ibus_processes() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wayexpand-stale-ibus-test-{}-{unique}",
            std::process::id()
        ));
        let proc_root = root.join("proc");
        fs::create_dir_all(&proc_root).unwrap();
        let installed = root.join("current-engine");
        let old = root.join("previous-engine");
        fs::write(&installed, b"current").unwrap();
        fs::write(&old, b"previous").unwrap();

        for (pid, executable, comm) in [
            (101, &installed, "wayexpand-ibus"),
            (202, &old, "wayexpand-ibus"),
            (303, &old, "other-process"),
            (404, &old, "ibus-wrapper"),
        ] {
            let process = proc_root.join(pid.to_string());
            fs::create_dir(&process).unwrap();
            fs::write(process.join("comm"), format!("{comm}\n")).unwrap();
            symlink(executable, process.join("exe")).unwrap();
        }
        fs::write(
            proc_root.join("404/cmdline"),
            b"/usr/libexec/wayexpand-ibus\0--ibus\0",
        )
        .unwrap();

        let stale = stale_engine_processes_at(
            &proc_root,
            unsafe { libc::geteuid() },
            &fs::canonicalize(&installed).unwrap(),
        );
        assert_eq!(
            stale,
            vec![
                StaleEngineProcess {
                    pid: 202,
                    executable: old.clone(),
                },
                StaleEngineProcess {
                    pid: 404,
                    executable: old.clone(),
                }
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_terminal_apps_are_valid_terminal_app_ids() {
        let mut config =
            Config::parse("[[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\n").unwrap();
        config.settings.terminal_apps = DEFAULT_TERMINAL_APPS
            .iter()
            .map(|app| (*app).to_owned())
            .collect();
        config.validate().unwrap();
    }

    #[test]
    fn rollback_restores_service_enablement_and_activity_independently() {
        let inventory = ServiceInventory::from_states(vec![
            (
                UserService::Evdev,
                UnitState {
                    installed: true,
                    enabled: false,
                    active: true,
                    failed: false,
                },
            ),
            (
                UserService::Control,
                UnitState {
                    installed: true,
                    enabled: true,
                    active: false,
                    failed: false,
                },
            ),
        ]);
        let snapshot = SessionSnapshot {
            ibus_daemon_was_running: true,
            global_engine: Some("xkb:us::eng".into()),
            preload_engines: Some("['xkb:us::eng']".into()),
            preload_engines_after: Some("['xkb:us::eng', 'wayexpand']".into()),
            kwin_input_method: Some(Some("old-input-method.so".into())),
            gnome_input_sources: None,
            gnome_input_sources_original: None,
            services: UserService::ALL
                .into_iter()
                .map(|service| OriginalServiceState::capture(service, inventory.state(service)))
                .collect(),
        };
        let mut actions = Vec::<(String, Vec<String>)>::new();
        let failures = snapshot.rollback_with(
            Some("kwriteconfig6"),
            |program, arguments| {
                actions.push((
                    program.to_owned(),
                    arguments
                        .iter()
                        .map(|argument| (*argument).to_owned())
                        .collect(),
                ));
                true
            },
            |program, arguments| {
                if program == "ibus" && arguments == ["engine"] {
                    Some("xkb:us::eng".to_owned())
                } else if program == "gsettings"
                    && arguments.first() == Some(&"get")
                    && arguments.get(2) == Some(&"preload-engines")
                {
                    Some("['xkb:us::eng', 'wayexpand']".to_owned())
                } else {
                    None
                }
            },
        );
        assert!(failures.is_empty(), "rollback failures: {failures:?}");

        assert_eq!(
            &actions[..4],
            [
                (
                    "systemctl".into(),
                    vec!["--user", "disable", "wayexpand-evdev.service"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "systemctl".into(),
                    vec!["--user", "start", "wayexpand-evdev.service"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "systemctl".into(),
                    vec!["--user", "enable", "wayexpand-control.service"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "systemctl".into(),
                    vec!["--user", "stop", "wayexpand-control.service"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
            ]
        );
        assert_eq!(
            &actions[4..],
            [
                (
                    "kwriteconfig6".into(),
                    KWIN_INPUT_METHOD_KEY
                        .into_iter()
                        .chain(["old-input-method.so"])
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "gsettings".into(),
                    vec![
                        "set",
                        "org.freedesktop.ibus.general",
                        "preload-engines",
                        "['xkb:us::eng']",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                ),
                (
                    "ibus".into(),
                    vec!["engine", "xkb:us::eng"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
            ]
        );
    }
}
