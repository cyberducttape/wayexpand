//! Review and restore a previously backed-up primary configuration.

use super::Args;
use crate::*;
use std::collections::BTreeMap;

fn expansion_map(config: &Config) -> BTreeMap<&str, &wayexpand_core::ExpansionConfig> {
    config
        .expansion
        .iter()
        .map(|expansion| (expansion.id.as_str(), expansion))
        .collect()
}

pub(crate) fn restore_command(mut args: Args) -> Result<()> {
    let backup_path = args.next().map(PathBuf::from).ok_or_else(|| {
        usage_error("usage: wayexpand restore <backup> [--check | --apply --yes --expected-current=HASH --expected-backup=HASH] [--json] [config]")
    })?;
    let mut apply = false;
    let mut check = false;
    let mut yes = false;
    let mut json_output = false;
    let mut config_path = None;
    let mut expected_current = None;
    let mut expected_backup = None;
    for argument in args {
        match argument.as_str() {
            "--apply" if !apply && !check => apply = true,
            "--check" if !apply && !check => check = true,
            "--yes" | "-y" => yes = true,
            "--json" => json_output = true,
            value if value.starts_with("--expected-current=") => {
                expected_current = Some(value.trim_start_matches("--expected-current=").to_owned())
            }
            value if value.starts_with("--expected-backup=") => {
                expected_backup = Some(value.trim_start_matches("--expected-backup=").to_owned())
            }
            value if !value.starts_with('-') && config_path.is_none() => {
                config_path = Some(PathBuf::from(value))
            }
            _ => usage_bail!(
                "usage: wayexpand restore <backup> [--check | --apply --yes --expected-current=HASH --expected-backup=HASH] [--json] [config]"
            ),
        }
    }
    if yes && !apply {
        usage_bail!("--yes is valid only with --apply")
    }
    if apply && !yes {
        bail!("restoring replaces the active configuration; review with `wayexpand restore {} --check --json`, then use the exact apply command printed by the review", backup_path.display())
    }
    if !apply && (expected_current.is_some() || expected_backup.is_some()) {
        usage_bail!("expected fingerprints are valid only with --apply")
    }
    if apply && (expected_current.is_none() || expected_backup.is_none()) {
        bail!("restore apply requires fingerprints from a prior `wayexpand restore <backup> --check --json` review")
    }
    let config_path = config_path.unwrap_or_else(default_config_path);
    let current = Config::load_versioned(&config_path)
        .map_err(|error| config_load_error(&config_path, error))?;
    let backup = Config::load_versioned(&backup_path)
        .map_err(|error| config_load_error(&backup_path, error))?;
    let current_fingerprint = current.revision.fingerprint();
    let backup_fingerprint = backup.revision.fingerprint();
    if apply
        && (expected_current.as_deref() != Some(current_fingerprint.as_str())
            || expected_backup.as_deref() != Some(backup_fingerprint.as_str()))
    {
        bail!("active configuration or backup changed after review; run `wayexpand restore <backup> --check --json` again")
    }
    let canonical_current = fs::canonicalize(&config_path).ok();
    if canonical_current.is_some() && canonical_current == fs::canonicalize(&backup_path).ok() {
        bail!("restore source is the active configuration; choose a separate backup file")
    }
    backup.config.validate().map_err(|error| {
        config_error(format!(
            "backup configuration is invalid: {}",
            error.safe_summary()
        ))
    })?;
    wayexpand_core::FleetConfig::validate_library(
        backup.config.clone(),
        config_path.parent().unwrap_or_else(|| Path::new(".")),
    )
    .map_err(|error| {
        config_error(format!(
            "restored configuration fails library or policy validation: {}",
            error.safe_summary()
        ))
    })?;

    let before = expansion_map(&current.config);
    let after = expansion_map(&backup.config);
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    for (id, candidate) in &after {
        match before.get(id) {
            None => added.push(candidate.trigger.clone()),
            Some(previous) if *previous != *candidate => modified.push(candidate.trigger.clone()),
            Some(_) => {}
        }
    }
    for (id, previous) in &before {
        if !after.contains_key(id) {
            removed.push(previous.trigger.clone());
        }
    }
    for triggers in [&mut added, &mut removed, &mut modified] {
        triggers.sort();
    }
    let settings_changed = current.config.settings != backup.config.settings;
    let hotkeys_changed = current.config.hotkey != backup.config.hotkey;
    let report = serde_json::json!({
        "schema_version": 1,
        "operation": if apply { "apply" } else { "check" },
        "valid": true,
        "backup": backup_path,
        "target": config_path,
        "current_fingerprint": current_fingerprint,
        "backup_fingerprint": backup_fingerprint,
        "changes": {
            "added_triggers": added,
            "removed_triggers": removed,
            "modified_triggers": modified,
            "settings_changed": settings_changed,
            "hotkeys_changed": hotkeys_changed,
        },
        "current_backup_created_before_apply": apply,
    });
    if json_output {
        println!("{report}");
    } else {
        println!(
            "Configuration restore {}",
            if apply { "apply" } else { "preview" }
        );
        println!(
            "Added triggers: {}",
            report["changes"]["added_triggers"]
                .as_array()
                .map_or(0, Vec::len)
        );
        println!(
            "Removed triggers: {}",
            report["changes"]["removed_triggers"]
                .as_array()
                .map_or(0, Vec::len)
        );
        println!(
            "Modified triggers: {}",
            report["changes"]["modified_triggers"]
                .as_array()
                .map_or(0, Vec::len)
        );
        println!("Settings changed: {settings_changed}; hotkeys changed: {hotkeys_changed}");
        if !apply {
            println!(
                "Apply with: wayexpand restore {} --apply --yes --expected-current={} --expected-backup={}",
                backup_path.display(),
                report["current_fingerprint"].as_str().unwrap_or_default(),
                report["backup_fingerprint"].as_str().unwrap_or_default(),
            );
        }
    }
    if !apply {
        return Ok(());
    }

    let safety_backup = crate::backup::default_backup_destination(&config_path);
    crate::backup::create_backup(&config_path, &safety_backup)?;
    Config::save_atomic_text_if_revision_matches(&config_path, backup.source(), &current.revision)
        .map_err(|error| {
            config_error(format!(
                "could not restore configuration: {}",
                error.safe_summary()
            ))
        })?;
    println!("Created pre-restore backup {}", safety_backup.display());
    if let Ok(client) = wayexpand_core::DaemonClient::from_environment() {
        if let Err(error) = client.execute(wayexpand_core::DaemonOperation::Reload) {
            eprintln!(
                "warning: configuration restored, but the daemon did not confirm reload: {error}"
            );
        }
    }
    println!("Configuration restored and validated. Review `wayexpand status` for the loaded daemon state.");
    Ok(())
}
