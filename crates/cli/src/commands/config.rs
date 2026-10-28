//! Snippet library and configuration commands: test, preview, list, search, validate, import, packs, enable/mode edits, backup, edit, and fleet.

use super::Args;
use crate::args::take_flag;
use crate::*;
use std::os::unix::fs::PermissionsExt;

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
    let form_triggers: Vec<(String, bool)> = config
        .expansion
        .iter()
        .filter(|expansion| {
            wayexpand_core::form_fields(&expansion.replacement)
                .is_ok_and(|fields| !fields.is_empty())
        })
        .map(|expansion| (expansion.trigger.clone(), expansion.enabled))
        .collect();
    let mut engine = ExpansionEngine::new(config).map_err(|error| {
        config_error(format!("configuration invalid: {}", error.safe_summary()))
    })?;
    if let Some(app_id) = preview_app.as_deref() {
        engine.set_current_window(Some(wayexpand_core::WindowContext {
            app_id: Some(app_id.to_owned()),
            title: None,
            instance_id: None,
        }));
    }
    let mut results = engine.process(InputEvent::Text(trigger.clone()));
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
        None => {
            let explanation = engine.explain(&trigger, "unknown");
            let form = explanation
                .snippet
                .as_ref()
                .and_then(|snippet| form_triggers.iter().find(|(trigger, _)| trigger == snippet));
            if let Some((form_trigger, enabled)) = form {
                let state = form_preview_state(&explanation, *enabled, preview_app.is_some());
                if requested_json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "matched": state.matched,
                            "trigger_recognized": true,
                            "trigger": form_trigger,
                            "form": true,
                            "previewable": false,
                            "reason": state.reason,
                        })
                    );
                } else if state.matched {
                    println!("trigger: {form_trigger}");
                    println!(
                        "this snippet opens an interactive form and cannot be rendered by preview"
                    );
                    println!("forms require a verified replacement route; the active route may refuse them safely");
                } else if state.app_filter_blocked {
                    println!("snippet {form_trigger} is not enabled for this application");
                } else {
                    println!("snippet {form_trigger} is disabled");
                }
            } else if requested_json {
                println!("{}", serde_json::json!({"matched": false}));
            } else {
                println!("no expansion matched");
            }
        }
    }
    Ok(())
}

struct FormPreviewState {
    matched: bool,
    app_filter_blocked: bool,
    reason: &'static str,
}

fn form_preview_state(
    explanation: &wayexpand_core::Explanation,
    enabled: bool,
    preview_app_supplied: bool,
) -> FormPreviewState {
    let app_filter_blocked = explanation.checks.iter().any(|check| {
        check.name == "app filter" && check.status == wayexpand_core::CheckStatus::Fail
    });
    let matched = enabled && !app_filter_blocked;
    let reason = if !enabled {
        "snippet is disabled"
    } else if app_filter_blocked && preview_app_supplied {
        "preview application does not satisfy this snippet's app filter"
    } else if app_filter_blocked {
        "snippet has an app filter; pass --preview-app APP to check it"
    } else {
        "interactive forms require a verified replacement route and cannot be rendered by this preview"
    };
    FormPreviewState {
        matched,
        app_filter_blocked,
        reason,
    }
}

#[cfg(test)]
mod preview_form_tests {
    use super::*;

    fn explanation(app_id: Option<&str>) -> wayexpand_core::Explanation {
        let config = Config::parse(
            "[[expansion]]\ntrigger = '.form'\nreplacement = '{{field:summary|required}}'\napp_filter = ['org.example.allowed']\n",
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        if let Some(app_id) = app_id {
            engine.set_current_window(Some(wayexpand_core::WindowContext {
                app_id: Some(app_id.to_owned()),
                title: None,
                instance_id: None,
            }));
        }
        engine.explain(".form", "unknown")
    }

    #[test]
    fn form_preview_respects_the_requested_application_filter() {
        let allowed = form_preview_state(&explanation(Some("org.example.allowed")), true, true);
        assert!(allowed.matched);
        assert!(!allowed.app_filter_blocked);
        assert!(allowed.reason.contains("verified replacement route"));

        let denied = form_preview_state(&explanation(Some("org.example.denied")), true, true);
        assert!(!denied.matched);
        assert!(denied.app_filter_blocked);
        assert!(denied.reason.contains("does not satisfy"));
    }

    #[test]
    fn form_preview_requires_an_application_to_check_a_filter() {
        let state = form_preview_state(&explanation(None), true, false);
        assert!(!state.matched);
        assert!(state.app_filter_blocked);
        assert!(state.reason.contains("pass --preview-app APP"));
    }

    #[test]
    fn disabled_form_remains_unmatched_even_for_an_allowed_application() {
        let state = form_preview_state(&explanation(Some("org.example.allowed")), false, true);
        assert!(!state.matched);
        assert!(!state.app_filter_blocked);
        assert_eq!(state.reason, "snippet is disabled");
    }
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
    let deny_risks = if let Some(index) = rest.iter().position(|arg| arg == "--deny-risks") {
        rest.remove(index);
        true
    } else {
        false
    };
    let requested_json = take_json_flag(&mut rest);
    if rest.len() > 1 {
        usage_bail!("usage: wayexpand validate [--fleet] [--json] [--deny-risks] [config]");
    }
    let path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let policy = load_policy()?;
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    Config::validate_library_files(&path).map_err(|error| {
        config_error(format!(
            "library configuration invalid: {}",
            error.safe_summary()
        ))
    })?;
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
    let risks = wayexpand_core::trigger_risks(&config);
    let command_count = config
        .expansion
        .iter()
        .filter(|expansion| expansion.command.is_some())
        .count()
        + config.hotkey.len();
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
                "trigger_risks": risks,
                "command_count": command_count,
            })
        );
    } else {
        println!(
            "configuration valid{}: {} expansion(s), buffer limit {}",
            if merged { " (fleet merged)" } else { "" },
            config.expansion.len(),
            config.settings.max_buffer_chars
        );
        for risk in &risks {
            println!("trigger risk: {} {}", risk.trigger, risk.reason);
        }
    }
    if deny_risks && !risks.is_empty() {
        return Err(config_error(format!(
            "{} trigger risk(s) found; rename the triggers or drop --deny-risks",
            risks.len()
        )));
    }
    Ok(())
}

/// Print the stable machine-readable configuration shape used by authoring
/// tools. This is intentionally descriptive; Config remains authoritative for
/// defaults, limits, and unknown-field rejection.
pub(crate) fn schema_command(mut args: Args) -> Result<()> {
    if args.next().is_some() {
        usage_bail!("usage: wayexpand schema");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "format": "toml",
            "root": {
                "expansion": { "type": "array", "items": "ExpansionConfig" },
                "hotkey": { "type": "array", "items": "HotkeyConfig" },
                "settings": { "type": "Settings" },
                "organization": { "type": "OrganizationPolicy" }
            },
            "types": {
                "ExpansionConfig": {
                    "required": ["trigger", "replacement"],
                    "fields": ["id", "trigger", "replacement", "description", "tags", "category", "enabled", "match_mode", "aliases", "app_filter", "propagate_case", "command", "output_file"]
                },
                "HotkeyConfig": {
                    "required": ["chord", "command"],
                    "fields": ["chord", "description", "command", "enabled"]
                },
                "Settings": {
                    "fields": ["max_buffer_chars", "undo_chord", "font_scale", "libei_token_persistence", "fcitx5_direct_commit", "template_env", "allow_clipboard", "clipboard_mode", "usage_stats"]
                },
                "CommandConfig": {
                    "fields": ["action", "program", "args", "timeout_ms", "cache_ms", "environment", "pass_env"]
                },
                "OrganizationPolicy": {
                    "fields": ["safe_mode", "disable_commands", "disable_hotkeys", "require_absolute_commands", "disable_title_matching", "allow_weak_app_filters", "require_atomic_replace", "require_exclusive_capture", "minimum_replacement_guarantee", "require_sensitive_focus", "require_verified_insert_target", "max_replacement_size", "allowed_backends", "allowed_packs", "require_signed_packs", "pack_signers_file", "disable_clipboard", "disable_template_env", "audit_prefix"]
                }
            }
        }))?
    );
    Ok(())
}

pub(crate) fn import_command(mut args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand import espanso <file> [--strict] [--report-json]";
    let format = args.next().ok_or_else(|| usage_error(USAGE))?;
    let source = args.next().ok_or_else(|| usage_error(USAGE))?;
    if format != "espanso" {
        usage_bail!("{USAGE}");
    }
    let mut strict = false;
    let mut report_json = false;
    for option in args {
        match option.as_str() {
            "--strict" => strict = true,
            "--report-json" => report_json = true,
            _ => usage_bail!("{USAGE}"),
        }
    }
    let mode = if strict {
        wayexpand_core::EspansoImportMode::Strict
    } else {
        wayexpand_core::EspansoImportMode::Permissive
    };
    let imported = wayexpand_core::import_espanso_with_mode(Path::new(&source), mode)?;
    if report_json {
        // Keep stderr a single JSON document so it can be redirected to a
        // file and parsed; the human-readable report would corrupt it.
        eprintln!("{}", serde_json::to_string(&imported.report)?);
    } else {
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
    }
    print!("{}", toml::to_string_pretty(&imported.config)?);
    Ok(())
}

/// When policy requires signed packs, its signers file is the
/// organization's trust root: a signers file the user chooses would let any
/// self-signed pack through.
fn pack_signers_override_refusal(
    policy: &wayexpand_core::OrganizationPolicy,
    signers_overridden: bool,
) -> Option<String> {
    (signers_overridden && policy.safe_mode && policy.require_signed_packs).then(|| {
        format!(
            "organization policy requires packs signed by a signer in {}; --signers cannot replace it",
            policy.pack_signers_file
        )
    })
}

fn installed_pack_root() -> Result<PathBuf> {
    wayexpand_core::Layer::Pack
        .default_dir()
        .ok_or_else(|| config_error("HOME is required to locate the local pack directory"))
}

fn ensure_pack_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("invalid pack id; use only letters, digits, '.', '_' or '-' ");
    }
    Ok(())
}

fn resolve_pack_path(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_owned();
    }
    let id = path.to_string_lossy();
    if ensure_pack_id(&id).is_ok() {
        if let Ok(root) = installed_pack_root() {
            let installed = root.join(id.as_ref());
            if installed.exists() {
                return installed;
            }
        }
    }
    path.to_owned()
}

fn installed_pack_report() -> Result<Vec<serde_json::Value>> {
    let root = installed_pack_root()?;
    let metadata = match std::fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(config_error(format!(
                "could not read pack directory: {error}"
            )))
        }
    };
    if !metadata.file_type().is_dir() {
        bail!(
            "local pack path is not a real directory: {}",
            root.display()
        );
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&root)
        .map_err(|error| config_error(format!("could not list local packs: {error}")))?
    {
        let entry =
            entry.map_err(|error| config_error(format!("could not list local packs: {error}")))?;
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|error| config_error(format!("could not inspect local pack: {error}")))?;
        if !metadata.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        match inspect_pack(&path) {
            Ok(inspection) => entries.push(serde_json::json!({
                "id": inspection.manifest.id,
                "name": inspection.manifest.name,
                "version": inspection.manifest.version,
                "path": path,
                "review": inspection.manifest.review.map(|review| format!("{} ({})", review.reviewed_by, review.reviewed_at)),
                "expansions": inspection.expansion_count,
                "valid": true,
            })),
            Err(error) => entries.push(serde_json::json!({
                "id": entry.file_name().to_string_lossy(),
                "path": path,
                "valid": false,
                "error": error.to_string(),
            })),
        }
    }
    entries.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(entries)
}

fn remove_installed_pack(id: &str) -> Result<()> {
    ensure_pack_id(id)?;
    let root = installed_pack_root()?;
    let target = root.join(id);
    let metadata = std::fs::symlink_metadata(&target)
        .map_err(|error| config_error(format!("could not find installed pack {id}: {error}")))?;
    if !metadata.file_type().is_dir() {
        bail!(
            "installed pack path is not a real directory: {}",
            target.display()
        );
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        bail!(
            "refusing to remove pack not owned by the current user: {}",
            target.display()
        );
    }
    std::fs::remove_dir_all(&target)
        .map_err(|error| config_error(format!("could not remove installed pack: {error}")))?;
    sync_directory(&root)?;
    Ok(())
}

fn install_local_pack(
    source: &Path,
    inspection: &wayexpand_core::PackInspection,
    replace: bool,
    allow_downgrade: bool,
    trusted_signers: Option<&Path>,
) -> Result<PathBuf> {
    let root = installed_pack_root()?;
    install_local_pack_at(
        source,
        inspection,
        replace,
        allow_downgrade,
        trusted_signers,
        &root,
    )
}

fn install_local_pack_at(
    source: &Path,
    inspection: &wayexpand_core::PackInspection,
    replace: bool,
    allow_downgrade: bool,
    trusted_signers: Option<&Path>,
    root: &Path,
) -> Result<PathBuf> {
    let id = &inspection.manifest.id;
    ensure_pack_id(id)?;
    let parent = root
        .parent()
        .ok_or_else(|| config_error("local pack directory has no parent"))?;
    // The fleet loader validates every parent of an installed TOML file. Keep
    // the application-owned directory private as well as the packs leaf, or
    // a permissive umask could make newly installed packs unloadable.
    prepare_private_directory(parent)?;
    let root_preexisting = std::fs::symlink_metadata(root).is_ok();
    std::fs::create_dir_all(root)
        .map_err(|error| config_error(format!("could not create local pack directory: {error}")))?;
    if !root_preexisting {
        let metadata = std::fs::symlink_metadata(root).map_err(|error| {
            config_error(format!("could not inspect new pack directory: {error}"))
        })?;
        if !metadata.file_type().is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            bail!("new pack directory is not a real directory owned by the current user");
        }
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| config_error(format!("could not secure new pack directory: {error}")),
        )?;
    }
    let root_metadata = std::fs::symlink_metadata(root).map_err(|error| {
        config_error(format!("could not inspect local pack directory: {error}"))
    })?;
    if !root_metadata.file_type().is_dir()
        || root_metadata.file_type().is_symlink()
        || root_metadata.uid() != unsafe { libc::geteuid() }
        || root_metadata.mode() & 0o022 != 0
    {
        bail!(
            "local pack directory must be a real, user-owned, non-writable-by-others directory: {} (dir={}, symlink={}, owner={}, mode={:o}, euid={})",
            root.display(), root_metadata.file_type().is_dir(), root_metadata.file_type().is_symlink(), root_metadata.uid(), root_metadata.mode() & 0o777, unsafe { libc::geteuid() }
        );
    }
    let lock_path = root.join(format!(".install-{id}.lock"));
    let _lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&lock_path)
        .map_err(|error| config_error(format!("another install may be running for {id}, or the install lock is unavailable: {error}")))?;
    let _lock_guard = PackInstallLock(lock_path);

    let staging_root = parent.join(".wayexpand-pack-staging");
    let backup_root = parent.join("wayexpand-pack-backups");
    prepare_private_directory(&staging_root)?;
    prepare_private_directory(&backup_root)?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stage = staging_root.join(format!("{id}-{}-{suffix}", std::process::id()));
    let _stage_guard = StagedPackDirectory(stage.clone());
    copy_pack_to_stage(source, &stage)?;
    let staged = inspect_pack(&stage)
        .map_err(|error| config_error(format!("copied pack failed validation: {error}")))?;
    if staged.manifest != inspection.manifest {
        let _ = std::fs::remove_dir_all(&stage);
        bail!("pack source changed during installation; inspect it again and retry");
    }
    if let Some(signers) = trusted_signers {
        wayexpand_core::verify_pack_signature(&stage, signers).map_err(|error| {
            config_error(format!("copied pack signature did not verify: {error}"))
        })?;
    }

    let target = root.join(id);
    let existing_metadata = std::fs::symlink_metadata(&target);
    let mut backup = None;
    match existing_metadata {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                let _ = std::fs::remove_dir_all(&stage);
                bail!(
                    "refusing to replace non-directory or symlink pack path: {}",
                    target.display()
                );
            }
            if !replace {
                let _ = std::fs::remove_dir_all(&stage);
                bail!("pack {id} is already installed; use `pack install --replace` to update it");
            }
            let current = inspect_pack(&target).map_err(|error| {
                config_error(format!(
                    "installed pack must validate before replacement: {error}"
                ))
            })?;
            let current_version =
                semver::Version::parse(&current.manifest.version).map_err(|error| {
                    config_error(format!("installed pack has invalid version: {error}"))
                })?;
            let incoming_version =
                semver::Version::parse(&inspection.manifest.version).map_err(|error| {
                    config_error(format!("incoming pack has invalid version: {error}"))
                })?;
            if incoming_version < current_version && !allow_downgrade {
                let _ = std::fs::remove_dir_all(&stage);
                bail!("refusing pack downgrade from {} to {}; install a higher version or remove the pack first", current.manifest.version, inspection.manifest.version);
            }
            let backup_path =
                backup_root.join(format!("{id}-{}-{suffix}", current.manifest.version));
            std::fs::rename(&target, &backup_path).map_err(|error| {
                config_error(format!("could not preserve previous pack: {error}"))
            })?;
            backup = Some(backup_path);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(config_error(format!(
                "could not inspect installation target: {error}"
            )));
        }
    }
    if let Err(error) = std::fs::rename(&stage, &target) {
        if let Some(backup) = &backup {
            let _ = std::fs::rename(backup, &target);
        }
        let _ = std::fs::remove_dir_all(&stage);
        return Err(config_error(format!(
            "could not activate installed pack: {error}"
        )));
    }
    sync_directory(root)?;
    Ok(target)
}

struct PackInstallLock(PathBuf);

impl Drop for PackInstallLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct StagedPackDirectory(PathBuf);

impl Drop for StagedPackDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn prepare_private_directory(path: &Path) -> Result<()> {
    let preexisting = std::fs::symlink_metadata(path).is_ok();
    std::fs::create_dir_all(path).map_err(|error| {
        config_error(format!("could not create private pack directory: {error}"))
    })?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| config_error(format!("could not inspect pack directory: {error}")))?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || (preexisting && metadata.mode() & 0o022 != 0)
    {
        bail!(
            "pack staging/backup directory must be user-owned and not writable by others: {}",
            path.display()
        );
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|error| {
        config_error(format!("could not secure pack staging directory: {error}"))
    })?;
    Ok(())
}

fn copy_pack_to_stage(source: &Path, stage: &Path) -> Result<()> {
    std::fs::create_dir(stage).map_err(|error| {
        config_error(format!("could not create pack staging directory: {error}"))
    })?;
    std::fs::set_permissions(stage, std::fs::Permissions::from_mode(0o700)).map_err(|error| {
        config_error(format!("could not secure pack staging directory: {error}"))
    })?;
    copy_pack_file(
        &source.join("wayexpand-pack.toml"),
        &stage.join("wayexpand-pack.toml"),
    )?;
    let signature = source.join(wayexpand_core::SIGNATURE_FILE);
    if std::fs::symlink_metadata(&signature).is_ok() {
        copy_pack_file(&signature, &stage.join(wayexpand_core::SIGNATURE_FILE))?;
    }
    let source_snippets = source.join("snippets");
    let metadata = std::fs::symlink_metadata(&source_snippets)
        .map_err(|error| config_error(format!("could not inspect pack snippets: {error}")))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!("pack snippets path is not a real directory");
    }
    let target_snippets = stage.join("snippets");
    std::fs::create_dir(&target_snippets).map_err(|error| {
        config_error(format!(
            "could not create staged snippets directory: {error}"
        ))
    })?;
    std::fs::set_permissions(&target_snippets, std::fs::Permissions::from_mode(0o700)).map_err(
        |error| {
            config_error(format!(
                "could not secure staged snippets directory: {error}"
            ))
        },
    )?;
    for entry in std::fs::read_dir(&source_snippets)
        .map_err(|error| config_error(format!("could not read pack snippets: {error}")))?
    {
        let entry =
            entry.map_err(|error| config_error(format!("could not read pack snippet: {error}")))?;
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            copy_pack_file(&entry.path(), &target_snippets.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn copy_pack_file(source: &Path, target: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|error| config_error(format!("could not inspect pack file: {error}")))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!(
            "pack installation accepts regular files only: {}",
            source.display()
        );
    }
    let mut input = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(source)
        .map_err(|error| config_error(format!("could not open pack file: {error}")))?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)
        .map_err(|error| config_error(format!("could not create staged pack file: {error}")))?;
    std::io::copy(&mut input, &mut output)
        .map_err(|error| config_error(format!("could not copy pack file: {error}")))?;
    output
        .sync_all()
        .map_err(|error| config_error(format!("could not sync staged pack file: {error}")))?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| config_error(format!("could not sync pack directory: {error}")))
}

pub(crate) fn pack_command(args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand pack inspect|import|verify <directory-or-id> [--signers FILE]\n       wayexpand pack install <directory> [--replace [--allow-downgrade]] [--allow-actions] [--signers FILE]\n       wayexpand pack list [--json] | path <id> | remove <id>\n       wayexpand pack sign <directory> --key FILE";
    let mut rest: Vec<String> = args.collect();
    let key = take_option(&mut rest, "--key")?;
    let signers_override = take_option(&mut rest, "--signers")?;
    let replace = take_flag(&mut rest, "--replace");
    let allow_downgrade = take_flag(&mut rest, "--allow-downgrade");
    let allow_actions = take_flag(&mut rest, "--allow-actions");
    let requested_json = take_json_flag(&mut rest);
    // `list` takes no positional path; normalize it to the shared two-field
    // parser shape used by the other pack actions.
    if rest.as_slice() == ["list"] {
        rest.push(String::new());
    }
    let [action, path] = rest.as_slice() else {
        usage_bail!("{USAGE}");
    };
    if replace && action != "install" {
        usage_bail!("--replace is only valid for `wayexpand pack install`");
    }
    if allow_downgrade && (action != "install" || !replace) {
        usage_bail!("--allow-downgrade requires `wayexpand pack install --replace`");
    }
    if allow_actions && action != "install" {
        usage_bail!("--allow-actions is only valid for `wayexpand pack install`");
    }
    if requested_json && action != "list" {
        usage_bail!("--json is only supported by `wayexpand pack list`");
    }
    let requested_path = PathBuf::from(path);
    let path = match action.as_str() {
        "inspect" | "verify" | "sign" | "import" | "install" => resolve_pack_path(&requested_path),
        _ => requested_path.clone(),
    };
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
        if let Some(refusal) = pack_signers_override_refusal(&policy, !policy_signers) {
            return Err(config_error(refusal));
        }
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
        "list" => {
            let packs = installed_pack_report()?;
            if requested_json {
                println!("{}", serde_json::json!({ "packs": packs }));
            } else if packs.is_empty() {
                println!("No local packs installed. Install one with `wayexpand pack install ./packs/<name>`. ");
            } else {
                for pack in packs {
                    println!(
                        "{} {} — {} [{}]",
                        pack["id"].as_str().unwrap_or("unknown"),
                        pack["version"].as_str().unwrap_or("invalid"),
                        pack["name"].as_str().unwrap_or("Invalid pack"),
                        pack["review"].as_str().unwrap_or("unreviewed")
                    );
                }
                println!("Edit local TOML under the pack path; use `wayexpand validate --fleet` after changes.");
            }
        }
        "path" => {
            let id = path.to_string_lossy();
            ensure_pack_id(&id)?;
            let target = installed_pack_root()?.join(id.as_ref());
            inspect_pack(&target)
                .map_err(|error| config_error(format!("installed pack is invalid: {error}")))?;
            println!("{}", target.display());
        }
        "remove" => {
            let id = path.to_string_lossy();
            ensure_pack_id(&id)?;
            remove_installed_pack(&id)?;
            println!("removed local pack {id}");
            let _ = control_operation(wayexpand_core::DaemonOperation::Reload);
        }
        "install" => {
            let inspection = inspect_pack(&path)?;
            let signer = check_signature()?;
            if !allow_actions && (inspection.command_count > 0 || inspection.hotkey_count > 0) {
                bail!("refusing to install command or hotkey actions by default; review the pack and pass --allow-actions to activate them");
            }
            if policy.safe_mode && !policy.pack_allowed(&inspection.manifest.id) {
                bail!(
                    "organization policy does not allow pack {}; ask an administrator to add it to allowed_packs",
                    inspection.manifest.id
                );
            }
            if policy.safe_mode && policy.disable_commands && inspection.command_count > 0 {
                bail!("organization policy disables commands; this pack contains command-backed entries");
            }
            if policy.safe_mode && policy.disable_hotkeys && inspection.hotkey_count > 0 {
                bail!("organization policy disables hotkeys; this pack contains hotkeys");
            }
            let installed = install_local_pack(
                &path,
                &inspection,
                replace,
                allow_downgrade,
                signer.is_some().then_some(&signers),
            )?;
            println!(
                "installed {} {} at {}{}",
                inspection.manifest.name,
                inspection.manifest.version,
                installed.display(),
                if signer.is_some() {
                    " (signature verified)"
                } else {
                    " (local pack; signature not verified)"
                }
            );
            println!("The pack is offline and editable. Run `wayexpand validate --fleet` to check the active library.");
            let _ = control_operation(wayexpand_core::DaemonOperation::Reload);
        }
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
            if let Some(review) = &inspection.manifest.review {
                println!(
                    "review record: {} on {} — {} (metadata; signature authenticates pack bytes)",
                    review.reviewed_by, review.reviewed_at, review.summary
                );
            } else {
                println!("review record: none");
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
        let daemon_result = wayexpand_core::DaemonClient::from_environment().and_then(|client| {
            client.execute(wayexpand_core::DaemonOperation::ClearUsageStats {
                path: stats_path.clone(),
            })
        });
        match daemon_result {
            Ok(response) => {
                if response.contains("does not match daemon configuration")
                    || response.starts_with("unknown command")
                {
                    // This CLI invocation targets a different library from
                    // the running daemon, or the daemon predates stats-clear;
                    // clear the requested file directly under its lock below.
                } else {
                    if response.starts_with("could not clear") || response.contains("unavailable") {
                        return Err(config_error(response.trim().to_string()));
                    }
                    println!("{}", response.trim());
                    return Ok(());
                }
            }
            Err(wayexpand_core::DaemonClientError::Connect { source, .. })
                if matches!(
                    source.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) => {}
            Err(wayexpand_core::DaemonClientError::SocketPathMissing) => {}
            Err(error) => return Err(daemon_error(error.to_string())),
        }
        match wayexpand_core::UsageStats::clear(&stats_path) {
            Ok(true) => println!("local usage statistics cleared"),
            Ok(false) => println!("no local usage statistics to clear"),
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
    let stats = wayexpand_core::UsageStats::load(&stats_path).map_err(|error| {
        config_error(format!(
            "could not load usage statistics at {}: {error}; the file was preserved. Use `wayexpand stats --clear` only if you intend to discard it",
            stats_path.display()
        ))
    })?;
    let report = stats.report(&config, now, days);
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
    println!(
        "Last {days} {}: {} {}",
        if days == 1 { "day" } else { "days" },
        report.expansions,
        if report.expansions == 1 {
            "expansion"
        } else {
            "expansions"
        }
    );
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
    const USAGE: &str = "usage: wayexpand sync [init [--remote URL]|status] [--json] [--allow-executable --approved-revision FULL_COMMIT] [config]";
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let remote = take_option(&mut rest, "--remote")?;
    let allow_executable = crate::args::take_flag(&mut rest, "--allow-executable");
    let approved_revision = take_option(&mut rest, "--approved-revision")?;
    if approved_revision.is_some() && !allow_executable {
        usage_bail!("--approved-revision requires --allow-executable; {USAGE}");
    }
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
            let report = crate::sync::sync_with_policy(
                &path,
                allow_executable,
                approved_revision.as_deref(),
            )
            .map_err(|error| config_error(format!("{error:#}")))?;
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

#[cfg(test)]
mod pack_policy_tests {
    use super::pack_signers_override_refusal;

    #[test]
    fn required_pack_signatures_refuse_a_user_chosen_signers_file() {
        let mut policy = wayexpand_core::OrganizationPolicy {
            safe_mode: true,
            require_signed_packs: true,
            ..Default::default()
        };
        assert!(pack_signers_override_refusal(&policy, true).is_some());
        assert!(pack_signers_override_refusal(&policy, false).is_none());
        // Without the requirement, an explicit signers file is the user's
        // own verification choice.
        policy.require_signed_packs = false;
        assert!(pack_signers_override_refusal(&policy, true).is_none());
        policy.require_signed_packs = true;
        policy.safe_mode = false;
        assert!(pack_signers_override_refusal(&policy, true).is_none());
    }
}

#[cfg(test)]
mod pack_install_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("wayexpand-{label}-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn make_pack(path: &Path, version: &str) -> wayexpand_core::PackInspection {
        std::fs::create_dir_all(path.join("snippets")).unwrap();
        std::fs::write(
            path.join("wayexpand-pack.toml"),
            format!(
                "format_version=1\nid=\"sample.local\"\nversion=\"{version}\"\nname=\"Local sample\"\npublisher=\"Test\"\n\n[review]\nreviewed_by=\"Test\"\nreviewed_at=\"2026-10-27\"\nsummary=\"Insert-only test content\"\n"
            ),
        )
        .unwrap();
        std::fs::write(
            path.join("snippets/main.toml"),
            "[[expansion]]\ntrigger=\";;local-pack-test\"\nreplacement=\"offline\"\n",
        )
        .unwrap();
        inspect_pack(path).unwrap()
    }

    #[test]
    fn local_pack_install_validates_versions_and_keeps_replacement_backup() {
        let base = temp_dir("pack-install");
        let source_v1 = base.join("source-v1");
        let source_v2 = base.join("source-v2");
        let root = base.join("packs");
        let first = make_pack(&source_v1, "1.0.0");
        let first_path =
            install_local_pack_at(&source_v1, &first, false, false, None, &root).unwrap();
        assert_eq!(inspect_pack(&first_path).unwrap().manifest.version, "1.0.0");
        assert!(install_local_pack_at(&source_v1, &first, false, false, None, &root).is_err());

        let second = make_pack(&source_v2, "1.1.0");
        let second_path =
            install_local_pack_at(&source_v2, &second, true, false, None, &root).unwrap();
        assert_eq!(
            inspect_pack(&second_path).unwrap().manifest.version,
            "1.1.0"
        );
        let backups = base.join("wayexpand-pack-backups");
        assert!(std::fs::read_dir(backups).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("sample.local-1.0.0-")
        }));
        assert!(install_local_pack_at(&source_v1, &first, true, false, None, &root).is_err());
        let restored = install_local_pack_at(&source_v1, &first, true, true, None, &root).unwrap();
        assert_eq!(inspect_pack(restored).unwrap().manifest.version, "1.0.0");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_pack_install_refuses_a_symlinked_install_root() {
        use std::os::unix::fs::symlink;

        let base = temp_dir("pack-symlink");
        let source = base.join("source");
        let outside = base.join("outside");
        let root = base.join("packs");
        let inspection = make_pack(&source, "1.0.0");
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, &root).unwrap();
        assert!(install_local_pack_at(&source, &inspection, false, false, None, &root).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}
