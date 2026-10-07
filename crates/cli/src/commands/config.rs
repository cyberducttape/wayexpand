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
            instance_id: None,
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

pub(crate) fn pack_command(args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand pack inspect|import|verify <directory> [--signers FILE]\n       wayexpand pack sign <directory> --key FILE";
    let mut rest: Vec<String> = args.collect();
    let key = take_option(&mut rest, "--key")?;
    let signers_override = take_option(&mut rest, "--signers")?;
    let [action, path] = rest.as_slice() else {
        usage_bail!("{USAGE}");
    };
    let path = PathBuf::from(path);
    let policy = load_policy()
        .map_err(|error| config_error(format!("organization policy is invalid: {error}")))?;
    // An explicit --signers file is the caller's own choice; the policy
    // default must be root-owned to be trusted.
    let policy_signers = signers_override.is_none();
    let signers = signers_override
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&policy.pack_signers_file));
    // Verify a signature when present; require one when policy says so.
    let check_signature = || -> Result<Option<String>> {
        let required = policy.safe_mode && policy.require_signed_packs;
        let verified = if policy_signers {
            wayexpand_core::trusted_signers_file(&signers)
                .and_then(|()| wayexpand_core::verify_pack_signature(&path, &signers))
        } else {
            wayexpand_core::verify_pack_signature(&path, &signers)
        };
        match verified {
            Ok(signer) => Ok(Some(signer)),
            Err(wayexpand_core::PackError::Unsigned) if !required => Ok(None),
            Err(error) if !required && !signers.exists() => {
                eprintln!(
                    "warning: pack signature not verified ({error}); no signers file at {}",
                    signers.display()
                );
                Ok(None)
            }
            Err(error) => Err(config_error(format!("pack rejected: {error}"))),
        }
    };
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
            if let Some(version) = &inspection.manifest.min_wayexpand_version {
                println!("requires WayExpand: {version} or newer");
            }
            println!("snippet files: {}", inspection.snippet_files);
            println!("expansions: {}", inspection.expansion_count);
            println!("hotkeys: {}", inspection.hotkey_count);
            println!(
                "commands: {} (disabled on import)",
                inspection.command_count
            );
            println!(
                "capabilities used: {}",
                if inspection.required_capabilities.is_empty() {
                    "none".to_owned()
                } else {
                    inspection.required_capabilities.join(", ")
                }
            );
            let signature = if !inspection.signed {
                "unsigned".to_owned()
            } else {
                match wayexpand_core::verify_pack_signature(&path, &signers) {
                    Ok(signer) => format!("valid, signed by {signer}"),
                    Err(error) => format!("present but not verified: {error}"),
                }
            };
            println!("signature: {signature}");
        }
        "verify" => {
            let signer = wayexpand_core::verify_pack_signature(&path, &signers)
                .map_err(|error| config_error(format!("pack signature: {error}")))?;
            println!("pack signature is valid; signed by {signer}");
        }
        "sign" => {
            let key =
                key.ok_or_else(|| usage_error("pack sign needs --key FILE (an SSH private key)"))?;
            wayexpand_core::sign_pack(&path, std::path::Path::new(&key))
                .map_err(|error| config_error(format!("could not sign pack: {error}")))?;
            println!(
                "signed {}",
                path.join(wayexpand_core::SIGNATURE_FILE).display()
            );
        }
        "import" => {
            let signer = check_signature()?;
            let (inspection, config, disabled_commands) = import_pack(&path)?;
            eprintln!(
                "pack {} {} imported{}; {} command action(s) disabled by default",
                inspection.manifest.name,
                inspection.manifest.version,
                signer
                    .map(|signer| format!(" (signed by {signer})"))
                    .unwrap_or_default(),
                disabled_commands
            );
            print!("{}", toml::to_string_pretty(&config)?);
        }
        _ => usage_bail!("{USAGE}"),
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

/// `wayexpand stats`: local usage statistics, never sent anywhere.
pub(crate) fn stats_command(args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand stats [--json] [--days N] [--clear] [config]";
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let clear = match rest.iter().position(|arg| arg == "--clear") {
        Some(index) => {
            rest.remove(index);
            true
        }
        None => false,
    };
    let days = match take_option(&mut rest, "--days")? {
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|days| (1..=3650).contains(days))
            .ok_or_else(|| usage_error("--days must be a number from 1 to 3650"))?,
        None => 30,
    };
    if rest.len() > 1 {
        usage_bail!("{USAGE}");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let stats_path = wayexpand_core::usage_stats_path(&path);
    if clear {
        match std::fs::remove_file(&stats_path) {
            Ok(()) => println!("local usage statistics cleared"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                println!("no local usage statistics to clear")
            }
            Err(error) => {
                return Err(config_error(format!(
                    "could not clear {}: {error}",
                    stats_path.display()
                )))
            }
        }
        return Ok(());
    }
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let report = wayexpand_core::UsageStats::load(&stats_path).report(&config, now, days);
    if requested_json {
        println!(
            "{}",
            serde_json::json!({
                "recording": config.settings.usage_stats,
                "file": stats_path,
                "report": report,
            })
        );
        return Ok(());
    }
    if !config.settings.usage_stats {
        println!("Usage recording is off (settings.usage_stats = false).");
    }
    println!("Last {days} days: {} expansions", report.expansions);
    println!(
        "Keystrokes avoided (all time): {}",
        report.keystrokes_avoided
    );
    if !report.top.is_empty() {
        println!("\nMost used:");
        for line in &report.top {
            println!("  {:<24} {}", line.trigger, line.count);
        }
    }
    if !report.unused_90_days.is_empty() {
        println!(
            "\n{} snippet(s) not used in 90 days: {}",
            report.unused_90_days.len(),
            report.unused_90_days.join(", ")
        );
    }
    if !report.trigger_risks.is_empty() {
        println!("\n{} trigger warning(s):", report.trigger_risks.len());
        for risk in &report.trigger_risks {
            println!("  {} — {}", risk.trigger, risk.reason);
        }
    }
    println!(
        "\nStored locally in {}; nothing leaves this machine.",
        stats_path.display()
    );
    Ok(())
}

/// `wayexpand sync`: optional Git synchronization of the library.
pub(crate) fn sync_command(args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand sync [init [--remote URL]|status] [--json] [config]";
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let remote = take_option(&mut rest, "--remote")?;
    let action = match rest.first().map(String::as_str) {
        Some("init") | Some("status") => rest.remove(0),
        _ => "now".to_owned(),
    };
    if rest.len() > 1 || (remote.is_some() && action != "init") {
        usage_bail!("{USAGE}");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    match action.as_str() {
        "init" => {
            let directory = crate::sync::init(&path, remote.as_deref())
                .map_err(|error| config_error(format!("{error:#}")))?;
            if requested_json {
                println!("{}", serde_json::json!({ "initialized": directory }));
            } else {
                println!(
                    "snippet library tracked with Git in {}",
                    directory.display()
                );
            }
        }
        "status" => {
            let status =
                crate::sync::status(&path).map_err(|error| config_error(format!("{error:#}")))?;
            if requested_json {
                println!("{}", serde_json::json!({ "status": status }));
            } else {
                println!("{status}");
            }
        }
        _ => {
            let report =
                crate::sync::sync(&path).map_err(|error| config_error(format!("{error:#}")))?;
            // A running daemon picks up the merged library (it validates
            // before switching, so a bad library never replaces a good one).
            let _ = control_operation(wayexpand_core::DaemonOperation::Reload);
            if requested_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "directory": report.directory,
                        "committed": report.committed,
                        "pulled": report.pulled,
                        "pushed": report.pushed,
                        "remote": report.remote,
                    })
                );
            } else {
                println!(
                    "library synchronized: {}{}{}",
                    if report.committed {
                        "committed local changes; "
                    } else {
                        "no local changes; "
                    },
                    if report.pulled {
                        "pulled remote changes; "
                    } else {
                        ""
                    },
                    match (&report.remote, report.pushed) {
                        (Some(remote), true) => format!("pushed to origin ({remote})"),
                        _ => "no remote configured".to_owned(),
                    }
                );
            }
        }
    }
    Ok(())
}
