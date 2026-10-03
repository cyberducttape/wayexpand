mod args;
mod backup;
mod build_info;
mod doctor;
mod errors;
mod setup;

use anyhow::{bail, Context, Error, Result};
use backup::{create_backup, default_backup_destination};
use doctor::backends::{
    print_backend_diagnostics, print_backend_selection_explain, session_description,
};
use doctor::broker::print_broker_diagnostics;
use doctor::certification::print_certification;
use doctor::files::{print_config_diagnostics, print_control_socket_diagnostics};
use doctor::json::print_json_diagnostics;
use doctor::policy::{load_policy, print_capabilities_diagnostics, print_policy_diagnostics};
use doctor::status::status_as_json;
use errors::{
    config_error, config_load_error, daemon_error, exit_code_for, normalize_error, usage_error,
};
use setup::{
    configure_setup_backend, libei_portal_candidate, prompt_mode_choice, recommended_setup_backend,
    setup_backend_allowed, setup_backend_for_mode,
};
use std::{
    env, fs,
    io::{self, Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use unicode_segmentation::UnicodeSegmentation;

const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTROL_RESPONSE_BYTES: usize = 4096;
use wayexpand_backend_ibus::engine_available as ibus_engine_available;
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_libei::{portal_token_path, reset_portal_token};
use wayexpand_backend_selection::{
    explain_auto_selection, probe_capabilities, recommended_route, RecommendedRoute,
};
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{
    all_capabilities, default_config_path, discover_backends, import_espanso, import_pack,
    inspect_pack, BackendKind, Config, ExpansionEngine, FleetConfig, InputEvent, MatchMode,
    OrganizationPolicy, CONTROL_STATUS_SCHEMA,
};

use args::{take_json_flag, take_option};

const EXIT_USAGE: i32 = 2;
const EXIT_CONFIG: i32 = 3;
const EXIT_DAEMON: i32 = 4;

macro_rules! usage_bail {
    ($($argument:tt)*) => {
        return Err(usage_error(format!($($argument)*)))
    };
}

fn main() {
    if let Err(error) = run().map_err(normalize_error) {
        eprintln!("Error: {error:?}");
        std::process::exit(exit_code_for(&error));
    }
}

fn run() -> Result<()> {
    // `wayexpand <command> --help` would otherwise reach the command's own
    // parser and be read as a trigger or config path.
    if env::args().skip(2).any(|argument| argument == "--help") {
        print_help();
        return Ok(());
    }
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--version") | Some("-V") | Some("version") => {
            println!(
                "wayexpand {} (commit {})",
                build_info::VERSION,
                build_info::COMMIT
            );
        }
        Some("test") => {
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
        }
        Some("test-hotkey") => {
            let chord_text = args.next().ok_or_else(|| {
                usage_error("usage: wayexpand test-hotkey <chord> [--json] [config]")
            })?;
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
        }
        Some("preview") => {
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
        }
        Some("list") => {
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
                    println!(
                        "{} {}{}",
                        if expansion.enabled { "[on ]" } else { "[off]" },
                        expansion.trigger,
                        if expansion.description.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", expansion.description)
                        }
                    );
                }
            }
        }
        Some("search") => {
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
                        "{} {} {}",
                        expansion.trigger,
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
                    println!(
                        "{} {}{}",
                        if expansion.enabled { "[on ]" } else { "[off]" },
                        expansion.trigger,
                        if expansion.description.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", expansion.description)
                        }
                    );
                }
            }
        }
        Some("validate") => {
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
                let fleet = FleetConfig::load_standard_with_base_and_policy(config, &policy)
                    .map_err(|error| {
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
        }
        Some("import") => {
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
        }
        Some("pack") => {
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
        }
        Some("set-enabled") => {
            let trigger = args.next().ok_or_else(|| {
                usage_error("usage: wayexpand set-enabled <trigger> <on|off> [config]")
            })?;
            let value = args.next().ok_or_else(|| {
                usage_error("usage: wayexpand set-enabled <trigger> <on|off> [config]")
            })?;
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
            let loaded =
                Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
            let expected_revision = loaded.revision;
            let mut config = loaded.config;
            let Some(expansion) = config
                .expansion
                .iter_mut()
                .find(|expansion| expansion.trigger == trigger)
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
        }
        Some("set-mode") => {
            let trigger = args.next().ok_or_else(|| {
                usage_error(
                    "usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]",
                )
            })?;
            let value = args.next().ok_or_else(|| {
                usage_error(
                    "usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]",
                )
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
                usage_bail!(
                    "usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]"
                );
            }
            let loaded =
                Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
            let expected_revision = loaded.revision;
            let mut config = loaded.config;
            let Some(expansion) = config
                .expansion
                .iter_mut()
                .find(|expansion| expansion.trigger == trigger)
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
        }
        Some("backup") => {
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
        }
        Some("edit") => {
            let config = args.next();
            if args.next().is_some() {
                usage_bail!("usage: wayexpand edit [config]");
            }
            let mut editor = Command::new("wayexpand-gui");
            if let Some(config) = config {
                editor.arg(config);
            }
            editor.spawn().context(
                "could not start wayexpand-gui; install the GUI package or run wayexpand-ui",
            )?;
        }
        Some("setup") => {
            let mut requested_backend: Option<String> = None;
            let mut requested_mode: Option<String> = None;
            let mut assume_yes = false;
            while let Some(argument) = args.next() {
                match argument.as_str() {
                    "--experimental-input-method-v2" => {
                        requested_backend = Some("input-method".into());
                    }
                    "--yes" | "-y" => assume_yes = true,
                    "--backend" => {
                        requested_backend = Some(args.next().ok_or_else(|| {
                            usage_error("--backend requires ibus, input-method, or evdev")
                        })?);
                    }
                    "--mode" => {
                        requested_mode = Some(args.next().ok_or_else(|| {
                            usage_error("--mode requires recommended, maximum, or experimental")
                        })?);
                    }
                    value if value.starts_with("--backend=") => {
                        requested_backend = Some(value.trim_start_matches("--backend=").to_owned());
                    }
                    value if value.starts_with("--mode=") => {
                        requested_mode = Some(value.trim_start_matches("--mode=").to_owned());
                    }
                    _ => {
                        usage_bail!(
                            "usage: wayexpand setup [--mode recommended|maximum|experimental] [--yes]"
                        )
                    }
                }
            }
            if requested_backend.is_some() && requested_mode.is_some() {
                usage_bail!("setup accepts either --mode or expert --backend, not both");
            }
            println!("WayExpand setup");
            println!(
                "WayExpand {} (commit {})",
                build_info::VERSION,
                build_info::COMMIT
            );
            println!("Session: {}", session_description());
            println!();
            Config::ensure_user_config(default_config_path()).map_err(|error| {
                config_error(format!(
                    "could not initialize configuration: {}",
                    error.safe_summary()
                ))
            })?;
            let capabilities = probe_capabilities();
            let policy = load_policy()?;
            let automatic = recommended_setup_backend(&capabilities, &policy);
            println!();
            println!("Compatibility modes");
            println!(
                "  Recommended         safest detected path; run doctor/certify for verification"
            );
            println!(
                "  Maximum compatibility broad application coverage; may observe global input"
            );
            println!(
                "  Experimental         protocol paths whose key pass-through is not certified"
            );
            println!();
            println!("Automatic recommendation: {}", automatic.label);
            println!("  {}", automatic.detail);
            let backend = if let Some(backend) = requested_backend {
                backend
            } else if let Some(mode) = requested_mode {
                setup_backend_for_mode(&mode, &capabilities, &policy)?
            } else if assume_yes {
                automatic.backend.to_owned()
            } else {
                let prompt = format!("Configure Recommended mode ({})? [Y/n] ", automatic.label);
                if prompt_yes_no(&prompt, true)? {
                    automatic.backend.to_owned()
                } else {
                    prompt_mode_choice(&capabilities, &policy)?
                }
            };
            if backend == "unavailable" {
                bail!(
                    "Recommended mode found no safe automatic path; use --mode experimental or configure permissions and rerun setup"
                );
            }
            if !setup_backend_allowed(&policy, &backend) {
                bail!("setup backend '{backend}' is disallowed by organization policy")
            }
            configure_setup_backend(&backend)?;
        }
        Some("doctor") => {
            let mut rest: Vec<String> = args.collect();
            let requested_json = take_json_flag(&mut rest);
            if rest.len() > 1 {
                usage_bail!("usage: wayexpand doctor [--json] [config]");
            }
            let config_path = rest
                .into_iter()
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            if requested_json {
                let healthy = print_json_diagnostics(&config_path)?;
                if !healthy {
                    bail!("doctor found configuration, policy, or control-socket problems");
                }
                return Ok(());
            }
            println!(
                "WayExpand {} (commit {})",
                build_info::VERSION,
                build_info::COMMIT
            );
            println!("Session: {}", session_description());
            println!("Feature limits: IME preedit/composition is unsupported; app_filter window tracking is KWin-only (fails closed elsewhere).");
            let config_ok = print_config_diagnostics(&config_path);
            let control_socket_ok = print_control_socket_diagnostics();
            let policy_ok = print_policy_diagnostics();
            let broker_ok = print_broker_diagnostics(Config::load(&config_path).ok().as_ref());
            let capture_ready = print_backend_diagnostics(true);
            print_capabilities_diagnostics();
            if !config_ok || !control_socket_ok || !policy_ok || !broker_ok || !capture_ready {
                bail!("doctor found configuration, runtime, or backend problems");
            }
        }
        Some("certify") => {
            let mut rest: Vec<String> = args.collect();
            let requested_json = take_json_flag(&mut rest);
            if !rest.is_empty() {
                usage_bail!("usage: wayexpand certify [--json]");
            }
            let certified = print_certification(requested_json)?;
            if !certified && !requested_json {
                bail!(
                    "certification is incomplete; see the reported unsupported or untested checks"
                );
            }
        }
        Some("backend") => match args.next().as_deref() {
            None => {
                print_backend_diagnostics(true);
            }
            Some("select") => {
                if args.next().as_deref() != Some("--explain") || args.next().is_some() {
                    usage_bail!("usage: wayexpand backend select --explain");
                }
                print_backend_selection_explain();
            }
            _ => usage_bail!("usage: wayexpand backend [select --explain]"),
        },
        Some("explain-backend") => {
            if args.next().is_some() {
                usage_bail!("usage: wayexpand explain-backend");
            }
            print_backend_selection_explain();
        }
        Some("fleet") => match args.next().as_deref() {
            Some("status") => {
                let mut rest: Vec<String> = args.collect();
                let requested_json = take_json_flag(&mut rest);
                if !rest.is_empty() {
                    usage_bail!("usage: wayexpand fleet status [--json]");
                }
                let path = default_config_path();
                let base = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
                let policy = load_policy()?;
                let fleet = FleetConfig::load_standard_with_base_and_policy(base, &policy)
                    .map_err(|error| {
                        config_error(format!(
                            "fleet configuration invalid: {}",
                            error.safe_summary()
                        ))
                    })?;
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
        },
        Some("portal") => match args.next().as_deref() {
            Some("status") => {
                if args.next().is_some() {
                    usage_bail!("usage: wayexpand portal status|reset");
                }
                let path = portal_token_path().context("cannot determine config directory")?;
                println!(
                    "portal restoration token: {} ({})",
                    if path.is_file() {
                        "present"
                    } else {
                        "not present"
                    },
                    path.display()
                );
            }
            Some("reset") => {
                if args.next().is_some() {
                    usage_bail!("usage: wayexpand portal status|reset");
                }
                let removed =
                    reset_portal_token().context("removing the libei portal restoration token")?;
                println!(
                    "{}",
                    if removed {
                        "removed the stored portal restoration token"
                    } else {
                        "no stored portal restoration token was present"
                    }
                );
                println!("the next libei connection may ask for portal access again");
            }
            _ => usage_bail!("usage: wayexpand portal status|reset"),
        },
        Some(requested @ ("status" | "reload" | "pause" | "resume" | "stop")) => {
            let status_argument = args.next();
            let requested_json = status_argument.as_deref() == Some("--json");
            if status_argument.is_some() && !requested_json {
                usage_bail!("usage: wayexpand {requested} [--json]");
            }
            if args.next().is_some() {
                usage_bail!("usage: wayexpand {requested} [--json]");
            }
            let response = control_request(requested)?;
            if requested_json {
                println!("{}", status_as_json(&response)?);
            } else {
                print!("{response}");
            }
        }
        Some("insert") => {
            let trigger = args
                .next()
                .ok_or_else(|| usage_error("usage: wayexpand insert <trigger>"))?;
            if args.next().is_some() {
                usage_bail!("usage: wayexpand insert <trigger>");
            }
            if trigger.is_empty() || trigger.chars().any(char::is_control) {
                usage_bail!("a trigger cannot be empty or contain control characters");
            }
            let response = control_request(&format!("insert {trigger}"))?;
            if response.trim_end() != "insert scheduled" {
                return Err(daemon_error(format!(
                    "daemon refused the insert: {}",
                    response.trim_end()
                )));
            }
            println!("insert scheduled");
        }
        Some("help") | Some("--help") | Some("-h") | None => print_help(),
        Some(command) => usage_bail!("unknown command {command:?}; try `wayexpand help`"),
    }
    Ok(())
}

/// `(usage, summary)` rows for `wayexpand help`, grouped by section.
const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Getting started",
        &[
            (
                "setup [--mode recommended|maximum|experimental] [--yes]",
                "Configure a safe compatibility mode",
            ),
            (
                "doctor [--json] [config]",
                "Diagnose configuration and backends",
            ),
            ("edit [config]", "Open the graphical snippet editor"),
        ],
    ),
    (
        "Daemon control",
        &[(
            "status|reload|pause|resume|stop [--json]",
            "Control a running daemon",
        )],
    ),
    (
        "Snippets",
        &[
            (
                "list [--json] [config]",
                "List configured expansions and hotkeys",
            ),
            (
                "search <query> [--json] [config]",
                "Search triggers, descriptions, and tags",
            ),
            (
                "test <text> [--json] [config]",
                "Simulate input and print a match",
            ),
            (
                "preview <trigger> [--preview-app APP] [--json] [config]",
                "Preview a replacement",
            ),
            (
                "test-hotkey <chord> [--json] [config]",
                "Resolve a hotkey without executing it",
            ),
            (
                "insert <trigger>",
                "Type a snippet at the cursor via the running daemon",
            ),
            (
                "set-enabled <trigger> <on|off> [config]",
                "Enable or disable an expansion",
            ),
            (
                "set-mode <trigger> <immediate|word-boundary> [config]",
                "Set the matching mode",
            ),
            (
                "validate [--fleet] [--json] [config]",
                "Validate configuration",
            ),
            (
                "backup [config] [destination]",
                "Create a non-overwriting config backup",
            ),
            (
                "import espanso <file>",
                "Convert an Espanso YAML file to TOML on stdout",
            ),
            (
                "pack inspect|import <directory>",
                "Inspect or safely import a local snippet pack",
            ),
        ],
    ),
    (
        "Backends and operations",
        &[
            ("backend", "Show backend availability"),
            ("explain-backend", "Explain automatic backend selection"),
            ("certify [--json]", "Run local compatibility certification"),
            (
                "fleet status [--json]",
                "Show merged fleet configuration status",
            ),
            (
                "portal status|reset",
                "Inspect or remove the libei portal token",
            ),
        ],
    ),
    (
        "Other",
        &[
            ("help", "Show this help"),
            ("version", "Print the installed version"),
        ],
    ),
];

fn help_text() -> String {
    let width = HELP_SECTIONS
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .map(|(usage, _)| usage.chars().count())
        .max()
        .unwrap_or(0);
    let mut help = format!(
        "WayExpand {} — secure Wayland text expansion\n\nusage: wayexpand <command> [options]\n",
        env!("CARGO_PKG_VERSION")
    );
    for (section, rows) in HELP_SECTIONS {
        help.push_str(&format!("\n{section}:\n"));
        for (usage, summary) in *rows {
            help.push_str(&format!("  {usage:<width$}  {summary}\n"));
        }
    }
    help.push_str(&format!(
        "\n[config] defaults to {}\nEnvironment: WAYEXPAND_CONFIG, WAYEXPAND_SOCKET, XDG_CONFIG_HOME, XDG_RUNTIME_DIR",
        default_config_path().display()
    ));
    help
}

/// Send one line to the daemon's control socket and return its reply.
fn control_request(command: &str) -> Result<String> {
    let path = std::env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        })
        .ok_or_else(|| daemon_error("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required"))?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|error| daemon_error(format!("connecting to {}: {error}", path.display())))?;
    stream
        .set_read_timeout(Some(CONTROL_IO_TIMEOUT))
        .map_err(|error| daemon_error(format!("configuring daemon socket: {error}")))?;
    stream
        .set_write_timeout(Some(CONTROL_IO_TIMEOUT))
        .map_err(|error| daemon_error(format!("configuring daemon socket: {error}")))?;
    writeln!(stream, "{command}")
        .map_err(|error| daemon_error(format!("sending daemon command: {error}")))?;
    let mut response = Vec::with_capacity(MAX_CONTROL_RESPONSE_BYTES);
    stream
        .take((MAX_CONTROL_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)
        .map_err(|error| daemon_error(format!("reading daemon response: {error}")))?;
    if response.len() > MAX_CONTROL_RESPONSE_BYTES {
        return Err(daemon_error(format!(
            "daemon control response exceeded {MAX_CONTROL_RESPONSE_BYTES} bytes"
        )));
    }
    let response = String::from_utf8(response)
        .map_err(|_| daemon_error("daemon returned a non-UTF-8 control response"))?;
    Ok(response)
}

fn print_help() {
    println!("{}", help_text());
}

fn prompt_yes_no(prompt: &str, default: bool) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_ascii_lowercase();
    if answer.is_empty() {
        return Ok(default);
    }
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests;
