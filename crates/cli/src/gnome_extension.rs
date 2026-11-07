//! Install, enable and inspect the WayExpand GNOME Shell extension, which
//! provides focused-window tracking (`app_filter`) on GNOME.

use crate::setup::run_setup_command;
use crate::setup_ibus::{command_output, format_string_list, parse_string_list};
use crate::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use wayexpand_backend_gnome_window::{
    disable_extension, enable_extension, extension_files, extension_status, ExtensionState,
    GnomeWindowTracker, EXTENSION_UUID,
};

const SYSTEM_EXTENSION_DIRS: [&str; 2] = [
    "/usr/share/gnome-shell/extensions",
    "/usr/local/share/gnome-shell/extensions",
];

/// The per-user extension directory GNOME Shell reads.
pub(crate) fn user_extension_dir() -> Option<PathBuf> {
    let data_home = env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
    Some(
        data_home
            .join("gnome-shell/extensions")
            .join(EXTENSION_UUID),
    )
}

/// Whether `directory` holds exactly the bundled extension files.
fn matches_bundled(directory: &Path) -> bool {
    extension_files().iter().all(|(name, contents)| {
        fs::read_to_string(directory.join(name)).is_ok_and(|on_disk| on_disk == *contents)
    })
}

fn disk_build_identity(directory: &Path) -> Option<String> {
    let mut digest = Sha256::new();
    for (name, _) in extension_files()
        .into_iter()
        .filter(|(name, _)| *name != "metadata.json")
    {
        let contents = fs::read(directory.join(name)).ok()?;
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(contents);
        digest.update([0xff]);
    }
    use std::fmt::Write as _;
    Some(
        digest
            .finalize()
            .iter()
            .fold(String::from("sha256:"), |mut identity, byte| {
                let _ = write!(identity, "{byte:02x}");
                identity
            }),
    )
}

fn disk_metadata_matches_build(directory: &Path, identity: &str) -> bool {
    let Ok(metadata) = fs::read_to_string(directory.join("metadata.json")) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&metadata)
        .ok()
        .and_then(|metadata| metadata["x-wayexpand-build-id"].as_str().map(str::to_owned))
        .is_some_and(|metadata_identity| metadata_identity == identity)
}

#[derive(Debug)]
struct BuildAssessment {
    state: &'static str,
    ready: bool,
    detail: String,
    fix: Option<String>,
}

fn assess_build_identity(
    on_disk: Option<&str>,
    metadata_matches: bool,
    bundled: &str,
    live: Option<&str>,
    live_error: Option<&str>,
) -> BuildAssessment {
    let Some(on_disk) = on_disk else {
        return BuildAssessment {
            state: "unavailable",
            ready: false,
            detail: "the installed GNOME extension build identity could not be read".into(),
            fix: Some("run `wayexpand setup` to install the bundled GNOME extension".into()),
        };
    };
    if !metadata_matches {
        return BuildAssessment {
            state: "invalid-on-disk",
            ready: false,
            detail:
                "the extension metadata build identity does not match its on-disk implementation"
                    .into(),
            fix: Some("run `wayexpand setup` to reinstall the GNOME extension".into()),
        };
    }
    if on_disk != bundled {
        return BuildAssessment {
            state: "update-required",
            ready: false,
            detail: format!("the installed GNOME extension is build {on_disk}, while this WayExpand installation bundles {bundled}"),
            fix: Some("run `wayexpand setup` to update the installed GNOME extension".into()),
        };
    }
    match live {
        Some(identity) if identity == bundled => BuildAssessment {
            state: "current",
            ready: true,
            detail: format!("GNOME Shell is running the current extension build {identity}"),
            fix: None,
        },
        Some(identity) => BuildAssessment {
            state: "logout-required",
            ready: false,
            detail: format!("the installed extension is build {bundled}, but GNOME Shell still has build {identity} loaded"),
            fix: Some("log out of GNOME and log back in to load the installed extension build".into()),
        },
        None => BuildAssessment {
            state: "logout-required",
            ready: false,
            detail: format!("the installed extension is build {bundled}, but its live build identity could not be verified{}", live_error.map(|error| format!(": {error}")).unwrap_or_default()),
            fix: Some("log out of GNOME and log back in to load and verify the installed extension build".into()),
        },
    }
}

/// Where GNOME Shell will load the extension from: a user copy wins over a
/// system copy.
pub(crate) fn installed_copy() -> Option<PathBuf> {
    user_extension_dir()
        .into_iter()
        .chain(
            SYSTEM_EXTENSION_DIRS
                .iter()
                .map(|directory| Path::new(directory).join(EXTENSION_UUID)),
        )
        .find(|directory| directory.join("metadata.json").is_file())
}

/// Install a fully staged user copy unless a current copy is already present.
fn install_user_copy() -> Result<(PathBuf, Option<ExtensionInstallTransaction>)> {
    if let Some(existing) = installed_copy().filter(|directory| matches_bundled(directory)) {
        return Ok((existing, None));
    }
    let directory = user_extension_dir()
        .context("cannot locate the GNOME Shell extension directory: HOME is not set")?;
    let transaction = install_extension_directory(&directory, &extension_files())?;
    Ok((directory, Some(transaction)))
}

/// Keeps the previous complete directory until GNOME activation has been
/// verified. Unless committed, dropping the transaction attempts to restore
/// the prior directory atomically.
struct ExtensionInstallTransaction {
    directory: PathBuf,
    previous: Option<PathBuf>,
    committed: bool,
}

impl ExtensionInstallTransaction {
    fn commit(&mut self) {
        if let Some(previous) = self.previous.take() {
            if let Err(error) = fs::remove_dir_all(&previous) {
                eprintln!(
                    "warning: could not remove previous GNOME extension version at {}: {error}",
                    previous.display()
                );
            }
        }
        self.committed = true;
    }

    fn rollback(&mut self) -> Result<()> {
        if self.committed {
            return Ok(());
        }
        if let Some(previous) = self.previous.as_ref() {
            exchange_directories(&self.directory, previous)
                .context("could not restore the previous GNOME extension directory")?;
            // The directory path now holds the old version. Mark the rollback
            // complete before cleaning the failed new version so Drop cannot
            // accidentally exchange the failed version back into place.
            self.committed = true;
            let failed_stage = self.previous.take().expect("previous path was present");
            fs::remove_dir_all(failed_stage)
                .context("could not remove the failed GNOME extension stage")?;
        } else {
            fs::remove_dir_all(&self.directory)
                .context("could not remove the failed GNOME extension installation")?;
            self.committed = true;
        }
        if let Some(parent) = self.directory.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

impl Drop for ExtensionInstallTransaction {
    fn drop(&mut self) {
        if !self.committed {
            if let Err(error) = self.rollback() {
                eprintln!(
                    "error: GNOME extension rollback failed at {}: {error}",
                    self.directory.display()
                );
            }
        }
    }
}

/// Stage and verify a complete extension before switching GNOME's directory.
/// The old complete directory remains available until live activation has
/// passed, and can be restored without exposing mixed-build files.
fn install_extension_directory(
    directory: &Path,
    files: &[(&str, &str)],
) -> Result<ExtensionInstallTransaction> {
    install_extension_directory_with(directory, files, write_extension_file)
}

fn write_extension_file(path: &Path, contents: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    file.set_permissions(fs::Permissions::from_mode(0o644))?;
    Ok(())
}

fn install_extension_directory_with(
    directory: &Path,
    files: &[(&str, &str)],
    mut write_file: impl FnMut(&Path, &str) -> Result<()>,
) -> Result<ExtensionInstallTransaction> {
    let parent = directory
        .parent()
        .context("GNOME extension directory has no parent")?;
    fs::create_dir_all(parent)?;
    let stem = directory
        .file_name()
        .context("GNOME extension directory has no name")?
        .to_string_lossy();
    let nonce = format!(
        "{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let staged = parent.join(format!(".{stem}.{nonce}.staged"));
    fs::create_dir(&staged)?;
    let stage_result = (|| -> Result<()> {
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
        for (name, contents) in files {
            let target = staged.join(name);
            write_file(&target, contents)?;
        }
        let identity = wayexpand_backend_gnome_window::bundled_extension_build_identity();
        if !matches_bundled(&staged)
            || disk_build_identity(&staged).as_deref() != Some(identity.as_str())
            || !disk_metadata_matches_build(&staged, &identity)
        {
            bail!(
                "staged GNOME extension files or build identity do not match this WayExpand build"
            );
        }
        fs::File::open(&staged)?.sync_all()?;
        Ok(())
    })();
    if let Err(error) = stage_result {
        let _ = fs::remove_dir_all(&staged);
        return Err(error);
    }

    let had_previous = match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
        Ok(_) => {
            let _ = fs::remove_dir_all(&staged);
            bail!("existing GNOME extension path is not a real directory");
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            let _ = fs::remove_dir_all(&staged);
            return Err(error).context("could not inspect existing GNOME extension directory");
        }
    };
    if had_previous {
        // Linux rename exchange atomically swaps the complete loaded path.
        // If the kernel/filesystem cannot provide it, fail without touching
        // the installed version rather than falling back to a visible gap.
        if let Err(error) = exchange_directories(&staged, directory) {
            let _ = fs::remove_dir_all(&staged);
            return Err(error).context("filesystem cannot atomically switch the GNOME extension");
        }
    } else if let Err(error) = fs::rename(&staged, directory) {
        let _ = fs::remove_dir_all(&staged);
        return Err(error).context("could not activate staged GNOME extension");
    }
    let mut transaction = ExtensionInstallTransaction {
        directory: directory.to_path_buf(),
        previous: had_previous.then_some(staged),
        committed: false,
    };
    if let Err(error) = fs::File::open(parent).and_then(|parent| parent.sync_all()) {
        let rollback = transaction.rollback();
        return Err(error).context(match rollback {
            Ok(()) => "directory switch durability check failed; previous extension restored",
            Err(_) => {
                "directory switch durability check failed and previous extension rollback failed"
            }
        });
    }
    Ok(transaction)
}

fn exchange_directories(left: &Path, right: &Path) -> std::io::Result<()> {
    let left_c = std::ffi::CString::new(left.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let right_c = std::ffi::CString::new(right.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both C strings are NUL-terminated sibling paths under a
    // user-owned directory and remain alive during the atomic exchange.
    let exchanged = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            left_c.as_ptr(),
            libc::AT_FDCWD,
            right_c.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if exchanged == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[derive(Clone)]
struct ExtensionSettings {
    enabled: Vec<String>,
    disabled: Vec<String>,
}

fn write_extension_setting(key: &str, values: &[String]) -> Result<()> {
    let status = run_setup_command(
        "gsettings",
        &["set", "org.gnome.shell", key, &format_string_list(values)],
    )?;
    if !status.success() {
        bail!("gsettings rejected org.gnome.shell {key}");
    }
    Ok(())
}

/// Keep the GNOME enabled/disabled lists recoverable while extension
/// activation is verified. A failed readiness check restores the exact
/// pre-setup lists along with the prior extension directory.
struct ExtensionSettingsTransaction {
    original: ExtensionSettings,
    committed: bool,
    changed_enabled: bool,
    changed_disabled: bool,
}

impl ExtensionSettingsTransaction {
    fn capture() -> Result<Self> {
        let read = |key: &str| -> Result<Vec<String>> {
            let output = command_output("gsettings", &["get", "org.gnome.shell", key])
                .with_context(|| format!("could not read GNOME {key}"))?;
            parse_string_list(&output).with_context(|| format!("could not parse GNOME {key}"))
        };
        Ok(Self {
            original: ExtensionSettings {
                enabled: read("enabled-extensions")?,
                disabled: read("disabled-extensions")?,
            },
            committed: false,
            changed_enabled: false,
            changed_disabled: false,
        })
    }

    fn enable_wayexpand(&mut self) -> Result<()> {
        let mut enabled = self.original.enabled.clone();
        let mut disabled = self.original.disabled.clone();
        if !enabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
            enabled.push(EXTENSION_UUID.to_owned());
            self.changed_enabled = true;
            write_extension_setting("enabled-extensions", &enabled)?;
        }
        if disabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
            disabled.retain(|uuid| uuid != EXTENSION_UUID);
            self.changed_disabled = true;
            write_extension_setting("disabled-extensions", &disabled)?;
        }
        Ok(())
    }

    fn commit(&mut self) {
        self.committed = true;
    }

    fn rollback(&mut self) -> Result<()> {
        if self.committed {
            return Ok(());
        }
        if self.changed_enabled {
            write_extension_setting("enabled-extensions", &self.original.enabled)?;
        }
        if self.changed_disabled {
            write_extension_setting("disabled-extensions", &self.original.disabled)?;
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for ExtensionSettingsTransaction {
    fn drop(&mut self) {
        if !self.committed {
            if let Err(error) = self.rollback() {
                eprintln!("error: could not restore GNOME extension enablement settings: {error}");
            }
        }
    }
}

/// Add the extension to GNOME's enabled list (it persists across logins)
/// and take it off the disabled list.
fn persist_enabled() -> Result<ExtensionSettingsTransaction> {
    let mut transaction = ExtensionSettingsTransaction::capture()?;
    transaction.enable_wayexpand()?;
    Ok(transaction)
}

/// A plain-language line and an optional fix for the extension's state.
pub(crate) fn describe_state(
    state: &ExtensionState,
    user_extensions_enabled: bool,
) -> (bool, String, Option<String>) {
    if !user_extensions_enabled {
        return (
            false,
            "GNOME has user extensions switched off, so the WayExpand window tracker cannot run".into(),
            Some("turn on Extensions in the Extensions app, or `gsettings set org.gnome.shell disable-user-extensions false`".into()),
        );
    }
    match state {
        ExtensionState::Active => (true, "the WayExpand GNOME Shell extension is active; app filters can see the focused app".into(), None),
        ExtensionState::Inactive => (false, "the WayExpand GNOME Shell extension is installed but turned off".into(), Some("run `wayexpand setup`, or `gnome-extensions enable wayexpand-window-tracker@cyberducttape.github.io`".into())),
        ExtensionState::Error(error) => (false, format!("GNOME Shell could not load the WayExpand extension: {}", if error.is_empty() { "no error text" } else { error }), Some("reinstall WayExpand and run `wayexpand setup`; see `journalctl --user -b | grep -i wayexpand` for details".into())),
        ExtensionState::OutOfDate => (false, "this GNOME Shell version is not supported by the WayExpand extension (GNOME 45 or newer is required)".into(), None),
        ExtensionState::Unknown => (false, "GNOME Shell has not loaded the WayExpand extension".into(), Some("run `wayexpand setup`; a newly installed extension starts after you log out and back in once".into())),
        ExtensionState::Other(number) => (false, format!("the WayExpand GNOME Shell extension is in state {number}"), Some("check again in a moment".into())),
    }
}

/// An enabled extension is not operational until its tracker protocol can
/// complete the same handshake the daemon uses. GNOME can report an
/// extension as active while its D-Bus interface is missing or incompatible.
fn verify_tracker_protocol(
    state: &ExtensionState,
    readiness: (bool, String, Option<String>),
    probe: Result<(), String>,
) -> (bool, String, Option<String>) {
    let (ready, message, fix) = readiness;
    if state != &ExtensionState::Active || !ready {
        return (ready, message, fix);
    }
    match probe {
        Ok(()) => (ready, message, fix),
        Err(error) => (
            false,
            format!("the extension is active, but the WayExpand tracker protocol is unavailable: {error}"),
            Some("run `wayexpand setup` to reinstall or enable the WayExpand GNOME Shell extension".into()),
        ),
    }
}

fn probe_if_active(state: &ExtensionState) -> Result<(), String> {
    if state == &ExtensionState::Active {
        GnomeWindowTracker::probe().map_err(|error| error.to_string())
    } else {
        Ok(())
    }
}

/// Setup step on GNOME: install the extension if needed and enable it.
/// App filters are optional, but IBus setup consumes this readiness result
/// because its session snapshot depends on the live Shell bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetupReadiness {
    Ready,
    SessionRestartRequired,
    Unavailable,
}

fn stop_failed_activation() {
    match disable_extension() {
        Ok(true) => println!("  stopped the extension activation attempted during setup"),
        Ok(false) => println!("  GNOME Shell did not confirm stopping the failed extension activation"),
        Err(error) => println!("  could not stop the failed extension activation ({error}); review `wayexpand doctor --gnome`"),
    }
}

pub(crate) fn setup_gnome_window_tracker() -> SetupReadiness {
    println!("GNOME window tracking (for snippets limited to certain apps):");
    let initial_status = match extension_status() {
        Ok(status) => status,
        Err(error) => {
            println!("  GNOME Shell state could not be read before changes ({error}); no extension files or settings were changed");
            return SetupReadiness::Unavailable;
        }
    };
    let (directory, mut install_transaction) = match install_user_copy() {
        Ok(installed) => installed,
        Err(error) => {
            println!("  could not install the WayExpand GNOME Shell extension: {error}");
            return SetupReadiness::Unavailable;
        }
    };
    println!("  extension: {}", directory.display());
    let mut settings_transaction = match persist_enabled() {
        Ok(transaction) => transaction,
        Err(error) => {
            println!("  could not prepare GNOME extension activation: {error}");
            return SetupReadiness::Unavailable;
        }
    };
    let status = match extension_status() {
        Ok(status) => status,
        Err(error) => {
            println!("  GNOME Shell readiness could not be verified ({error}); rolling back the extension update");
            return SetupReadiness::Unavailable;
        }
    };
    let mut activation_attempted = false;
    let status = if status.user_extensions_enabled && status.state != ExtensionState::Active {
        // Shell enables extensions it already knows at once; one installed
        // after this login is refused until the next login.
        activation_attempted = true;
        if let Err(error) = enable_extension() {
            println!(
                "  GNOME Shell could not activate the extension ({error}); rolling back the update"
            );
            if initial_status.state != ExtensionState::Active {
                stop_failed_activation();
            }
            return SetupReadiness::Unavailable;
        }
        match extension_status() {
            Ok(status) => status,
            Err(error) => {
                println!("  activation was requested, but GNOME Shell readiness could not be verified ({error}); rolling back the update");
                if initial_status.state != ExtensionState::Active {
                    stop_failed_activation();
                }
                return SetupReadiness::Unavailable;
            }
        }
    } else {
        status
    };
    let readiness = describe_state(&status.state, status.user_extensions_enabled);
    let protocol = probe_if_active(&status.state);
    let (mut ok, mut message, mut fix) =
        verify_tracker_protocol(&status.state, readiness, protocol);
    let mut session_restart_required = false;
    let bundled_identity = wayexpand_backend_gnome_window::bundled_extension_build_identity();
    let disk_identity = disk_build_identity(&directory);
    let metadata_matches = disk_identity
        .as_deref()
        .is_some_and(|identity| disk_metadata_matches_build(&directory, identity));
    if status.state == ExtensionState::Active {
        let live_result = wayexpand_backend_gnome_window::extension_build_identity();
        let live_identity = live_result.as_ref().ok().map(String::as_str);
        let assessment = assess_build_identity(
            disk_identity.as_deref(),
            metadata_matches,
            &bundled_identity,
            live_identity,
            live_result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .as_deref(),
        );
        if !assessment.ready {
            message = assessment.detail.clone();
            fix = assessment.fix.clone();
            if assessment.state == "logout-required"
                && metadata_matches
                && disk_identity.as_deref() == Some(bundled_identity.as_str())
            {
                // The installed package is complete and GNOME is running the
                // previous build. Keep the new directory for next login,
                // while refusing to call this session operational yet.
                session_restart_required = true;
            } else {
                ok = false;
            }
        }
        println!(
            "  extension build: {} (disk {}, live {})",
            assessment.state,
            disk_identity.as_deref().unwrap_or("unknown"),
            live_identity.unwrap_or("unverified")
        );
    } else if status.state == ExtensionState::Unknown
        && metadata_matches
        && disk_identity.as_deref() == Some(bundled_identity.as_str())
    {
        println!(
            "  extension build: installed {}, awaiting GNOME login to load it",
            disk_identity.as_deref().unwrap_or("unknown")
        );
    }
    println!("  {} {message}", if ok { "✓" } else { "!" });
    if status.state == ExtensionState::Active {
        match wayexpand_backend_gnome_window::active_input_source() {
            Ok((kind, id)) => {
                let source = if kind == "ibus" && id == "wayexpand" {
                    "WayExpand".to_owned()
                } else {
                    "another configured source".to_owned()
                };
                println!("  ✓ GNOME live input-source control: Shell reports {source}");
            }
            Err(error) => println!(
                "  ! GNOME live input-source control is unavailable ({error}); a newly updated extension may require logging out and back in"
            ),
        }
    }
    if status.state == ExtensionState::Unknown && status.user_extensions_enabled {
        println!("  GNOME Shell on Wayland loads newly installed extensions at login: log out and back in once, and app filters start working (it stays enabled after that).");
    } else if let Some(fix) = fix {
        println!("  Fix: {fix}");
    }
    if (session_restart_required
        || (status.state == ExtensionState::Unknown && status.user_extensions_enabled))
        && metadata_matches
        && disk_identity.as_deref() == Some(bundled_identity.as_str())
    {
        if let Some(transaction) = install_transaction.as_mut() {
            transaction.commit();
        }
        settings_transaction.commit();
        return SetupReadiness::SessionRestartRequired;
    }
    if !ok {
        if activation_attempted && initial_status.state != ExtensionState::Active {
            stop_failed_activation();
        }
        return SetupReadiness::Unavailable;
    }
    if let Some(transaction) = install_transaction.as_mut() {
        transaction.commit();
    }
    settings_transaction.commit();
    SetupReadiness::Ready
}

/// Doctor's view of the extension, for human and JSON output. `None` outside
/// GNOME.
pub(crate) fn doctor_report() -> Option<serde_json::Value> {
    if wayexpand_backend_selection::Compositor::detect()
        != wayexpand_backend_selection::Compositor::Gnome
    {
        return None;
    }
    let installed = installed_copy();
    let current = installed.as_deref().is_some_and(matches_bundled);
    let bundled_identity = wayexpand_backend_gnome_window::bundled_extension_build_identity();
    let disk_identity = installed.as_deref().and_then(disk_build_identity);
    let metadata_matches = installed
        .as_deref()
        .zip(disk_identity.as_deref())
        .is_some_and(|(directory, identity)| disk_metadata_matches_build(directory, identity));
    let (state, user_extensions_enabled) = match extension_status() {
        Ok(status) => (Some(status.state), status.user_extensions_enabled),
        Err(_) => {
            // Some headless and restarted GNOME sessions temporarily fail
            // the Extensions manager query while the extension's own tracker
            // object is already live. Its exported identity exists only
            // between enable() and disable(), so a response is direct evidence
            // that the extension instance is running. The later Ping and
            // build-identity checks still verify its protocol and version.
            if wayexpand_backend_gnome_window::extension_build_identity().is_ok() {
                (Some(ExtensionState::Active), true)
            } else {
                (None, true)
            }
        }
    };
    let (protocol_state, protocol_detail) = match &state {
        Some(ExtensionState::Active) => match probe_if_active(&ExtensionState::Active) {
            Ok(()) => (
                "pass",
                "the tracker D-Bus protocol and live build identity responded".to_owned(),
            ),
            Err(error) => (
                "fail",
                format!("the tracker D-Bus handshake failed: {error}"),
            ),
        },
        Some(_) => (
            "unknown",
            "the extension is not loaded, so its D-Bus handshake cannot be tested".to_owned(),
        ),
        None => (
            "unknown",
            "GNOME Shell did not answer on the session bus".to_owned(),
        ),
    };
    let (mut ok, mut message, mut fix) = match &state {
        Some(state) => {
            let readiness = describe_state(state, user_extensions_enabled);
            let protocol = if protocol_state == "pass" {
                Ok(())
            } else {
                Err(protocol_detail.clone())
            };
            verify_tracker_protocol(state, readiness, protocol)
        }
        None => (
            false,
            "GNOME Shell is not answering on the session bus".into(),
            None,
        ),
    };
    let live_identity_result = if state == Some(ExtensionState::Active) {
        Some(wayexpand_backend_gnome_window::extension_build_identity())
    } else {
        None
    };
    let live_identity = live_identity_result
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(String::as_str);
    let live_error = live_identity_result
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .map(ToString::to_string);
    let build = if state == Some(ExtensionState::Active) {
        assess_build_identity(
            disk_identity.as_deref(),
            metadata_matches,
            &bundled_identity,
            live_identity,
            live_error.as_deref(),
        )
    } else if state == Some(ExtensionState::Unknown)
        && metadata_matches
        && disk_identity.as_deref() == Some(bundled_identity.as_str())
    {
        assess_build_identity(
            disk_identity.as_deref(),
            metadata_matches,
            &bundled_identity,
            None,
            Some("GNOME Shell has not loaded the installed extension"),
        )
    } else {
        BuildAssessment {
            state: "not-running",
            ready: true,
            detail: "GNOME Shell has not confirmed a running extension instance".into(),
            fix: None,
        }
    };
    if state == Some(ExtensionState::Active) && !build.ready {
        ok = false;
        message = build.detail.clone();
        fix = build.fix.clone();
    }
    let fix = fix.or_else(|| {
        (!current && installed.is_some())
            .then(|| "run `wayexpand setup` to update the installed extension".into())
    });
    let state_label = match &state {
        Some(ExtensionState::Active) => "active",
        Some(ExtensionState::Inactive) => "inactive",
        Some(ExtensionState::Error(_)) => "error",
        Some(ExtensionState::OutOfDate) => "out-of-date",
        Some(ExtensionState::Unknown) => "not-loaded",
        Some(ExtensionState::Other(_)) => "other",
        None => "shell-unavailable",
    };
    let input_source_control = if state == Some(ExtensionState::Active) {
        wayexpand_backend_gnome_window::active_input_source()
            .ok()
            .map(|(kind, id)| {
                serde_json::json!({
                    "available": true,
                    "wayexpand_active": kind == "ibus" && id == "wayexpand",
                })
            })
            .unwrap_or_else(|| serde_json::json!({"available": false, "wayexpand_active": null}))
    } else {
        serde_json::json!({"available": false, "wayexpand_active": null})
    };
    Some(serde_json::json!({
        "uuid": EXTENSION_UUID,
        "installed_path": installed.map(|path| path.display().to_string()),
        "installed_matches_bundled": current,
        "build_identity": {
            "bundled": bundled_identity,
            "on_disk": disk_identity,
            "on_disk_metadata_matches": metadata_matches,
            "live": live_identity,
            "runtime_state": build.state,
            "detail": build.detail,
            "fix": build.fix,
        },
        "state": state_label,
        "protocol_handshake": {
            "state": protocol_state,
            "detail": protocol_detail,
        },
        "user_extensions_enabled": user_extensions_enabled,
        "ready": ok,
        "detail": message,
        "fix": fix,
        "input_source_control": input_source_control,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_extension_state_explains_itself_and_only_active_is_ready() {
        for state in [
            ExtensionState::Active,
            ExtensionState::Inactive,
            ExtensionState::Error("boom".into()),
            ExtensionState::OutOfDate,
            ExtensionState::Unknown,
            ExtensionState::Other(6),
        ] {
            let (ok, message, _) = describe_state(&state, true);
            assert_eq!(ok, state == ExtensionState::Active, "{state:?}");
            assert!(!message.is_empty());
        }
        let (ok, _, fix) = describe_state(&ExtensionState::Active, false);
        assert!(!ok);
        assert!(fix.unwrap().contains("disable-user-extensions false"));
        let (_, _, fix) = describe_state(&ExtensionState::Unknown, true);
        assert!(fix.unwrap().contains("log out"));
    }

    #[test]
    fn active_extension_is_not_ready_when_tracker_protocol_probe_fails() {
        let result = verify_tracker_protocol(
            &ExtensionState::Active,
            describe_state(&ExtensionState::Active, true),
            Err("protocol version mismatch".into()),
        );
        assert!(!result.0);
        assert!(result.1.contains("protocol version mismatch"));
        assert!(result.2.unwrap().contains("wayexpand setup"));

        let ready = verify_tracker_protocol(
            &ExtensionState::Active,
            describe_state(&ExtensionState::Active, true),
            Ok(()),
        );
        assert!(ready.0);
    }

    #[test]
    fn live_extension_build_must_match_installed_build() {
        let expected = "sha256:current";
        let current = assess_build_identity(Some(expected), true, expected, Some(expected), None);
        assert!(current.ready);
        assert_eq!(current.state, "current");

        let stale = assess_build_identity(
            Some(expected),
            true,
            expected,
            Some("sha256:previous"),
            None,
        );
        assert!(!stale.ready);
        assert_eq!(stale.state, "logout-required");
        assert!(stale
            .detail
            .contains("still has build sha256:previous loaded"));
        assert!(stale.fix.unwrap().contains("log out"));

        let unverified =
            assess_build_identity(Some(expected), true, expected, None, Some("no property"));
        assert!(!unverified.ready);
        assert_eq!(unverified.state, "logout-required");

        let outdated = assess_build_identity(
            Some("sha256:old-disk"),
            true,
            expected,
            Some(expected),
            None,
        );
        assert_eq!(outdated.state, "update-required");
        assert!(outdated.fix.unwrap().contains("wayexpand setup"));
    }

    #[test]
    fn disk_build_identity_and_metadata_are_content_bound() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-gnome-build-id-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for (name, contents) in extension_files() {
            fs::write(root.join(name), contents).unwrap();
        }
        let identity = disk_build_identity(&root).unwrap();
        assert_eq!(
            identity,
            wayexpand_backend_gnome_window::bundled_extension_build_identity()
        );
        assert!(disk_metadata_matches_build(&root, &identity));
        fs::write(root.join("extension.js"), "// newer code").unwrap();
        assert!(!disk_metadata_matches_build(
            &root,
            &disk_build_identity(&root).unwrap()
        ));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn user_copy_is_written_from_the_bundled_files() {
        let root = std::env::temp_dir().join(format!("wayexpand-gnome-ext-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let directory = root.join(EXTENSION_UUID);
        fs::create_dir_all(&directory).unwrap();
        assert!(!matches_bundled(&directory));
        for (name, contents) in extension_files() {
            fs::write(directory.join(name), contents).unwrap();
        }
        assert!(matches_bundled(&directory));
        fs::write(directory.join("extension.js"), "// stale").unwrap();
        assert!(!matches_bundled(&directory));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn extension_update_stages_a_complete_directory_before_switching() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-gnome-ext-atomic-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let directory = root.join(EXTENSION_UUID);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("old-file"), "old version").unwrap();

        let mut transaction = install_extension_directory(&directory, &extension_files()).unwrap();

        assert!(matches_bundled(&directory));
        assert!(!directory.join("old-file").exists());
        transaction.commit();
        let entries = fs::read_dir(&root).unwrap().count();
        assert_eq!(entries, 1, "staging/backup directory leaked after switch");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stage_failures_leave_the_previous_extension_untouched() {
        for (label, errno) in [
            ("interrupted", libc::EINTR),
            ("full-disk", libc::ENOSPC),
            ("permission", libc::EACCES),
        ] {
            let root = std::env::temp_dir().join(format!(
                "wayexpand-gnome-ext-stage-failure-{}-{label}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let directory = root.join(EXTENSION_UUID);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("old-file"), "old complete version").unwrap();

            let mut writes = 0;
            let result = install_extension_directory_with(
                &directory,
                &extension_files(),
                |path, contents| {
                    writes += 1;
                    if writes == 2 {
                        return Err(std::io::Error::from_raw_os_error(errno).into());
                    }
                    write_extension_file(path, contents)
                },
            );
            assert!(result.is_err(), "{label} failure was not propagated");
            assert_eq!(
                fs::read_to_string(directory.join("old-file")).unwrap(),
                "old complete version"
            );
            assert_eq!(
                fs::read_dir(&root).unwrap().count(),
                1,
                "{label} left staging data"
            );
            fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn invalid_staged_build_identity_is_rejected_before_the_directory_switch() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-gnome-ext-invalid-stage-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let directory = root.join(EXTENSION_UUID);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("old-file"), "old complete version").unwrap();

        let result =
            install_extension_directory_with(&directory, &extension_files(), |path, contents| {
                let contents = if path.file_name().is_some_and(|name| name == "extension.js") {
                    "// content from a different build"
                } else {
                    contents
                };
                write_extension_file(path, contents)
            });

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(directory.join("old-file")).unwrap(),
            "old complete version"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn activation_failure_rolls_back_the_whole_previous_directory() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-gnome-ext-activation-rollback-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let directory = root.join(EXTENSION_UUID);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("old-file"), "old complete version").unwrap();

        let mut transaction = install_extension_directory(&directory, &extension_files()).unwrap();
        assert!(
            matches_bundled(&directory),
            "new directory was not activated"
        );
        transaction.rollback().unwrap(); // Simulated GNOME activation/probe failure.

        assert_eq!(
            fs::read_to_string(directory.join("old-file")).unwrap(),
            "old complete version"
        );
        assert!(!matches_bundled(&directory));
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            1,
            "rollback left staging data"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failed_first_install_is_removed_on_rollback() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-gnome-ext-first-install-rollback-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let directory = root.join(EXTENSION_UUID);

        let mut transaction = install_extension_directory(&directory, &extension_files()).unwrap();
        transaction.rollback().unwrap();
        assert!(!directory.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(&root).unwrap();
    }
}
