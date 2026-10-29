//! Named, local configuration profiles.

use super::Args;
use crate::*;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;

/// Manage named offline workspaces as independently validated full libraries.
/// Switching atomically replaces the active primary config; shared fleet
/// layers remain attached to the config directory and are validated normally.
pub(crate) fn profile_command(mut args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand profile list [config]\n       wayexpand profile create <name> [config]\n       wayexpand profile switch <name> [config]\n       wayexpand profile remove <name> [config]\n       wayexpand profile assign-app <app-id> <profile> [config]\n       wayexpand profile unassign-app <app-id> [config]";
    let action = args.next().ok_or_else(|| usage_error(USAGE))?;
    if matches!(action.as_str(), "assign-app" | "unassign-app") {
        let app_id = args.next().ok_or_else(|| usage_error(USAGE))?;
        let profile = (action == "assign-app").then(|| args.next()).flatten();
        if action == "assign-app" && profile.is_none() {
            usage_bail!("{USAGE}");
        }
        let config_path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(default_config_path);
        if args.next().is_some() {
            usage_bail!("{USAGE}");
        }
        update_app_profile_assignment(&config_path, &app_id, profile.as_deref())?;
        return Ok(());
    }
    let name = if action == "list" { None } else { args.next() };
    if action != "list" && name.is_none() {
        usage_bail!("{USAGE}");
    }
    let config_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    if args.next().is_some() {
        usage_bail!("{USAGE}");
    }
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("profiles");
    if action != "list" || root.exists() {
        prepare_profile_root(&root)?;
    }
    let active_path = root.join("active");
    let active = read_profile_active(&active_path)?;
    let assignments_path = root.join("applications.toml");
    let assignments = if action == "list" && !root.exists() {
        wayexpand_core::ProfileAssignments::default()
    } else {
        wayexpand_core::ProfileAssignments::load(&assignments_path)
            .map_err(|error| config_error(error.to_string()))?
    };
    match action.as_str() {
        "list" => {
            if !root.exists() {
                println!("No profiles yet. Create one with `wayexpand profile create personal`.");
                return Ok(());
            }
            for entry in std::fs::read_dir(&root)
                .map_err(|error| config_error(format!("could not list profiles: {error}")))?
            {
                let entry = entry.map_err(|error| {
                    config_error(format!("could not read profile entry: {error}"))
                })?;
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "toml")
                {
                    let Some(profile) = path.file_stem().and_then(|value| value.to_str()) else {
                        continue;
                    };
                    let state = match Config::load(&path) {
                        Ok(config) => match wayexpand_core::FleetConfig::validate_library(
                            config,
                            config_path.parent().unwrap_or_else(|| Path::new(".")),
                        ) {
                            Ok(()) => "valid",
                            Err(_) => "invalid",
                        },
                        Err(_) => "invalid",
                    };
                    println!(
                        "{}{} [{state}]",
                        if active.as_deref() == Some(profile) {
                            "* "
                        } else {
                            "  "
                        },
                        profile
                    );
                }
            }
            for (app_id, profile) in &assignments.applications {
                println!("  {app_id} -> {profile}");
            }
        }
        "create" => {
            let name = name.as_deref().unwrap();
            validate_profile_name(name)?;
            prepare_profile_root(&root)?;
            let target = root.join(format!("{name}.toml"));
            match std::fs::symlink_metadata(&target) {
                Ok(_) => bail!("profile {name} already exists"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(config_error(format!(
                        "could not inspect profile destination: {error}"
                    )))
                }
            }
            let current = Config::load_versioned(&config_path)
                .map_err(|error| config_load_error(&config_path, error))?;
            Config::validate_library_files(&config_path)
                .map_err(|error| config_error(format!("current library is invalid: {error}")))?;
            Config::save_atomic_text(&target, current.source()).map_err(|error| {
                config_error(format!("could not save profile: {}", error.safe_summary()))
            })?;
            write_profile_active(&active_path, name)?;
            println!("created and activated profile {name}");
        }
        "switch" => {
            let name = name.as_deref().unwrap();
            validate_profile_name(name)?;
            let target = root.join(format!("{name}.toml"));
            let selected = Config::load_versioned(&target)
                .map_err(|error| config_load_error(&target, error))?;
            wayexpand_core::FleetConfig::validate_library(
                selected.config.clone(),
                config_path.parent().unwrap_or_else(|| Path::new(".")),
            )
            .map_err(|error| config_error(format!("profile library is invalid: {error}")))?;
            let current = Config::load_versioned(&config_path)
                .map_err(|error| config_load_error(&config_path, error))?;
            if let Some(active_profile) = active.as_deref() {
                let previous = root.join(format!("{active_profile}.toml"));
                if previous != target {
                    Config::save_atomic_text(&previous, current.source()).map_err(|error| {
                        config_error(format!(
                            "could not checkpoint active profile: {}",
                            error.safe_summary()
                        ))
                    })?;
                }
            }
            Config::save_atomic_text(&config_path, selected.source()).map_err(|error| {
                config_error(format!(
                    "could not activate profile: {}",
                    error.safe_summary()
                ))
            })?;
            if let Err(error) = write_profile_active(&active_path, name) {
                let _ = Config::save_atomic_text(&config_path, current.source());
                if let Some(previous_profile) = active.as_deref() {
                    let _ = write_profile_active(&active_path, previous_profile);
                } else {
                    let _ = std::fs::remove_file(&active_path);
                }
                return Err(error);
            }
            let same_as_default = std::fs::canonicalize(&config_path)
                .ok()
                .zip(std::fs::canonicalize(default_config_path()).ok())
                .is_some_and(|(selected, default)| selected == default);
            if same_as_default {
                if let Err(error) = wayexpand_core::DaemonClient::from_environment()
                    .and_then(|client| client.execute(wayexpand_core::DaemonOperation::Reload))
                {
                    eprintln!("warning: profile is active, but the daemon did not confirm a reload: {error}");
                }
            }
            println!("activated profile {name}");
        }
        "remove" => {
            let name = name.as_deref().unwrap();
            validate_profile_name(name)?;
            if active.as_deref() == Some(name) {
                bail!("cannot remove the active profile; switch to another profile first");
            }
            if assignments
                .applications
                .values()
                .any(|profile| profile == name)
            {
                bail!("cannot remove profile {name}; unassign its applications first");
            }
            let target = root.join(format!("{name}.toml"));
            let metadata = std::fs::symlink_metadata(&target)
                .map_err(|error| config_error(format!("could not inspect profile: {error}")))?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                bail!("profile path must be a regular file");
            }
            std::fs::remove_file(&target)
                .map_err(|error| config_error(format!("could not remove profile: {error}")))?;
            println!("removed profile {name}");
        }
        _ => usage_bail!("{USAGE}"),
    }
    Ok(())
}

fn update_app_profile_assignment(
    config_path: &Path,
    app_id: &str,
    profile: Option<&str>,
) -> Result<()> {
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("profiles");
    prepare_profile_root(&root)?;
    let assignments_path = root.join("applications.toml");
    let mut assignments = wayexpand_core::ProfileAssignments::load(&assignments_path)
        .map_err(|error| config_error(error.to_string()))?;
    let normalized_app = wayexpand_core::ProfileAssignments::normalize_application_id(app_id)
        .map_err(|error| config_error(error.to_string()))?;
    if profile.is_some() {
        let name = profile.unwrap();
        validate_profile_name(name)?;
        let profile_path = root.join(format!("{name}.toml"));
        let config = Config::load_versioned(&profile_path)
            .map_err(|error| config_load_error(&profile_path, error))?;
        wayexpand_core::FleetConfig::validate_library(
            config.config,
            config_path.parent().unwrap_or_else(|| Path::new(".")),
        )
        .map_err(|error| config_error(format!("profile library is invalid: {error}")))?;
        assignments
            .applications
            .insert(normalized_app, name.to_owned());
        println!(
            "assigned application {} to profile {name}",
            app_id.to_ascii_lowercase()
        );
    } else {
        if assignments.applications.remove(&normalized_app).is_none() {
            bail!("application {} has no profile assignment", app_id);
        }
        println!(
            "removed profile assignment for {}",
            app_id.to_ascii_lowercase()
        );
    }
    assignments
        .save(&assignments_path)
        .map_err(|error| config_error(error.to_string()))?;
    let using_default_config = std::fs::canonicalize(config_path)
        .ok()
        .zip(std::fs::canonicalize(default_config_path()).ok())
        .is_some_and(|(selected, default)| selected == default);
    if using_default_config {
        if let Err(error) = wayexpand_core::DaemonClient::from_environment()
            .and_then(|client| client.execute(wayexpand_core::DaemonOperation::Reload))
        {
            eprintln!("warning: application profile assignment was saved, but the daemon did not confirm a reload: {error}");
        }
    }
    Ok(())
}

fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 32
        || !name.as_bytes()[0].is_ascii_lowercase()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("profile names must start with a lowercase letter and contain only lowercase letters, digits, or hyphens (max 32 characters)");
    }
    Ok(())
}

fn prepare_profile_root(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)
        .map_err(|error| config_error(format!("could not create profiles directory: {error}")))?;
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|error| config_error(format!("could not inspect profiles directory: {error}")))?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        bail!("profiles path must be a real directory owned by the current user");
    }
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| config_error(format!("could not secure profiles directory: {error}")))?;
    Ok(())
}

fn write_profile_active(path: &Path, name: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| config_error("profile state has no parent"))?;
    let temporary = parent.join(format!(
        ".active-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| config_error(format!("could not create profile state: {error}")))?;
    use std::io::Write;
    file.write_all(name.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| config_error(format!("could not write profile state: {error}")))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| config_error(format!("could not activate profile state: {error}")))?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| config_error(format!("could not sync profiles directory: {error}")))?;
    Ok(())
}

fn read_profile_active(path: &Path) -> Result<Option<String>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(config_error(format!(
                "could not inspect active profile state: {error}"
            )))
        }
    };
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        bail!("active profile state must be a private user-owned regular file");
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| config_error(format!("could not open active profile state: {error}")))?;
    let mut name = String::new();
    file.read_to_string(&mut name)
        .map_err(|error| config_error(format!("could not read active profile state: {error}")))?;
    validate_profile_name(&name)?;
    Ok(Some(name))
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("wayexpand-profile-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn profile_names_are_safe_path_components() {
        for name in ["personal", "customer-42", "work2"] {
            assert!(validate_profile_name(name).is_ok());
        }
        for name in ["", "../work", "Work", "-work", "work.name", "work/name"] {
            assert!(validate_profile_name(name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn active_profile_state_is_private_and_round_trips() {
        let root = temp_dir();
        let path = root.join("active");
        assert_eq!(read_profile_active(&path).unwrap(), None);
        write_profile_active(&path, "personal").unwrap();
        assert_eq!(
            read_profile_active(&path).unwrap().as_deref(),
            Some("personal")
        );
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn active_profile_state_refuses_symlinks() {
        use std::os::unix::fs::symlink;
        let root = temp_dir();
        let target = root.join("target");
        let active = root.join("active");
        std::fs::write(&target, "personal").unwrap();
        symlink(&target, &active).unwrap();
        assert!(read_profile_active(&active).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
