//! Install, enable and inspect the WayExpand GNOME Shell extension, which
//! provides focused-window tracking (`app_filter`) on GNOME.

use crate::setup::run_setup_command;
use crate::setup_ibus::{command_output, format_string_list, parse_string_list};
use crate::*;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use wayexpand_backend_gnome_window::{
    enable_extension, extension_files, extension_status, ExtensionState, GnomeWindowTracker,
    EXTENSION_UUID,
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

/// Write the bundled extension into the user directory, file by file via
/// temporary files, unless a current copy is already installed.
fn install_user_copy() -> Result<PathBuf> {
    if let Some(existing) = installed_copy().filter(|directory| matches_bundled(directory)) {
        return Ok(existing);
    }
    let directory = user_extension_dir()
        .context("cannot locate the GNOME Shell extension directory: HOME is not set")?;
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))?;
    for (name, contents) in extension_files() {
        let target = directory.join(name);
        let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        drop(file);
        fs::rename(&temporary, &target)?;
    }
    Ok(directory)
}

/// Add the extension to GNOME's enabled list (it persists across logins)
/// and take it off the disabled list.
fn persist_enabled() -> bool {
    let read = |key: &str| {
        command_output("gsettings", &["get", "org.gnome.shell", key])
            .and_then(|output| parse_string_list(&output))
    };
    let write = |key: &str, values: &[String]| {
        run_setup_command(
            "gsettings",
            &["set", "org.gnome.shell", key, &format_string_list(values)],
        )
        .is_ok_and(|status| status.success())
    };
    let Some(mut enabled) = read("enabled-extensions") else {
        return false;
    };
    let mut ok = true;
    if !enabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
        enabled.push(EXTENSION_UUID.to_owned());
        ok &= write("enabled-extensions", &enabled);
    }
    if let Some(mut disabled) = read("disabled-extensions") {
        if disabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
            disabled.retain(|uuid| uuid != EXTENSION_UUID);
            ok &= write("disabled-extensions", &disabled);
        }
    }
    ok
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
/// It never fails setup: app filters are optional, and every outcome is
/// explained with its fix.
pub(crate) fn setup_gnome_window_tracker() {
    println!("GNOME window tracking (for snippets limited to certain apps):");
    let directory = match install_user_copy() {
        Ok(directory) => directory,
        Err(error) => {
            println!("  could not install the WayExpand GNOME Shell extension: {error}");
            return;
        }
    };
    println!("  extension: {}", directory.display());
    if !persist_enabled() {
        println!("  could not add it to GNOME's enabled extensions (gsettings); run `gnome-extensions enable {EXTENSION_UUID}` after your next login");
    }
    let status = match extension_status() {
        Ok(status) => status,
        Err(error) => {
            println!(
                "  GNOME Shell did not answer ({error}); it loads the extension at your next login"
            );
            return;
        }
    };
    let status = if status.user_extensions_enabled && status.state != ExtensionState::Active {
        // Shell enables extensions it already knows at once; one installed
        // after this login is refused until the next login.
        let _ = enable_extension();
        extension_status().unwrap_or(status)
    } else {
        status
    };
    let readiness = describe_state(&status.state, status.user_extensions_enabled);
    let protocol = probe_if_active(&status.state);
    let (ok, message, fix) = verify_tracker_protocol(&status.state, readiness, protocol);
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
    let (state, user_extensions_enabled) = match extension_status() {
        Ok(status) => (Some(status.state), status.user_extensions_enabled),
        Err(_) => (None, true),
    };
    let (ok, message, fix) = match &state {
        Some(state) => {
            let readiness = describe_state(state, user_extensions_enabled);
            let protocol = probe_if_active(state);
            verify_tracker_protocol(state, readiness, protocol)
        }
        None => (
            false,
            "GNOME Shell is not answering on the session bus".into(),
            None,
        ),
    };
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
        "state": state_label,
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
}
