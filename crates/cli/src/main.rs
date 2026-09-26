mod args;
mod build_info;

use anyhow::{bail, Context, Error, Result};
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
use wayexpand_backend_selection::{explain_auto_selection, probe_capabilities};
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{
    all_capabilities, default_config_path, discover_backends, import_espanso, BackendKind, Config,
    ExpansionEngine, FleetConfig, InputEvent, MatchMode, OrganizationPolicy,
};

use args::{take_json_flag, take_option};

const EXIT_USAGE: i32 = 2;
const EXIT_CONFIG: i32 = 3;
const EXIT_DAEMON: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliErrorKind {
    Usage,
    Config,
    Daemon,
    Operational,
}

#[derive(Debug)]
struct CliError {
    kind: CliErrorKind,
    message: String,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for CliError {}

fn classified_error(kind: CliErrorKind, message: impl Into<String>) -> Error {
    Error::new(CliError {
        kind,
        message: message.into(),
    })
}

fn usage_error(message: impl Into<String>) -> Error {
    classified_error(CliErrorKind::Usage, message)
}

fn config_error(message: impl Into<String>) -> Error {
    classified_error(CliErrorKind::Config, message)
}

fn daemon_error(message: impl Into<String>) -> Error {
    classified_error(CliErrorKind::Daemon, message)
}

macro_rules! usage_bail {
    ($($argument:tt)*) => {
        return Err(usage_error(format!($($argument)*)))
    };
}

fn exit_code_for(error: &anyhow::Error) -> i32 {
    match error.downcast_ref::<CliError>().map(|error| error.kind) {
        Some(CliErrorKind::Usage) => EXIT_USAGE,
        Some(CliErrorKind::Config) => EXIT_CONFIG,
        Some(CliErrorKind::Daemon) => EXIT_DAEMON,
        Some(CliErrorKind::Operational) | None => 1,
    }
}

fn main() {
    if let Err(error) = run().map_err(normalize_error) {
        eprintln!("Error: {error:?}");
        std::process::exit(exit_code_for(&error));
    }
}

fn normalize_error(error: Error) -> Error {
    if error.downcast_ref::<CliError>().is_some() {
        error
    } else {
        classified_error(CliErrorKind::Operational, error.to_string())
    }
}

fn run() -> Result<()> {
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
            let config = Config::load(path).map_err(|error| {
                config_error(format!("configuration invalid: {}", error.safe_summary()))
            })?;
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
            let config = Config::load(&path).map_err(|error| {
                config_error(format!("configuration invalid: {}", error.safe_summary()))
            })?;
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
            let config = Config::load(&path).map_err(|error| {
                config_error(format!("configuration invalid: {}", error.safe_summary()))
            })?;
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
            let config = Config::load(&path).map_err(|error| {
                config_error(format!("configuration invalid: {}", error.safe_summary()))
            })?;
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
            let config = Config::load(&path).map_err(|error| {
                config_error(format!("configuration invalid: {}", error.safe_summary()))
            })?;
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
            let config = Config::load(&path).map_err(|error| {
                config_error(format!("configuration invalid: {}", error.safe_summary()))
            })?;
            let (config, policy_violations) = if merged {
                let fleet = FleetConfig::load_standard_with_base_and_policy(config, &policy)
                    .map_err(|error| {
                        config_error(format!("fleet configuration invalid: {error}"))
                    })?;
                (fleet.config, fleet.policy_violations)
            } else {
                let mut config = config;
                config
                    .apply_administrator_policy(&policy)
                    .map_err(|error| {
                        config_error(format!(
                            "organization policy rejects configuration: {error}"
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
            if imported.skipped > 0 {
                eprintln!(
                    "warning: skipped {} Espanso match(es) without a string replacement",
                    imported.skipped
                );
            }
            print!("{}", toml::to_string_pretty(&imported.config)?);
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
                _ => bail!("enabled state must be on or off"),
            };
            let path = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            if args.next().is_some() {
                usage_bail!("usage: wayexpand set-enabled <trigger> <on|off> [config]");
            }
            let mut config = Config::load(&path).map_err(|error| {
                config_error(format!("configuration invalid: {}", error.safe_summary()))
            })?;
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
            config.save_atomic(&path).map_err(|error| {
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
                _ => bail!("match mode must be immediate or word-boundary"),
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
            let mut config = Config::load(&path).map_err(|error| {
                config_error(format!("configuration invalid: {}", error.safe_summary()))
            })?;
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
            config.save_atomic(&path).map_err(|error| {
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
            let destination = args.next().map(PathBuf::from).unwrap_or_else(|| {
                let mut path = source.clone();
                path.set_extension("toml.bak");
                path
            });
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
            let config_ok = print_config_diagnostics(&config_path);
            let control_socket_ok = print_control_socket_diagnostics();
            let policy_ok = print_policy_diagnostics();
            let capture_ready = print_backend_diagnostics(true);
            print_capabilities_diagnostics();
            if !config_ok || !control_socket_ok || !policy_ok || !capture_ready {
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
                let base = Config::load(default_config_path()).map_err(|error| {
                    config_error(format!("configuration invalid: {}", error.safe_summary()))
                })?;
                let policy = load_policy()?;
                let fleet = FleetConfig::load_standard_with_base_and_policy(base, &policy)
                    .map_err(|error| {
                        config_error(format!("fleet configuration invalid: {error}"))
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
            let path = std::env::var_os("WAYEXPAND_SOCKET")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("XDG_RUNTIME_DIR")
                        .map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
                })
                .ok_or_else(|| daemon_error("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required"))?;
            let mut stream = UnixStream::connect(&path).map_err(|error| {
                daemon_error(format!("connecting to {}: {error}", path.display()))
            })?;
            stream
                .set_read_timeout(Some(CONTROL_IO_TIMEOUT))
                .map_err(|error| daemon_error(format!("configuring daemon socket: {error}")))?;
            stream
                .set_write_timeout(Some(CONTROL_IO_TIMEOUT))
                .map_err(|error| daemon_error(format!("configuring daemon socket: {error}")))?;
            writeln!(stream, "{requested}")
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
            if requested_json {
                println!("{}", status_as_json(&response)?);
            } else {
                print!("{response}");
            }
        }
        Some("help") | Some("--help") | Some("-h") | None => print_help(),
        Some(command) => usage_bail!("unknown command {command:?}; try `wayexpand help`"),
    }
    Ok(())
}

fn session_description() -> &'static str {
    match env::var("XDG_SESSION_TYPE").ok().as_deref() {
        Some("wayland") if env::var_os("DISPLAY").is_some() => "Wayland (XWayland available)",
        Some("wayland") => "Wayland",
        Some("x11") => "X11",
        _ if env::var_os("WAYLAND_DISPLAY").is_some() => "Wayland",
        _ if env::var_os("DISPLAY").is_some() => "X11 or XWayland",
        _ => "not detected",
    }
}

fn create_backup(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::metadata(source)
        .with_context(|| format!("reading configuration {}", source.display()))?;
    if !metadata.is_file() {
        bail!("configuration is not a regular file: {}", source.display());
    }

    let mut source_file = fs::File::open(source)
        .with_context(|| format!("opening configuration {}", source.display()))?;
    let mut destination_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .with_context(|| format!("creating configuration backup {}", destination.display()))?;
    if let Err(error) = io::copy(&mut source_file, &mut destination_file) {
        let _ = fs::remove_file(destination);
        return Err(error)
            .with_context(|| format!("writing configuration backup {}", destination.display()));
    }
    Ok(())
}

fn print_help() {
    let help = format!(
        "WayExpand {} — secure Wayland text expansion\n\nusage: wayexpand <command> [options]\n\ncommands:\n  setup [--mode recommended|maximum|experimental] [--yes] Configure a safe compatibility mode\n  status|reload|pause|resume|stop [--json]                 Control a running daemon\n  edit [config]                                            Open the graphical snippet editor\n  doctor [--json] [config]                                 Diagnose configuration and backends\n  certify [--json]                                         Run local compatibility certification\n  test <text> [--json] [config]                            Simulate input and print a match\n  test-hotkey <chord> [--json] [config]                   Resolve a hotkey without executing it\n  preview <trigger> [--json] [config]                      Preview a replacement\n  list [--json] [config]                                   List configured expansions and hotkeys\n  search <query> [--json] [config]                         Search triggers, descriptions, and tags\n  validate [config]                                        Validate configuration\n  import espanso <file>                                    Import an Espanso YAML file\n  set-enabled <trigger> <on|off> [config]                 Enable or disable an expansion\n  set-mode <trigger> <mode> [config]                       Set immediate or word-boundary matching\n  backup [config] [destination]                            Create a non-overwriting config backup\n  backend                                                  Show backend availability\n  explain-backend                                          Explain expert backend selection\n  help                                                     Show this help\n  version                                                  Print the installed version\n\nEnvironment: WAYEXPAND_CONFIG, WAYEXPAND_SOCKET, XDG_CONFIG_HOME, XDG_RUNTIME_DIR\nDefault config: {}",
        env!("CARGO_PKG_VERSION"),
        default_config_path().display()
    );
    let help = help
        .replace(
            "preview <trigger> [--json] [config]",
            "preview <trigger> [--preview-app APP] [--json] [config]",
        )
        .replace("validate [config]", "validate [--fleet] [--json] [config]");
    println!("{help}");
    println!(
        "\nOperational commands:\n  fleet status [--json]                                    Show merged fleet configuration status\n  portal status|reset                                      Inspect or remove the libei portal token"
    );
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

struct SetupRecommendation {
    backend: &'static str,
    label: &'static str,
    detail: &'static str,
}

fn recommended_setup_backend(
    capabilities: &wayexpand_backend_selection::Capabilities,
    policy: &OrganizationPolicy,
) -> SetupRecommendation {
    if ibus_engine_available() && setup_backend_allowed(policy, "ibus") {
        return SetupRecommendation {
            backend: "ibus",
            label: "IBus",
            detail: "toolkit-aware committed text with password/PIN purpose support; no raw keyboard access",
        };
    }
    // The packaged maximum-compatibility service is evdev + libei. A
    // wlroots virtual-keyboard probe alone is not enough to claim that the
    // service it will enable can start.
    if capabilities.has_dev_input
        && capabilities.has_direct_libei_socket
        && setup_backend_allowed(policy, "evdev")
    {
        return SetupRecommendation {
            backend: "evdev",
            label: "Maximum compatibility",
            detail:
                "evdev capture with a detected output path; password-field awareness is unavailable",
        };
    }
    SetupRecommendation {
        backend: "unavailable",
        label: "No safe automatic path",
        detail: "setup will not enable an experimental or globally observing path automatically",
    }
}

fn setup_backend_for_mode(
    mode: &str,
    capabilities: &wayexpand_backend_selection::Capabilities,
    policy: &OrganizationPolicy,
) -> Result<String> {
    match mode {
        "recommended" => Ok(recommended_setup_backend(capabilities, policy)
            .backend
            .to_owned()),
        "maximum" => {
            if !capabilities.has_dev_input {
                bail!("Maximum compatibility requires a readable /dev/input keyboard")
            }
            if !capabilities.has_direct_libei_socket && !libei_portal_candidate() {
                bail!(
                    "Maximum compatibility requires a detected libei/EIS path or a KDE/GNOME portal candidate"
                )
            }
            if !setup_backend_allowed(policy, "evdev") {
                bail!("Maximum compatibility is disallowed by organization policy")
            }
            Ok("evdev".into())
        }
        "experimental" => {
            if !capabilities.has_input_method_v2 {
                bail!(
                    "Experimental mode requires an available input-method-v2 compositor interface"
                )
            }
            if !setup_backend_allowed(policy, "input-method") {
                bail!("Experimental input-method-v2 mode is disallowed by organization policy")
            }
            Ok("input-method".into())
        }
        other => bail!(
            "unknown compatibility mode {other:?}; choose recommended, maximum, or experimental"
        ),
    }
}

fn setup_backend_allowed(policy: &OrganizationPolicy, backend: &str) -> bool {
    match backend {
        "ibus" | "input-method" => policy.backend_allowed("input-method-v2"),
        // The packaged setup path enables wayexpand-evdev.service, whose
        // declared output is evdev + libei. Do not treat a wlroots-only
        // policy as permission to activate that different service.
        "evdev" => policy.backend_allowed("libei"),
        _ => false,
    }
}

fn libei_portal_candidate() -> bool {
    if std::env::var_os("LIBEI_SOCKET").is_some() {
        return false;
    }
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .split(':')
        .any(|desktop| desktop.contains("kde") || desktop.contains("gnome"))
}

fn prompt_mode_choice(
    capabilities: &wayexpand_backend_selection::Capabilities,
    policy: &OrganizationPolicy,
) -> Result<String> {
    print!("Choose a mode [recommended/maximum/experimental/q]: ");
    io::stdout().flush()?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;
    let choice = choice.trim().to_ascii_lowercase();
    match choice.as_str() {
        "recommended" | "maximum" | "experimental" => {
            setup_backend_for_mode(choice.as_str(), capabilities, policy)
        }
        _ => bail!("setup cancelled; choose recommended, maximum, or experimental"),
    }
}

fn configure_setup_backend(backend: &str) -> Result<()> {
    match backend {
        "ibus" => {
            if !ibus_engine_available() {
                bail!("WayExpand's IBus engine is not installed or IBus is unavailable")
            }
            println!("Configuring the WayExpand IBus engine for the current session...");
            let restart = Command::new("ibus").arg("restart").status()?;
            if !restart.success() {
                bail!("IBus restart failed; no WayExpand service was enabled")
            }
            let select = Command::new("ibus")
                .args(["engine", "wayexpand"])
                .status()?;
            if !select.success() {
                bail!("could not select the WayExpand IBus engine")
            }
            println!("WayExpand IBus integration is active. Run `wayexpand doctor` to inspect the session.");
        }
        "input-method" => enable_user_service("wayexpand-input-method.service")?,
        "evdev" => {
            println!("Warning: evdev can observe global keyboard input and has no password-field signal.");
            println!("Portal consent may be requested by libei; raw-input permissions are not changed by setup.");
            enable_user_service("wayexpand-evdev.service")?;
        }
        _ => bail!("unknown setup backend {backend:?}; choose ibus, input-method, or evdev"),
    }
    Ok(())
}

fn enable_user_service(service: &str) -> Result<()> {
    let installed = Command::new("systemctl")
        .args(["--user", "cat", service])
        .status()?;
    if !installed.success() {
        bail!("user service {service} is not installed; run the WayExpand installer first")
    }
    let reload_arguments = ["--user", "daemon-reload"];
    let enable_arguments = ["--user", "enable", "--now", service];
    for arguments in [&reload_arguments[..], &enable_arguments[..]] {
        let status = Command::new("systemctl").args(arguments).status()?;
        if !status.success() {
            bail!("systemd could not apply {service}; no further setup actions were taken")
        }
    }
    println!("Enabled and started {service}.");
    println!("Run `wayexpand status` and `wayexpand doctor` to verify the live integration.");
    Ok(())
}

fn print_backend_diagnostics(include_experimental_input_method: bool) -> bool {
    let backends = discover_backends();
    for status in &backends {
        let detail = format!(
            "{}; implementation={}, availability={}, permission={}",
            status.detail,
            status.implementation(),
            status.availability(),
            status.permission()
        );
        println!("{:28} {:?} ({})", status.kind, status.state, detail);
    }
    let policy = load_policy().unwrap_or_else(|_| OrganizationPolicy {
        safe_mode: true,
        allowed_backends: vec!["none".into()],
        ..OrganizationPolicy::default()
    });
    let ibus_policy_allowed = policy.backend_allowed("input-method-v2");
    let ibus_installed = ibus_engine_available();
    if ibus_installed && ibus_policy_allowed {
        println!(
            "IBus WayExpand engine: installed and ready to configure (password/PIN awareness)"
        );
    } else if ibus_installed {
        println!("IBus WayExpand engine: installed but disallowed by organization policy");
    } else {
        println!("IBus WayExpand engine: not installed or discoverable (install the IBus component to use it)");
    }
    // Doctor is also used in CI and for validating a config outside a desktop
    // session. Explain the session limitation; the caller still reports an
    // unhealthy result when no usable path can be established.
    if !display_session_available() {
        if std::env::var_os("DISPLAY").is_some() {
            println!(
                "X11 session: no native X11 global-capture backend is implemented; evdev + libei "
            );
            println!(
                "is the available cross-session route and requires explicit input permission."
            );
        } else {
            println!("No active Wayland or X11 display detected; backend probes were skipped.");
        }
        return false;
    }
    let live_capabilities = probe_capabilities();
    let wlroots_available = if live_capabilities.has_virtual_keyboard {
        println!("wlroots probe: virtual keyboard globals available");
        true
    } else {
        match WlrootsInjector::probe() {
            Ok(_) => {
                println!("wlroots probe: virtual keyboard globals available");
                true
            }
            Err(error) => {
                println!("wlroots probe: unavailable ({error})");
                false
            }
        }
    };
    let input_method_available = if include_experimental_input_method {
        if live_capabilities.has_input_method_v2 {
            println!("input-method-v2 probe: manager and seat connection succeeded");
            true
        } else {
            match InputMethodSource::probe() {
                Ok(_) => {
                    println!("input-method-v2 probe: manager and seat connection succeeded");
                    true
                }
                Err(error) => {
                    println!("input-method-v2 probe: unavailable ({error})");
                    false
                }
            }
        }
    } else {
        false
    };
    let evdev_readable = live_capabilities.has_dev_input;
    // libei needs a RemoteDesktop portal with EIS support (or an explicit
    // LIBEI_SOCKET); probing the portal would pop a consent dialog, so this
    // only recognizes desktops known to ship one.
    let libei_plausible = libei_portal_candidate();

    // These are non-invasive protocol probes, not end-to-end typing tests.
    // Keep that distinction visible: a successful globals/seat probe cannot
    // prove that a real GTK or Qt client will preserve every key event.
    let mut available_combinations = Vec::new();
    let mut trial_combinations = Vec::new();
    let ibus_available = ibus_installed && ibus_policy_allowed;
    if input_method_available && policy.backend_allowed("input-method-v2") {
        available_combinations
            .push("--source=input-method (protocol probe passed; live typing unverified)");
    }
    if evdev_readable && wlroots_available && policy.backend_allowed("wlroots") {
        available_combinations.push(
            "--source=evdev --backend=wlroots (protocol probe passed; live insertion unverified)",
        );
    }
    if evdev_readable && libei_plausible && policy.backend_allowed("libei") {
        trial_combinations.push(
            "--source=evdev --backend=libei (available to try; interactive authorization required)",
        );
    }

    if !capture_path_available(
        ibus_available,
        available_combinations.len(),
        trial_combinations.len(),
    ) {
        println!("Capture readiness: NOT READY (no source+backend combination detected)");
        if evdev_readable && !libei_plausible {
            println!(
                "evdev is readable, but no output backend was detected: the wlroots probe \
                 failed and this desktop is not known to provide a libei portal."
            );
        } else {
            println!(
                "Next step: use a compositor with input-method-v2 support, or \
                 `--source=evdev` (requires `input` group membership; see SECURITY.md for the \
                 sensitive-field tradeoff) paired with `--backend=wlroots` or `--backend=libei`."
            );
        }
        return false;
    }
    if ibus_available && available_combinations.is_empty() && trial_combinations.is_empty() {
        println!(
            "Capture readiness: AVAILABLE TO TRY (IBus is installed; live client typing is not verified)"
        );
    } else if available_combinations.is_empty() && !trial_combinations.is_empty() {
        println!(
            "Capture readiness: AUTHORIZATION REQUIRED (libei was detected heuristically; interactive portal consent is required)"
        );
    } else if available_combinations.is_empty() {
        println!(
            "Capture readiness: AVAILABLE TO TRY (no backend was verified; interactive authorization required)"
        );
    } else {
        println!(
            "Capture readiness: AVAILABLE TO TRY (protocol probe passed; end-to-end typing is not verified)"
        );
    }
    if ibus_available {
        println!(
            "  available to try: IBus committed-text path (password/PIN awareness; live typing unverified)"
        );
    }
    println!(
        "Automatic selection: stdin + libei (raw evdev capture is disabled unless `--source=evdev` is explicitly selected)"
    );
    if evdev_readable {
        println!(
            "evdev probe: readable, but not enabled automatically; explicit `--source=evdev` acknowledges global keyboard visibility"
        );
    }
    for combination in &available_combinations {
        println!("  available to try: wayexpand-daemon {combination}");
    }
    for combination in &trial_combinations {
        println!("  available to try: wayexpand-daemon {combination}");
    }
    println!("Setup guidance:");
    if input_method_available {
        println!(
            "  experimental opt-in: systemctl --user enable --now wayexpand-input-method.service"
        );
        println!("  warning: unsupported non-text keys may be lost");
    }
    if evdev_readable {
        println!(
            "  explicit evdev: requires the input group/udev grant and has no password-field signal"
        );
    }
    if libei_plausible && evdev_readable {
        if let Some(path) = portal_token_path() {
            println!(
                "  libei portal token: {} (use `wayexpand portal reset` to re-authorize)",
                if path.is_file() {
                    "present"
                } else {
                    "not present"
                }
            );
        }
    }
    ibus_available || !available_combinations.is_empty()
}

fn print_backend_selection_explain() {
    match explain_auto_selection() {
        Ok(explanation) => print!("{explanation}"),
        Err(error) => eprintln!("backend selection failed: {error}"),
    }
}

fn capture_path_available(
    ibus_available: bool,
    available_count: usize,
    trial_count: usize,
) -> bool {
    ibus_available || available_count > 0 || trial_count > 0
}

fn print_certification(json: bool) -> Result<bool> {
    let required_scenarios = certification_scenarios()?;
    let capabilities = probe_capabilities();
    let policy_result = wayexpand_core::load_organization_policy();
    let policy_allows = |backend: &str| {
        policy_result
            .as_ref()
            .is_ok_and(|policy| policy.backend_allowed(backend))
    };
    let policy_allows_ibus = policy_allows("input-method-v2");
    let selection = wayexpand_backend_selection::auto_select(None, None)
        .ok()
        .filter(|selection| {
            policy_result.as_ref().is_ok_and(|policy| {
                policy.backend_allowed(wayexpand_core::policy_backend_name(
                    selection.pair.source(),
                    selection.pair.backend(),
                ))
            })
        });
    let ibus_installed = ibus_engine_available();
    let ibus = ibus_installed && policy_allows_ibus;
    let selected_label = if ibus {
        "IBus"
    } else {
        selection
            .as_ref()
            .map(|selection| selection.pair.source())
            .unwrap_or("none")
    };
    let mut checks = Vec::new();
    let mut add_check = |category: &str, name: &str, status: &str, detail: &str| {
        checks.push(serde_json::json!({
            "category": category,
            "name": name,
            "status": status,
            "detail": detail,
        }));
    };

    let certification_config = env::var_os("WAYEXPAND_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    match Config::load(&certification_config) {
        Ok(_) => add_check(
            "configuration",
            "active configuration",
            "verified",
            "the configured expansion file parses and passes security validation",
        ),
        Err(error) => add_check(
            "configuration",
            "active configuration",
            "failed",
            &format!("configuration is not usable: {}", error.safe_summary()),
        ),
    }
    match &policy_result {
        Ok(_) => add_check(
            "policy",
            "organization policy",
            "verified",
            "the shared secure organization-policy loader accepted the policy state",
        ),
        Err(error) => add_check(
            "policy",
            "organization policy",
            "failed",
            &format!("organization policy blocks startup: {error}"),
        ),
    }

    if capabilities.compositor != wayexpand_backend_selection::Compositor::Unknown {
        add_check(
            "environment",
            "desktop identified",
            "verified",
            capabilities.compositor.name(),
        );
    } else {
        add_check(
            "environment",
            "desktop identified",
            "unknown",
            "XDG_CURRENT_DESKTOP is not a recognized compositor",
        );
    }
    if ibus {
        add_check(
            "input-path",
            "IBus engine installed",
            "available",
            "IBus is a candidate for Recommended mode",
        );
    } else if ibus_installed && !policy_allows_ibus {
        add_check(
            "input-path",
            "IBus engine installed",
            "unsupported",
            "IBus is installed but input-method-v2 is disallowed by organization policy",
        );
    } else {
        add_check(
            "input-path",
            "IBus engine installed",
            "unsupported",
            "the WayExpand IBus component is not installed or discoverable",
        );
    }
    if capabilities.has_input_method_v2 && policy_allows("input-method-v2") {
        add_check(
            "input-path",
            "input-method-v2 protocol",
            "available",
            "protocol manager and seat probe succeeded; live key pass-through remains untested",
        );
    } else if capabilities.has_input_method_v2 {
        add_check(
            "input-path",
            "input-method-v2 protocol",
            "unsupported",
            "the compositor exposed the protocol, but organization policy disallows this backend",
        );
    } else {
        add_check(
            "input-path",
            "input-method-v2 protocol",
            "unsupported",
            "the compositor did not expose a usable input-method-v2 interface",
        );
    }
    if capabilities.has_virtual_keyboard && policy_allows("wlroots") {
        add_check(
            "output-path",
            "wlroots virtual keyboard",
            "available",
            "virtual keyboard globals were found; end-to-end insertion remains untested",
        );
    } else if capabilities.has_virtual_keyboard {
        add_check(
            "output-path",
            "wlroots virtual keyboard",
            "unsupported",
            "the compositor exposed the protocol, but organization policy disallows this backend",
        );
    } else {
        add_check(
            "output-path",
            "wlroots virtual keyboard",
            "unsupported",
            "the compositor did not expose zwp_virtual_keyboard_v1",
        );
    }
    if capabilities.has_direct_libei_socket && policy_allows("libei") {
        add_check(
            "output-path",
            "libei/EIS transport",
            "available",
            "an explicit LIBEI_SOCKET is present; portal authorization was not re-requested",
        );
    } else if capabilities.has_direct_libei_socket {
        add_check(
            "output-path",
            "libei/EIS transport",
            "unsupported",
            "an EIS socket was detected, but organization policy disallows this backend",
        );
    } else {
        add_check("output-path", "libei/EIS transport", "authorization-required", "portal probing is intentionally non-interactive; run the selected mode to authorize it");
    }

    let selected_capture = if ibus { "ibus" } else { selected_label };
    let selection_status = certification_selection_status(selected_capture);
    let selection_detail = if selected_capture == "stdin" {
        "automatic selection is the conservative stdin-only fallback; no keyboard capture path is configured".to_owned()
    } else {
        format!("automatic selection currently resolves to {selected_capture}")
    };
    add_check(
        "selection",
        "automatic mode selection",
        selection_status,
        &selection_detail,
    );

    // These checks intentionally remain NOT RUN until a compositor-specific
    // harness drives real GTK/Qt/Wayland clients. A preflight must never turn
    // protocol availability into a false CERTIFIED claim.
    for name in &required_scenarios {
        let category = certification_scenario_category(name);
        add_check(
            category,
            name,
            "not-run",
            "requires the compositor certification harness; no claim is made from a static probe",
        );
    }

    let certified = checks
        .iter()
        .all(|check| check["status"] == "verified" || check["status"] == "available")
        && !checks.is_empty();
    let limitations = if ibus {
        vec![
            "GTK and Qt client behavior still requires live certification",
            "IME/preedit composition is not supported",
            "surrounding-text behavior depends on the client toolkit",
        ]
    } else if selected_capture == "stdin" {
        vec![
            "no automatic keyboard input path is selected",
            "text expansion is available only through the stdin test harness",
        ]
    } else {
        wayexpand_core::all_capabilities()
            .into_iter()
            .filter(|caps| caps.backend_name == selected_capture)
            .flat_map(|caps| caps.limitations.iter().copied())
            .collect::<Vec<_>>()
    };
    let report = serde_json::json!({
        "schema": 1,
        "wayexpand_version": build_info::VERSION,
        "wayexpand_commit": build_info::COMMIT,
        "certified": certified,
        "desktop": capabilities.compositor.name(),
        "config_path": certification_config,
        "selected_mode": selected_capture,
        "required_scenarios": required_scenarios,
        "checks": checks,
        "limitations": limitations,
    });
    if json {
        println!("{report}");
    } else {
        println!("WayExpand Desktop Certification");
        println!(
            "  WayExpand: {} (commit {})",
            build_info::VERSION,
            build_info::COMMIT
        );
        println!("  Desktop: {}", report["desktop"]);
        println!("  Selected mode: {}", report["selected_mode"]);
        for check in report["checks"].as_array().into_iter().flatten() {
            println!(
                "  [{:18}] {:32} {}",
                check["status"].as_str().unwrap_or("unknown"),
                check["name"].as_str().unwrap_or("unknown"),
                check["detail"].as_str().unwrap_or("")
            );
        }
        println!(
            "\nResult: {}",
            if certified {
                "CERTIFIED"
            } else {
                "NOT CERTIFIED"
            }
        );
        println!("Run the compositor harness before treating this record as a support claim.");
    }
    Ok(certified)
}

fn certification_selection_status(selected_capture: &str) -> &'static str {
    match selected_capture {
        "none" => "failed",
        "stdin" => "unsupported",
        _ => "available",
    }
}

fn certification_scenarios() -> Result<Vec<String>> {
    let matrix: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/certification/compositor-matrix.json"
    ))
    .context("checked-in compositor certification matrix is invalid")?;
    matrix["required_scenarios"]
        .as_array()
        .context("certification matrix has no required_scenarios array")?
        .iter()
        .map(|scenario| {
            scenario
                .as_str()
                .map(str::to_owned)
                .context("certification matrix contains a non-string scenario")
        })
        .collect()
}

fn certification_scenario_category(scenario: &str) -> &'static str {
    match scenario {
        "printable-press-release" | "held-keys-repeat" | "modifier-navigation" => {
            "typing-integrity"
        }
        "unicode-combining" | "multiline-rapid" => "text-integrity",
        "password-field" | "focus-cross-window" => "safety",
        "config-reload" | "daemon-restart" | "compositor-restart" | "failed-insertion" => {
            "recovery"
        }
        "ime-preedit" => "input-method",
        _ => "other",
    }
}

/// Stable, automation-friendly diagnostic output for service managers and
/// fleet health checks. It deliberately avoids compositor probes that can
/// block or mutate session state; those remain in the human doctor output.
fn print_json_diagnostics(path: &Path) -> Result<bool> {
    let config_result = Config::load(path);
    let config_ok = config_result.is_ok();
    let socket_path = std::env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        });
    let socket_exists = socket_path.as_ref().is_some_and(|socket| socket.exists());
    let socket_valid = socket_path
        .as_deref()
        .is_none_or(existing_control_socket_is_healthy);
    let policy_result = load_policy();
    let policy = match &policy_result {
        Ok(policy) => policy.clone(),
        Err(_) => OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["none".into()],
            ..OrganizationPolicy::default()
        },
    };
    let backends: Vec<_> = discover_backends()
        .into_iter()
        .map(|status| {
            serde_json::json!({
                "kind": status.kind.to_string(),
                "state": format!("{:?}", status.state),
                "implementation": status.implementation(),
                "availability": status.availability(),
                "permission": status.permission(),
                "policy_allowed": backend_policy_allowed(status.kind, &policy),
                "detail": status.detail,
            })
        })
        .collect();
    let ibus_installed = ibus_engine_available();
    let policy_json = print_policy_diagnostics_json(&policy_result);
    let capabilities = print_capabilities_diagnostics_json();
    let live_capabilities = probe_capabilities();
    let (capture_state, capture_detail) =
        capture_readiness(&live_capabilities, ibus_installed, &policy);
    let recommendation = recommended_setup_backend(&live_capabilities, &policy);
    let setup_recommendation = serde_json::json!({
        "mode": if recommendation.backend == "unavailable" { "none" } else if recommendation.backend == "evdev" { "maximum" } else { "recommended" },
        "backend": recommendation.backend,
        "label": recommendation.label,
        "detail": recommendation.detail,
        "ready": recommendation.backend != "unavailable",
    });
    let automatic_selection = wayexpand_backend_selection::auto_select(None, None)
        .map(|selection| {
            let policy_allowed = policy.backend_allowed(wayexpand_core::policy_backend_name(
                selection.pair.source(),
                selection.pair.backend(),
            ));
            serde_json::json!({
                "source": selection.pair.source(),
                "backend": selection.pair.backend(),
                "reason": selection.reason,
                "policy_allowed": policy_allowed,
                "ready": automatic_selection_is_ready(
                    selection.pair.source(),
                    selection.pair.backend(),
                    &policy,
                ),
            })
        })
        .unwrap_or_else(|error| {
            serde_json::json!({
                "source": serde_json::Value::Null,
                "backend": serde_json::Value::Null,
                "reason": error.to_string(),
                "policy_allowed": false,
                "ready": false,
            })
        });
    let setup_ibus_ready = recommendation.backend == "ibus" && ibus_installed;
    let setup_recommendation = if setup_ibus_ready {
        serde_json::json!({
            "mode": "recommended",
            "backend": "ibus",
            "label": "IBus",
            "detail": "toolkit-aware committed text with password/PIN purpose support; no raw keyboard access",
            "ready": true,
        })
    } else {
        setup_recommendation
    };
    let selection_ok = automatic_selection["reason"].is_string()
        && automatic_selection["source"].is_string()
        && automatic_selection["ready"].as_bool().unwrap_or(false);
    let policy_ok = policy_json
        .get("policy")
        .and_then(|policy| policy.get("valid"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let healthy = config_ok
        && policy_ok
        && display_session_available()
        && (selection_ok || setup_ibus_ready)
        && socket_valid;
    println!(
        "{}",
        serde_json::json!({
            "wayexpand_version": build_info::VERSION,
            "wayexpand_commit": build_info::COMMIT,
            "healthy": healthy,
            "wayland": std::env::var_os("WAYLAND_DISPLAY").is_some(),
            "config": {
                "path": path,
                "valid": config_ok,
                "error": config_result.err().map(|error| error.safe_summary()),
            },
            "control_socket": {
                "path": socket_path,
                "configured": socket_path.is_some(),
                "exists": socket_exists,
                "valid": socket_valid,
            },
            "ibus": {
                "installed": ibus_installed,
                "status": if ibus_installed { "available to configure" } else { "not installed" },
            },
            "policy": policy_json,
            "backends": backends,
            "capabilities": capabilities,
            "automatic_selection": automatic_selection,
            "setup_recommendation": setup_recommendation,
            "capture_readiness": {
                "state": capture_state,
                "detail": capture_detail,
                "end_to_end_verified": false,
            },
        })
    );
    Ok(healthy)
}

fn display_session_available() -> bool {
    display_session_flags(
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("WAYLAND_SOCKET").is_some(),
        std::env::var_os("DISPLAY").is_some(),
    )
}

fn display_session_flags(wayland_display: bool, wayland_socket: bool, x11_display: bool) -> bool {
    wayland_display || wayland_socket || x11_display
}

fn backend_policy_allowed(kind: BackendKind, policy: &OrganizationPolicy) -> Option<bool> {
    kind.policy_name()
        .map(|backend| policy.backend_allowed(backend))
}

fn existing_control_socket_is_healthy(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    control_socket_parent_is_healthy(path)
        && metadata.file_type().is_socket()
        && metadata.uid() == rustix::process::geteuid().as_raw()
        && metadata.mode() & 0o077 == 0
}

fn control_socket_parent_is_healthy(path: &Path) -> bool {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return false;
    };
    let Ok(resolved_parent) = fs::canonicalize(parent) else {
        return false;
    };
    let current_uid = rustix::process::geteuid().as_raw();
    let mut current = resolved_parent.as_path();
    let mut immediate = true;
    loop {
        let Ok(metadata) = fs::metadata(current) else {
            return false;
        };
        if !metadata.is_dir() {
            return false;
        }
        if metadata.uid() != current_uid && metadata.uid() != 0 {
            return false;
        }
        let mode = metadata.mode() & 0o7777;
        let root_sticky = metadata.uid() == 0 && mode & 0o1000 != 0;
        if mode & 0o022 != 0 && (!root_sticky || immediate) {
            return false;
        }
        if current == Path::new("/") {
            return true;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
        immediate = false;
    }
}

fn automatic_selection_is_ready(source: &str, backend: &str, policy: &OrganizationPolicy) -> bool {
    source != "stdin"
        && policy.backend_allowed(wayexpand_core::policy_backend_name(source, backend))
}

/// Classify non-invasive session probes without calling them an end-to-end
/// guarantee. A protocol/global probe can establish that a path is worth
/// trying, but only a compositor/client harness can verify typing integrity.
fn capture_readiness(
    capabilities: &wayexpand_backend_selection::Capabilities,
    ibus_installed: bool,
    policy: &OrganizationPolicy,
) -> (&'static str, &'static str) {
    if (ibus_installed || capabilities.has_input_method_v2)
        && policy.backend_allowed("input-method-v2")
    {
        return (
            "available-to-try",
            "a protocol or IBus probe succeeded; live client typing is not verified",
        );
    }
    if capabilities.has_dev_input
        && capabilities.has_virtual_keyboard
        && policy.backend_allowed("wlroots")
    {
        return (
            "available-to-try",
            "evdev capture and a virtual-keyboard output probe succeeded; live typing is not verified",
        );
    }
    if capabilities.has_dev_input
        && capabilities.has_direct_libei_socket
        && policy.backend_allowed("libei")
    {
        return (
            "available-to-try",
            "evdev capture and a direct EIS socket were detected; live typing is not verified",
        );
    }
    if capabilities.has_dev_input && libei_portal_candidate() && policy.backend_allowed("libei") {
        return (
            "authorization-required",
            "evdev is readable and a libei portal candidate was detected; interactive authorization is required",
        );
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("WAYLAND_SOCKET").is_none()
    {
        return (
            "not-probed",
            "no active Wayland session was detected; backend probes were skipped",
        );
    }
    (
        "unavailable",
        "no non-invasive source and output path was detected",
    )
}

fn status_as_json(response: &str) -> Result<serde_json::Value> {
    let mut object = serde_json::Map::new();
    let mut lines = response.lines();
    if let Some(state) = lines.next() {
        object.insert("response".into(), state.into());
    }
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = match value {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            value
                if matches!(
                    key,
                    "command_queue_depth"
                        | "command_queue_rejected_total"
                        | "command_timeout_total"
                        | "command_failure_total"
                ) =>
            {
                match value.parse::<u64>() {
                    Ok(number) => serde_json::Value::Number(number.into()),
                    Err(_) => value.into(),
                }
            }
            _ => value.into(),
        };
        object.insert(key.to_owned(), value);
    }
    Ok(serde_json::Value::Object(object))
}

fn print_config_diagnostics(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => {
            let mode = metadata.mode() & 0o777;
            println!(
                "Config: {} (mode {:04o}, uid {})",
                path.display(),
                mode,
                metadata.uid()
            );
            if !metadata.file_type().is_file() {
                println!("Config warning: path is not a regular file");
            }
            let current_uid = rustix::process::geteuid().as_raw();
            if metadata.uid() != current_uid && metadata.uid() != 0 {
                println!("Config warning: file is not owned by the current user or root");
            }
            if mode & 0o022 != 0 {
                println!("Config warning: file is writable by group or other users");
            }
            print_config_parent_diagnostics(path);
            match Config::load(path) {
                Ok(_) => {
                    println!("Config validation: OK");
                    true
                }
                Err(error) => {
                    println!("Config validation: FAILED ({})", error.safe_summary());
                    false
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("Config: {} (not found)", path.display());
            false
        }
        Err(error) => {
            println!("Config: {} (unreadable: {error})", path.display());
            false
        }
    }
}

fn print_config_parent_diagnostics(path: &Path) {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut current = resolved
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    loop {
        let Ok(metadata) = fs::metadata(current) else {
            return;
        };
        if !metadata.is_dir() {
            println!("Config warning: parent is not a directory");
            return;
        }
        let current_uid = rustix::process::geteuid().as_raw();
        let mode = metadata.mode() & 0o7777;
        // Sticky protection limits unlink/rename rights but does not make an
        // untrusted directory a valid configuration ancestor. Keep doctor in
        // lockstep with Config::load's security policy.
        if metadata.uid() != current_uid && metadata.uid() != 0 {
            println!(
                "Config warning: parent {} is owned by untrusted uid {}",
                current.display(),
                metadata.uid()
            );
        }
        if mode & 0o022 != 0 && mode & 0o1000 == 0 {
            println!(
                "Config warning: parent {} is writable by group or other users without sticky protection (mode {:04o})",
                current.display(),
                mode & 0o7777
            );
            println!("Config fix: chmod go-w {}", current.display());
        }
        if current == Path::new("/") {
            return;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
}

fn print_control_socket_diagnostics() -> bool {
    let Some(path) = std::env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        })
    else {
        println!("Control socket: disabled (XDG_RUNTIME_DIR unavailable)");
        return true;
    };

    let mut valid = true;
    println!("Control socket: {}", path.display());
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    let resolved_parent = match fs::canonicalize(parent) {
        Ok(resolved) => Some(resolved),
        Err(error) => {
            println!("Control socket warning: parent is unavailable ({error})");
            valid = false;
            None
        }
    };
    if let Some(resolved_parent) = resolved_parent.as_deref() {
        let current_uid = rustix::process::geteuid().as_raw();
        let mut current = resolved_parent;
        let mut immediate = true;
        loop {
            match fs::metadata(current) {
                Ok(metadata) if metadata.is_dir() => {
                    if metadata.uid() != current_uid && metadata.uid() != 0 {
                        println!(
                            "Control socket warning: {} is not owned by the current user or root",
                            if immediate { "parent" } else { "an ancestor" }
                        );
                        valid = false;
                    }
                    let mode = metadata.mode() & 0o7777;
                    let root_sticky = metadata.uid() == 0 && mode & 0o1000 != 0;
                    if mode & 0o022 != 0 && (!root_sticky || immediate) {
                        println!(
                            "Control socket warning: {} is writable by group or other users",
                            if immediate { "parent" } else { "an ancestor" }
                        );
                        valid = false;
                    }
                }
                Ok(_) => {
                    println!("Control socket warning: parent is not a directory");
                    valid = false;
                    break;
                }
                Err(error) => {
                    println!("Control socket warning: parent is unavailable ({error})");
                    valid = false;
                    break;
                }
            }
            if current == Path::new("/") {
                break;
            }
            current = current.parent().unwrap_or_else(|| Path::new("/"));
            immediate = false;
        }
    }
    let existing_path = resolved_parent
        .as_deref()
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| path.clone());

    match fs::symlink_metadata(&existing_path) {
        Ok(metadata) => {
            let mode = metadata.mode() & 0o777;
            println!(
                "Control socket existing path: mode {:04o}, uid {}, socket={}",
                mode,
                metadata.uid(),
                metadata.file_type().is_socket()
            );
            if !metadata.file_type().is_socket() {
                println!("Control socket warning: existing path is not a socket");
                valid = false;
            }
            if metadata.uid() != rustix::process::geteuid().as_raw() {
                println!("Control socket warning: existing path is not owned by the current user");
                valid = false;
            }
            if mode & 0o077 != 0 {
                println!("Control socket warning: existing socket is more permissive than 0600");
                valid = false;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("Control socket existing path: not present (will be created)");
            valid = false;
        }
        Err(error) => {
            println!("Control socket existing path: unreadable ({error})");
            valid = false;
        }
    }
    valid
}

fn load_policy() -> Result<OrganizationPolicy> {
    wayexpand_core::load_organization_policy().map_err(|error| anyhow::anyhow!(error))
}

fn absolute_command_policy_diagnostic(policy: &OrganizationPolicy) -> Option<&'static str> {
    policy
        .require_absolute_commands
        .then_some(if policy.safe_mode {
            "Require absolute command paths: enforced"
        } else {
            "Require absolute command paths: audit only"
        })
}

fn print_policy_diagnostics_json(policy_result: &Result<OrganizationPolicy>) -> serde_json::Value {
    let policy_json = match policy_result {
        Ok(policy) => {
            serde_json::json!({
                "valid": true,
                "safe_mode": policy.safe_mode,
                "disable_commands": policy.disable_commands,
                "require_absolute_commands": policy.require_absolute_commands,
                "disable_hotkeys": policy.disable_hotkeys,
                "disable_title_matching": policy.disable_title_matching,
                "max_replacement_size": policy.max_replacement_size,
                "allowed_backends": policy.allowed_backends,
                "allowed_packs": policy.allowed_packs,
                "audit_prefix": policy.audit_prefix,
                "is_active": policy.is_active(),
            })
        }
        Err(error) => {
            serde_json::json!({
                "valid": false,
                "error": error.to_string(),
            })
        }
    };

    serde_json::json!({
        "path": wayexpand_core::ORGANIZATION_POLICY_PATH,
        "exists": Path::new(wayexpand_core::ORGANIZATION_POLICY_PATH).exists(),
        "policy": policy_json,
    })
}

fn print_capabilities_diagnostics() {
    println!("\nBackend capabilities:");
    let all_caps = all_capabilities();
    for caps in all_caps {
        let env_support = if caps.environment_compatible() {
            "yes"
        } else {
            "no"
        };
        println!(
            "  {}: environment compatible: {}",
            caps.backend_name, env_support
        );
        println!("      live protocol probe: reported separately above (not inferred here)");
        println!("    Features: {}", caps.feature_summary);
        for limitation in caps.limitations {
            println!("    Limitation: {limitation}");
        }
        println!(
            "    Max replacement: {}",
            if caps.max_replacement_size == 0 {
                "unlimited".to_string()
            } else {
                format!("{} bytes", caps.max_replacement_size)
            }
        );
    }
}

fn print_capabilities_diagnostics_json() -> serde_json::Value {
    let all_caps = all_capabilities();
    let caps_json: Vec<_> = all_caps
        .iter()
        .map(|caps| {
            serde_json::json!({
                "backend": caps.backend_name,
                "environment_compatible": caps.environment_compatible(),
                "multiline": caps.multiline,
                "exclusive_capture": caps.exclusive_capture,
                "text_method": caps.text_method.to_string(),
                "max_replacement_size": caps.max_replacement_size,
                "feature_summary": caps.feature_summary,
                "limitations": caps.limitations,
            })
        })
        .collect();
    serde_json::Value::Array(caps_json)
}

fn print_policy_diagnostics() -> bool {
    if !Path::new(wayexpand_core::ORGANIZATION_POLICY_PATH).exists() {
        println!(
            "Organization policy: {} (not found, using default permissive policy)",
            wayexpand_core::ORGANIZATION_POLICY_PATH
        );
        return true;
    }

    match load_policy() {
        Ok(policy) => {
            println!(
                "Organization policy: {} (valid)",
                wayexpand_core::ORGANIZATION_POLICY_PATH
            );
            if policy.is_active() {
                println!(
                    "  Safe mode: {}",
                    if policy.safe_mode {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                if policy.disable_commands {
                    println!("  Disable commands: enabled");
                }
                if let Some(diagnostic) = absolute_command_policy_diagnostic(&policy) {
                    println!("  {diagnostic}");
                }
                if policy.disable_hotkeys {
                    println!("  Disable hotkeys: enabled");
                }
                if policy.disable_title_matching {
                    println!("  Disable title matching: enabled");
                }
                if policy.max_replacement_size > 0 {
                    println!(
                        "  Max replacement size: {} bytes",
                        policy.max_replacement_size
                    );
                }
                if !policy.allowed_backends.is_empty() {
                    println!("  Allowed backends: {:?}", policy.allowed_backends);
                }
                if !policy.allowed_packs.is_empty() {
                    println!("  Allowed packs: {:?}", policy.allowed_packs);
                }
                if !policy.audit_prefix.is_empty() {
                    println!("  Audit prefix: {}", policy.audit_prefix);
                }
            } else {
                println!("  (all constraints disabled, using defaults)");
            }
            true
        }
        Err(error) => {
            println!(
                "Organization policy: {} (invalid: {})",
                wayexpand_core::ORGANIZATION_POLICY_PATH,
                error
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn doctor_policy_json_reports_absolute_command_requirement() {
        let policy = OrganizationPolicy {
            safe_mode: false,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };
        let diagnostics = print_policy_diagnostics_json(&Ok(policy));

        assert_eq!(
            diagnostics["policy"]["require_absolute_commands"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            absolute_command_policy_diagnostic(&OrganizationPolicy {
                safe_mode: false,
                require_absolute_commands: true,
                ..OrganizationPolicy::default()
            }),
            Some("Require absolute command paths: audit only")
        );
    }

    #[test]
    fn backup_refuses_existing_destination_atomically() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let source = std::env::temp_dir().join(format!("wayexpand-backup-source-{suffix}"));
        let destination = std::env::temp_dir().join(format!("wayexpand-backup-dest-{suffix}"));
        fs::write(&source, "source").unwrap();
        fs::write(&destination, "existing").unwrap();

        let error = create_backup(&source, &destination).unwrap_err();
        assert!(error.to_string().contains("creating configuration backup"));
        assert_eq!(fs::read_to_string(&destination).unwrap(), "existing");

        let _ = fs::remove_file(source);
        let _ = fs::remove_file(destination);
    }

    #[test]
    fn status_json_preserves_types_and_ignores_banner() {
        let value = status_as_json(
            "running\nsource=stdin\npaused=true\ncommand_queue_depth=3\nconfig_state=ok\n",
        )
        .unwrap();
        assert_eq!(value["response"], "running");
        assert_eq!(value["source"], "stdin");
        assert_eq!(value["paused"], true);
        assert_eq!(value["command_queue_depth"], 3);
        assert_eq!(value["config_state"], "ok");
    }

    /// Contract test for docs/COMPATIBILITY.md's `wayexpand status --json`
    /// section: the exact daemon status line documented there as the
    /// "Stable" example must still produce exactly the documented field
    /// set (no more, no less) and types. If this fails, either the
    /// implementation changed in a way that needs a compatibility note, or
    /// the documentation needs to be updated to match -- either way it
    /// should not be silently discovered by a user's integration breaking.
    #[test]
    fn status_json_matches_documented_stable_contract() {
        let daemon_response = "running\n\
             source=input-method\n\
             backend=input-method-v2\n\
             state=connected\n\
             paused=false\n\
             config=/home/user/.config/wayexpand/expansions.toml\n\
             config_state=ok\n\
             command_queue_depth=0\n\
             command_queue_rejected_total=0\n\
             command_timeout_total=0\n\
             command_failure_total=0";
        let value = status_as_json(daemon_response).unwrap();
        let object = value.as_object().expect("status --json returns an object");
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/contracts/status-json.json"))
                .expect("status contract fixture must be valid JSON");
        let documented_fields = contract["fields"]
            .as_object()
            .expect("status contract fields must be an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            object
                .keys()
                .map(|key| key.as_str())
                .collect::<std::collections::BTreeSet<_>>(),
            documented_fields.into_iter().collect(),
            "status --json fields no longer match docs/COMPATIBILITY.md's documented Stable contract"
        );
        assert_eq!(value["response"], "running");
        assert_eq!(value["source"], "input-method");
        assert_eq!(value["backend"], "input-method-v2");
        assert_eq!(value["state"], "connected");
        assert_eq!(value["paused"], false);
        assert_eq!(
            value["config"],
            "/home/user/.config/wayexpand/expansions.toml"
        );
        assert_eq!(value["config_state"], "ok");
        assert_eq!(value["command_queue_depth"], 0);
        assert_eq!(value["command_queue_rejected_total"], 0);
        assert_eq!(value["command_timeout_total"], 0);
        assert_eq!(value["command_failure_total"], 0);
    }

    #[test]
    fn stable_cli_shape_fixture_is_valid_and_includes_status_contract() {
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/contracts/cli-json-shapes.json"
        ))
        .expect("CLI contract fixture must be valid JSON");
        let doctor_fields = contract["doctor"]
            .as_array()
            .expect("doctor contract must be an array")
            .iter()
            .map(|field| field.as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            doctor_fields,
            [
                "healthy",
                "wayland",
                "config",
                "control_socket",
                "policy",
                "backends",
                "capabilities",
                "automatic_selection",
                "setup_recommendation",
                "capture_readiness",
            ]
            .into_iter()
            .collect()
        );
        let status_fields = contract["status"]
            .as_array()
            .expect("status contract must be an array")
            .iter()
            .map(|field| field.as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            status_fields,
            [
                "response",
                "source",
                "backend",
                "state",
                "paused",
                "config",
                "config_state",
                "command_queue_depth",
                "command_queue_rejected_total",
                "command_timeout_total",
                "command_failure_total",
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn json_flag_is_recognized_in_any_position() {
        let mut args = vec!["expansions.toml".to_string(), "--json".to_string()];
        assert!(take_json_flag(&mut args));
        assert_eq!(args, vec!["expansions.toml".to_string()]);

        let mut args = vec!["--json".to_string(), "expansions.toml".to_string()];
        assert!(take_json_flag(&mut args));
        assert_eq!(args, vec!["expansions.toml".to_string()]);

        let mut args = vec!["expansions.toml".to_string()];
        assert!(!take_json_flag(&mut args));
        assert_eq!(args, vec!["expansions.toml".to_string()]);
    }

    #[test]
    fn setup_modes_do_not_promote_unavailable_paths() {
        let no_devices = wayexpand_backend_selection::Capabilities {
            has_input_method_v2: false,
            has_virtual_keyboard: true,
            has_direct_libei_socket: false,
            has_dev_input: false,
            has_window_tracker: false,
            compositor: wayexpand_backend_selection::Compositor::Gnome,
        };
        let policy = OrganizationPolicy::default();
        assert!(setup_backend_for_mode("maximum", &no_devices, &policy).is_err());
        assert!(setup_backend_for_mode("experimental", &no_devices, &policy).is_err());

        let experimental = wayexpand_backend_selection::Capabilities {
            has_input_method_v2: true,
            ..no_devices
        };
        assert_eq!(
            setup_backend_for_mode("experimental", &experimental, &policy).unwrap(),
            "input-method"
        );
    }

    #[test]
    fn setup_modes_respect_runtime_backend_policy_names() {
        let capabilities = wayexpand_backend_selection::Capabilities {
            has_input_method_v2: true,
            has_direct_libei_socket: true,
            has_dev_input: true,
            ..Default::default()
        };
        let ibus_only = OrganizationPolicy {
            allowed_backends: vec!["input-method-v2".into()],
            ..Default::default()
        };
        assert_eq!(
            setup_backend_for_mode("experimental", &capabilities, &ibus_only).unwrap(),
            "input-method"
        );
        assert!(setup_backend_for_mode("maximum", &capabilities, &ibus_only).is_err());

        let raw_only = OrganizationPolicy {
            allowed_backends: vec!["libei".into()],
            ..Default::default()
        };
        assert!(setup_backend_for_mode("experimental", &capabilities, &raw_only).is_err());
        assert_eq!(
            setup_backend_for_mode("maximum", &capabilities, &raw_only).unwrap(),
            "evdev"
        );

        let wlroots_only = OrganizationPolicy {
            allowed_backends: vec!["wlroots".into()],
            ..Default::default()
        };
        assert!(setup_backend_for_mode("maximum", &capabilities, &wlroots_only).is_err());
    }

    #[test]
    fn capture_readiness_never_promotes_a_probe_to_verified() {
        let capabilities = wayexpand_backend_selection::Capabilities {
            has_input_method_v2: true,
            ..Default::default()
        };
        assert_eq!(
            capture_readiness(&capabilities, false, &OrganizationPolicy::default()),
            (
                "available-to-try",
                "a protocol or IBus probe succeeded; live client typing is not verified"
            )
        );
    }

    #[test]
    fn certification_does_not_call_stdin_only_selection_available() {
        assert_eq!(certification_selection_status("stdin"), "unsupported");
        assert_eq!(certification_selection_status("ibus"), "available");
        assert_eq!(certification_selection_status("none"), "failed");
    }

    #[test]
    fn human_capture_readiness_accepts_ibus_as_the_only_path() {
        assert!(capture_path_available(true, 0, 0));
        assert!(!capture_path_available(false, 0, 0));
    }

    #[test]
    fn doctor_health_requires_a_graphical_session() {
        assert!(!display_session_flags(false, false, false));
        assert!(display_session_flags(true, false, false));
        assert!(display_session_flags(false, true, false));
        assert!(display_session_flags(false, false, true));
    }

    #[test]
    fn control_socket_health_rejects_missing_regular_and_insecure_paths() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock must be after the Unix epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("wx-sock-{unique}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let missing = root.join("missing.sock");
        assert!(!existing_control_socket_is_healthy(&missing));

        let regular = root.join("regular.sock");
        std::fs::write(&regular, b"not a socket").unwrap();
        assert!(!existing_control_socket_is_healthy(&regular));

        let socket = root.join("wayexpand.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .unwrap();
        assert!(existing_control_socket_is_healthy(&socket));
        drop(listener);

        let insecure_parent = root.join("insecure");
        std::fs::create_dir(&insecure_parent).unwrap();
        std::fs::set_permissions(
            &insecure_parent,
            std::os::unix::fs::PermissionsExt::from_mode(0o777),
        )
        .unwrap();
        let insecure_socket = insecure_parent.join("wayexpand.sock");
        let insecure_listener = std::os::unix::net::UnixListener::bind(&insecure_socket).unwrap();
        std::fs::set_permissions(
            &insecure_socket,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .unwrap();
        assert!(!existing_control_socket_is_healthy(&insecure_socket));
        drop(insecure_listener);
        std::fs::set_permissions(
            &insecure_parent,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_ibus_is_available_to_try_not_certified() {
        let (state, detail) =
            capture_readiness(&Default::default(), true, &OrganizationPolicy::default());
        assert_eq!(state, "available-to-try");
        assert!(detail.contains("live client typing is not verified"));
    }

    #[test]
    fn capture_readiness_hides_policy_disallowed_paths() {
        let capabilities = wayexpand_backend_selection::Capabilities {
            has_input_method_v2: true,
            has_virtual_keyboard: true,
            has_direct_libei_socket: true,
            has_dev_input: true,
            ..Default::default()
        };
        let policy = OrganizationPolicy {
            allowed_backends: vec!["none".into()],
            ..Default::default()
        };
        let (state, _) = capture_readiness(&capabilities, true, &policy);
        assert!(matches!(state, "unavailable" | "not-probed"));
    }

    #[test]
    fn automatic_selection_health_requires_policy_permission() {
        let policy = OrganizationPolicy {
            allowed_backends: vec!["wlroots".into()],
            ..Default::default()
        };
        assert!(!automatic_selection_is_ready("evdev", "libei", &policy));
        assert!(automatic_selection_is_ready("evdev", "wlroots", &policy));
        assert!(!automatic_selection_is_ready("stdin", "wlroots", &policy));
    }

    #[test]
    fn output_probe_without_a_capture_source_is_not_a_ready_path() {
        let capabilities = wayexpand_backend_selection::Capabilities {
            has_virtual_keyboard: true,
            ..Default::default()
        };
        let (state, detail) =
            capture_readiness(&capabilities, false, &OrganizationPolicy::default());
        assert!(matches!(state, "unavailable" | "not-probed"));
        assert!(
            detail.contains("no non-invasive source and output path")
                || detail.contains("no active Wayland session was detected")
        );
    }

    #[test]
    fn preview_app_option_accepts_equals_and_separate_values() {
        let mut equals = vec![
            "--preview-app=thunderbird".to_string(),
            "config".to_string(),
        ];
        assert_eq!(
            take_option(&mut equals, "--preview-app").unwrap(),
            Some("thunderbird".into())
        );
        assert_eq!(equals, vec!["config"]);

        let mut separate = vec!["--preview-app".to_string(), "konsole".to_string()];
        assert_eq!(
            take_option(&mut separate, "--preview-app").unwrap(),
            Some("konsole".into())
        );
        assert!(separate.is_empty());
    }

    #[test]
    fn exit_codes_follow_typed_categories_not_error_wording() {
        assert_eq!(
            exit_code_for(&usage_error("daemon socket unavailable")),
            EXIT_USAGE
        );
        assert_eq!(
            exit_code_for(&config_error("daemon socket unavailable")),
            EXIT_CONFIG
        );
        assert_eq!(
            exit_code_for(&daemon_error("configuration invalid: parse error")),
            EXIT_DAEMON
        );
        assert_eq!(
            exit_code_for(&anyhow::anyhow!("usage: this is unclassified")),
            1
        );
    }
}
