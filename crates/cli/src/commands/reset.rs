//! Previewable, scoped reset operations. Snippet libraries are never removed.

use super::Args;
use crate::*;
use serde_json::json;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Integration,
    Services,
    Preferences,
}

impl Scope {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "--integration" => Self::Integration,
            "--services" => Self::Services,
            "--preferences" => Self::Preferences,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Integration => "integration",
            Self::Services => "services",
            Self::Preferences => "preferences",
        }
    }
}

pub(crate) fn reset_command(args: Args) -> Result<()> {
    let mut scope = None;
    let mut apply = false;
    let mut check = false;
    let mut yes = false;
    let mut json_output = false;
    for argument in args {
        match argument.as_str() {
            "--check" => {
                if apply || check {
                    usage_bail!("choose either --check or --apply")
                }
                check = true;
            }
            "--apply" => {
                if check || apply {
                    usage_bail!("choose either --check or --apply")
                }
                apply = true;
            }
            "--yes" | "-y" => yes = true,
            "--json" => json_output = true,
            value if Scope::parse(value).is_some() && scope.is_none() => {
                scope = Scope::parse(value)
            }
            _ => usage_bail!("usage: wayexpand reset --integration|--services|--preferences [--check|--apply --yes] [--json]"),
        }
    }
    let scope = scope.ok_or_else(|| usage_error("choose one reset scope"))?;
    if apply && !yes {
        bail!("reset changes integration state; review `wayexpand reset {} --check`, then pass --apply --yes", scope.name())
    }
    if yes && !apply {
        usage_bail!("--yes is valid only with --apply")
    }
    let inventory = crate::services::ServiceInventory::probe();
    let units: Vec<&str> = crate::services::UserService::ALL
        .into_iter()
        .filter(|service| {
            let state = inventory.state(*service);
            state.installed && (state.enabled || state.active)
        })
        .map(crate::services::UserService::unit)
        .collect();
    let prefs = default_config_path().with_file_name("gui-prefs.toml");
    let prefs_present = match fs::symlink_metadata(&prefs) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("inspecting GUI preferences"),
    };
    let backup = (scope == Scope::Preferences && prefs_present)
        .then(|| crate::backup::default_backup_destination(&prefs));
    let actions = match scope {
        Scope::Integration => vec![
            "Disable and stop installed WayExpand user services".to_owned(),
            "Remove the evdev raw-input consent marker".to_owned(),
            "Remove the saved libei portal restoration token".to_owned(),
            "Keep expansions.toml and all snippet libraries unchanged".to_owned(),
        ],
        Scope::Services => vec![
            "Disable and stop installed WayExpand user services".to_owned(),
            "Keep configuration, snippets, portal authorization, and permissions unchanged"
                .to_owned(),
        ],
        Scope::Preferences => vec![
            "Back up gui-prefs.toml, if it exists".to_owned(),
            "Remove GUI-only display and onboarding preferences".to_owned(),
            "Keep expansions.toml and all snippet libraries unchanged".to_owned(),
        ],
    };
    let report = json!({
        "schema_version": 1,
        "scope": scope.name(),
        "mode": if apply { "apply" } else { "check" },
        "actions": actions,
        "affected_services": units,
        "preferences_backup": backup.as_ref().map(|path| path.display().to_string()),
        "snippet_library_affected": false,
    });
    if json_output {
        println!("{report}");
    } else {
        println!(
            "Reset {} ({})",
            scope.name(),
            if apply { "apply" } else { "preview" }
        );
        for action in report["actions"].as_array().into_iter().flatten() {
            println!("  - {}", action.as_str().unwrap_or(""));
        }
        if !units.is_empty() {
            println!("Affected user services: {}", units.join(", "));
        }
        if let Some(path) = backup.as_ref() {
            println!("Preferences backup: {}", path.display());
        }
        println!("Expansion library: unchanged");
        if !apply {
            println!(
                "Apply with: wayexpand reset --{} --apply --yes",
                scope.name()
            );
        }
    }
    if !apply {
        return Ok(());
    }
    match scope {
        Scope::Services | Scope::Integration => {
            for unit in &units {
                let status = crate::setup::run_setup_command(
                    "systemctl",
                    &["--user", "disable", "--now", unit],
                )?;
                if !status.success() {
                    bail!("systemd could not disable {unit}; inspect `systemctl --user status {unit}` before retrying")
                }
            }
            if scope == Scope::Integration {
                if let Some(marker) = crate::setup::evdev_consent_marker() {
                    remove_private_marker(&marker)?;
                }
                if let Some(marker) = crate::setup::legacy_evdev_consent_marker() {
                    remove_private_marker(&marker)?;
                }
                let removed = crate::reset_portal_token()
                    .context("resetting the saved desktop portal authorization")?;
                println!(
                    "Portal restoration token: {}",
                    if removed { "removed" } else { "not present" }
                );
            }
        }
        Scope::Preferences => {
            if let Some(backup) = backup {
                crate::backup::create_backup(&prefs, &backup)?;
                fs::remove_file(&prefs).with_context(|| {
                    format!("removing GUI preferences after backup {}", backup.display())
                })?;
                println!("Backed up preferences to {}", backup.display());
            } else {
                println!("No GUI preferences file was present.");
            }
        }
    }
    println!("Reset complete. Expansion libraries were not changed.");
    Ok(())
}

fn remove_private_marker(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            fs::remove_file(path)
                .with_context(|| format!("removing consent marker {}", path.display()))?;
        }
        Ok(_) => bail!(
            "refusing to remove non-regular consent marker {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspecting consent marker {}", path.display()))
        }
    }
    Ok(())
}
