//! Snippet library and configuration commands: test, preview, list, search, validate, import, packs, enable/mode edits, backup, edit, and fleet.

use super::Args;
use crate::*;

pub(crate) fn test_command(mut args: Args) -> Result<()> {
    let trigger = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand test <text> [--json] [config]"))?;
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    if rest.len() > 1 {
        usage_bail!("usage: wayexpand test <text> [--json] [config]");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    let mut engine = ExpansionEngine::new(config).map_err(|error| {
        config_error(format!("configuration invalid: {}", error.safe_summary()))
    })?;
    let mut results = engine.process(InputEvent::Text(trigger));
    results.extend(engine.process(InputEvent::EndOfInput));
    if requested_json {
        println!(
            "{}",
            serde_json::json!({
                "matched": !results.is_empty(),
                "results": results.iter().map(|result| serde_json::json!({
                    "trigger_characters": result.trigger.graphemes(true).count(),
                    "erase_characters": result.matched_text.graphemes(true).count(),
                    "replacement_bytes": result.insert.len(),
                    "replacement": result.insert,
                    "cursor_offset": result.cursor_offset,
                })).collect::<Vec<_>>(),
            })
        );
    } else {
        match results.last() {
            Some(result) => println!("{}", result.insert),
            None => println!("no expansion matched"),
        }
    }
    Ok(())
}

pub(crate) fn test_hotkey_command(mut args: Args) -> Result<()> {
    let chord_text = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand test-hotkey <chord> [--json] [config]"))?;
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    if rest.len() > 1 {
        usage_bail!("usage: wayexpand test-hotkey <chord> [--json] [config]");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let chord = wayexpand_core::KeyChord::parse(&chord_text)
        .map_err(|error| config_error(format!("invalid hotkey chord: {error}")))?;
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    let engine = ExpansionEngine::new(config).map_err(|error| {
        config_error(format!("configuration invalid: {}", error.safe_summary()))
    })?;
    let actions = engine.process_key(&chord);
    if requested_json {
        println!(
            "{}",
            serde_json::json!({
                "chord": chord.to_string(),
                "matched": !actions.is_empty(),
                "actions": actions.iter().map(|action| serde_json::json!({
                    "description": action.description,
                    "program": action.command.program,
                    "args": action.command.args,
                })).collect::<Vec<_>>(),
            })
        );
    } else if actions.is_empty() {
        println!("no hotkey matched {}", chord);
    } else {
        for action in actions {
            println!("{} → {}", chord, action.description);
        }
    }
    Ok(())
}

pub(crate) fn preview_command(mut args: Args) -> Result<()> {
    let trigger = args.next().ok_or_else(|| {
            usage_error("usage: wayexpand preview <trigger> [--preview-app APP|--preview-app=APP] [--json] [config]")
        })?;
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let preview_app = take_option(&mut rest, "--preview-app")?;
    if rest.len() > 1 {
        usage_bail!("usage: wayexpand preview <trigger> [--preview-app APP|--preview-app=APP] [--json] [config]");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    let mut engine = ExpansionEngine::new(config).map_err(|error| {
        config_error(format!("configuration invalid: {}", error.safe_summary()))
    })?;
    if let Some(app_id) = preview_app {
        engine.set_current_window(Some(wayexpand_core::WindowContext {
            app_id: Some(app_id),
            title: None,
        }));
    }
    let mut results = engine.process(InputEvent::Text(trigger));
    results.extend(engine.process(InputEvent::EndOfInput));
    match results.last() {
        Some(result) => {
            if requested_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "matched": true,
                        "trigger": result.trigger,
                        "replacement": result.insert,
                        "cursor_offset": result.cursor_offset,
                    })
                );
            } else {
                println!("trigger: {}", result.trigger);
                println!("replacement:");
                println!("{}", result.insert);
            }
        }
        None if requested_json => println!("{}", serde_json::json!({"matched": false})),
        None => println!("no expansion matched"),
    }
    Ok(())
}

/// One human-readable library line: state, trigger, aliases, description.
fn expansion_line(expansion: &wayexpand_core::ExpansionConfig) -> String {
    let mut line = format!(
        "{} {}",
        if expansion.enabled { "[on ]" } else { "[off]" },
        expansion.trigger
    );
    if !expansion.aliases.is_empty() {
        line.push_str(&format!(" (also {})", expansion.aliases.join(", ")));
    }
    if !expansion.description.is_empty() {
        line.push_str(&format!(" — {}", expansion.description));
    }
    line
}

pub(crate) fn list_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    if rest.len() > 1 {
        usage_bail!("usage: wayexpand list [--json] [config]");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    if requested_json {
        println!(
            "{}",
            serde_json::json!({
                "config": path,
                "count": config.expansion.len(),
                "hotkey_count": config.hotkey.len(),
                "expansions": config.expansion,
                "hotkeys": config.hotkey,
            })
        );
    } else {
        println!(
            "{} expansion(s) in {}",
            config.expansion.len(),
            path.display()
        );
        for expansion in config.expansion {
            println!("{}", expansion_line(&expansion));
        }
    }
    Ok(())
}

pub(crate) fn search_command(mut args: Args) -> Result<()> {
    let query = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand search <query> [--json] [config]"))?;
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    if rest.len() > 1 {
        usage_bail!("usage: wayexpand search <query> [--json] [config]");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let query_lower = query.to_lowercase();
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    let matches: Vec<_> = config
        .expansion
        .into_iter()
        .filter(|expansion| {
            format!(
                "{} {} {} {}",
                expansion.trigger,
                expansion.aliases.join(" "),
                expansion.description,
                expansion.tags.join(" ")
            )
            .to_lowercase()
            .contains(&query_lower)
        })
        .collect();
    if requested_json {
        println!(
            "{}",
            serde_json::json!({
                "query": query,
                "config": path,
                "count": matches.len(),
                "expansions": matches,
            })
        );
    } else if matches.is_empty() {
        println!("no expansions matched {query:?}");
    } else {
        for expansion in &matches {
            println!("{}", expansion_line(expansion));
        }
    }
    Ok(())
}

pub(crate) fn validate_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let merged = if let Some(index) = rest.iter().position(|arg| arg == "--fleet") {
        rest.remove(index);
        true
    } else {
        false
    };
    let requested_json = take_json_flag(&mut rest);
    if rest.len() > 1 {
        usage_bail!("usage: wayexpand validate [--fleet] [--json] [config]");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let policy = load_policy()?;
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    let (config, policy_violations) = if merged {
        let fleet =
            FleetConfig::load_standard_with_base_and_policy(config, &policy).map_err(|error| {
                config_error(format!(
                    "fleet configuration invalid: {}",
                    error.safe_summary()
                ))
            })?;
        (fleet.config, fleet.policy_violations)
    } else {
        let mut config = config;
        config
            .apply_administrator_policy(&policy)
            .map_err(|error| {
                config_error(format!(
                    "organization policy rejects configuration: {}",
                    error.safe_summary()
                ))
            })?;
        (config, Vec::new())
    };
    for violation in &policy_violations {
        eprintln!("policy warning: {violation}");
    }
    if config.organization.require_absolute_commands && !config.organization.safe_mode {
        let relative_commands = config
            .expansion
            .iter()
            .filter_map(|expansion| expansion.command.as_ref())
            .chain(config.hotkey.iter().map(|hotkey| &hotkey.command))
            .filter(|command| {
                config
                    .organization
                    .command_path_violation(&command.program)
                    .is_some()
            })
            .count();
        if relative_commands > 0 {
            eprintln!(
                    "warning: audit policy found {relative_commands} command(s) with relative program paths; they are allowed in audit mode"
                );
        }
    }
    if requested_json {
        println!(
            "{}",
            serde_json::json!({
                "valid": true,
                "fleet": merged,
                "expansion_count": config.expansion.len(),
                "hotkey_count": config.hotkey.len(),
                "max_buffer_chars": config.settings.max_buffer_chars,
                "policy_violations": policy_violations,
            })
        );
    } else {
        println!(
            "configuration valid{}: {} expansion(s), buffer limit {}",
            if merged { " (fleet merged)" } else { "" },
            config.expansion.len(),
            config.settings.max_buffer_chars
        );
    }
    Ok(())
}

pub(crate) fn import_command(mut args: Args) -> Result<()> {
    let format = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand import espanso <file>"))?;
    let source = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand import espanso <file>"))?;
    if args.next().is_some() || format != "espanso" {
        usage_bail!("usage: wayexpand import espanso <file>");
    }
    let imported = import_espanso(Path::new(&source))?;
    eprintln!(
        "Espanso migration: {} fully migrated, {} migrated with warnings, {} unsupported",
        imported.report.fully_migrated,
        imported.report.migrated_with_warnings,
        imported.report.unsupported
    );
    for warning in &imported.report.warnings {
        for detail in &warning.details {
            eprintln!("warning: {}: {detail}", warning.trigger);
        }
    }
    for unsupported in &imported.report.unsupported_matches {
        eprintln!(
            "unsupported: {}: {}",
            unsupported.trigger, unsupported.reason
        );
    }
    print!("{}", toml::to_string_pretty(&imported.config)?);
    Ok(())
}

pub(crate) fn pack_command(mut args: Args) -> Result<()> {
    let action = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand pack inspect|import <directory>"))?;
    let path = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand pack inspect|import <directory>"))?;
    if args.next().is_some() {
        usage_bail!("usage: wayexpand pack inspect|import <directory>");
    }
    match action.as_str() {
        "inspect" => {
            let inspection = inspect_pack(&path)?;
            println!(
                "{} {}",
                inspection.manifest.name, inspection.manifest.version
            );
            println!("id: {}", inspection.manifest.id);
            println!("publisher: {}", inspection.manifest.publisher);
            if !inspection.manifest.description.is_empty() {
                println!("description: {}", inspection.manifest.description);
            }
            println!("snippet files: {}", inspection.snippet_files);
            println!("expansions: {}", inspection.expansion_count);
            println!("hotkeys: {}", inspection.hotkey_count);
            println!(
                "commands: {} (disabled on import)",
                inspection.command_count
            );
        }
        "import" => {
            let (inspection, config, disabled_commands) = import_pack(&path)?;
            eprintln!(
                "pack {} {} imported; {} command action(s) disabled by default",
                inspection.manifest.name, inspection.manifest.version, disabled_commands
            );
            print!("{}", toml::to_string_pretty(&config)?);
        }
        _ => usage_bail!("usage: wayexpand pack inspect|import <directory>"),
    }
    Ok(())
}

pub(crate) fn set_enabled_command(mut args: Args) -> Result<()> {
    let trigger = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand set-enabled <trigger> <on|off> [config]"))?;
    let value = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand set-enabled <trigger> <on|off> [config]"))?;
    let enabled = match value.as_str() {
        "on" | "true" | "1" => true,
        "off" | "false" | "0" => false,
        _ => usage_bail!("enabled state must be on or off, not {value:?}"),
    };
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    if args.next().is_some() {
        usage_bail!("usage: wayexpand set-enabled <trigger> <on|off> [config]");
    }
    let loaded = Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    let expected_revision = loaded.revision;
    let mut config = loaded.config;
    let Some(expansion) = config
        .expansion
        .iter_mut()
        .find(|expansion| expansion.answers_to(&trigger))
    else {
        bail!("no expansion found for trigger {trigger:?}");
    };
    expansion.enabled = enabled;
    config.validate().map_err(|error| {
        config_error(format!(
            "configuration invalid after edit: {}",
            error.safe_summary()
        ))
    })?;
    config
        .save_atomic_if_revision_matches(&path, &expected_revision)
        .map_err(|error| {
            config_error(format!(
                "could not save configuration: {}",
                error.safe_summary()
            ))
        })?;
    println!(
        "{} {}",
        if enabled { "enabled" } else { "disabled" },
        trigger
    );
    Ok(())
}

pub(crate) fn set_mode_command(mut args: Args) -> Result<()> {
    let trigger = args.next().ok_or_else(|| {
        usage_error("usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]")
    })?;
    let value = args.next().ok_or_else(|| {
        usage_error("usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]")
    })?;
    let mode = match value.as_str() {
        "immediate" => MatchMode::Immediate,
        "word-boundary" => MatchMode::WordBoundary,
        _ => usage_bail!("match mode must be immediate or word-boundary, not {value:?}"),
    };
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    if args.next().is_some() {
        usage_bail!("usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]");
    }
    let loaded = Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    let expected_revision = loaded.revision;
    let mut config = loaded.config;
    let Some(expansion) = config
        .expansion
        .iter_mut()
        .find(|expansion| expansion.answers_to(&trigger))
    else {
        bail!("no expansion found for trigger {trigger:?}");
    };
    expansion.match_mode = mode;
    config.validate().map_err(|error| {
        config_error(format!(
            "configuration invalid after edit: {}",
            error.safe_summary()
        ))
    })?;
    config
        .save_atomic_if_revision_matches(&path, &expected_revision)
        .map_err(|error| {
            config_error(format!(
                "could not save configuration: {}",
                error.safe_summary()
            ))
        })?;
    println!("{} {}", value, trigger);
    Ok(())
}

pub(crate) fn backup_command(mut args: Args) -> Result<()> {
    let source = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let destination = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_backup_destination(&source));
    if args.next().is_some() {
        usage_bail!("usage: wayexpand backup [config] [destination]");
    }
    create_backup(&source, &destination)?;
    println!("created configuration backup {}", destination.display());
    Ok(())
}

pub(crate) fn edit_command(mut args: Args) -> Result<()> {
    let config = args.next();
    if args.next().is_some() {
        usage_bail!("usage: wayexpand edit [config]");
    }
    let mut editor = Command::new("wayexpand-gui");
    if let Some(config) = config {
        editor.arg(config);
    }
    editor
        .spawn()
        .context("could not start wayexpand-gui; install the GUI package or run wayexpand-ui")?;
    Ok(())
}

pub(crate) fn fleet_command(mut args: Args) -> Result<()> {
    match args.next().as_deref() {
        Some("status") => {
            let mut rest: Vec<String> = args.collect();
            let requested_json = take_json_flag(&mut rest);
            if !rest.is_empty() {
                usage_bail!("usage: wayexpand fleet status [--json]");
            }
            let path = default_config_path();
            let base = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
            let policy = load_policy()?;
            let fleet = FleetConfig::load_standard_with_base_and_policy(base, &policy).map_err(
                |error| {
                    config_error(format!(
                        "fleet configuration invalid: {}",
                        error.safe_summary()
                    ))
                },
            )?;
            for violation in &fleet.policy_violations {
                eprintln!("policy warning: {violation}");
            }
            if requested_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "active": true,
                        "files_loaded": fleet.stats.total_files_loaded,
                        "expansion_count": fleet.stats.total_expansions,
                        "hotkey_count": fleet.stats.total_hotkeys,
                        "layers": fleet.stats.layers_applied,
                        "policy_violations": fleet.policy_violations,
                        "expansions": fleet.config.expansion.iter().map(|expansion| serde_json::json!({
                            "trigger": expansion.trigger,
                            "source": fleet.trigger_source(&expansion.trigger),
                        })).collect::<Vec<_>>(),
                        "hotkeys": fleet.config.hotkey.iter().map(|hotkey| serde_json::json!({
                            "chord": hotkey.chord,
                            "source": fleet.hotkeys_source.get(&hotkey.chord),
                        })).collect::<Vec<_>>(),
                    })
                );
            } else {
                println!("Fleet configuration: active");
                println!("Files loaded: {}", fleet.stats.total_files_loaded);
                println!("Expansions: {}", fleet.stats.total_expansions);
                println!("Hotkeys: {}", fleet.stats.total_hotkeys);
                for violation in &fleet.policy_violations {
                    println!("Policy violation: {violation}");
                }
                for layer in fleet.stats.layers_applied {
                    println!("Layer: {layer}");
                }
            }
        }
        _ => usage_bail!("usage: wayexpand fleet status"),
    };
    Ok(())
}
