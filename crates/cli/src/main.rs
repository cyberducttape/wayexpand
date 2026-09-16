use anyhow::{bail, Context, Result};
use std::{
    env, fs,
    io::{Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTROL_RESPONSE_BYTES: usize = 4096;
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{
    default_config_path, discover_backends, import_espanso, Config, ExpansionEngine, InputEvent,
    MatchMode,
};

/// Pulls the first `--json` flag out of `args`, wherever it appears, so
/// subcommands accept it in any position rather than one fixed slot.
fn take_json_flag(args: &mut Vec<String>) -> bool {
    if let Some(position) = args.iter().position(|arg| arg == "--json") {
        args.remove(position);
        true
    } else {
        false
    }
}

const EXIT_USAGE: i32 = 2;
const EXIT_CONFIG: i32 = 3;
const EXIT_DAEMON: i32 = 4;

/// Classifies an error by the message conventions this CLI already uses
/// consistently ("usage: ...", "configuration invalid", daemon-socket
/// context messages) so callers can distinguish failure causes without
/// parsing free-form text, while avoiding a bespoke error type per site.
fn exit_code_for(error: &anyhow::Error) -> i32 {
    let message = error.to_string();
    if message.starts_with("usage:") || message.starts_with("unknown command") {
        EXIT_USAGE
    } else if message.contains("configuration invalid") {
        EXIT_CONFIG
    } else if message.contains("connecting to")
        || message.contains("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required")
        || message.contains("daemon control response")
        || message.contains("daemon returned a non-UTF-8")
    {
        EXIT_DAEMON
    } else {
        1
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:?}");
        std::process::exit(exit_code_for(&error));
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--version") | Some("-V") | Some("version") => {
            println!("wayexpand {}", env!("CARGO_PKG_VERSION"));
        }
        Some("test") => {
            let trigger = args
                .next()
                .context("usage: wayexpand test <text> [--json] [config]")?;
            let mut rest: Vec<String> = args.collect();
            let requested_json = take_json_flag(&mut rest);
            if rest.len() > 1 {
                bail!("usage: wayexpand test <text> [--json] [config]");
            }
            let path = rest
                .into_iter()
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            let config = Config::load(path).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
            })?;
            let mut engine = ExpansionEngine::new(config).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
            })?;
            let mut results = engine.process(InputEvent::Text(trigger));
            results.extend(engine.process(InputEvent::Boundary));
            if requested_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "matched": !results.is_empty(),
                        "results": results.iter().map(|result| serde_json::json!({
                            "trigger_characters": result.trigger.chars().count(),
                            "erase_characters": result.erase_chars,
                            "replacement_bytes": result.insert.len(),
                            "replacement": result.insert,
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
            let chord_text = args
                .next()
                .context("usage: wayexpand test-hotkey <chord> [--json] [config]")?;
            let mut rest: Vec<String> = args.collect();
            let requested_json = take_json_flag(&mut rest);
            if rest.len() > 1 {
                bail!("usage: wayexpand test-hotkey <chord> [--json] [config]");
            }
            let path = rest
                .into_iter()
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            let chord = wayexpand_core::KeyChord::parse(&chord_text)
                .map_err(|error| anyhow::anyhow!("invalid hotkey chord: {error}"))?;
            let config = Config::load(&path).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
            })?;
            let engine = ExpansionEngine::new(config).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
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
            let trigger = args
                .next()
                .context("usage: wayexpand preview <trigger> [--json] [config]")?;
            let mut rest: Vec<String> = args.collect();
            let requested_json = take_json_flag(&mut rest);
            if rest.len() > 1 {
                bail!("usage: wayexpand preview <trigger> [--json] [config]");
            }
            let path = rest
                .into_iter()
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            let config = Config::load(&path).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
            })?;
            let mut engine = ExpansionEngine::new(config).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
            })?;
            let mut results = engine.process(InputEvent::Text(trigger));
            results.extend(engine.process(InputEvent::Boundary));
            match results.last() {
                Some(result) => {
                    if requested_json {
                        println!("{}", serde_json::json!({
                            "matched": true,
                            "trigger": result.trigger,
                            "replacement": result.insert,
                        }));
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
                bail!("usage: wayexpand list [--json] [config]");
            }
            let path = rest
                .into_iter()
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            let config = Config::load(&path).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
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
                println!("{} expansion(s) in {}", config.expansion.len(), path.display());
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
                .context("usage: wayexpand search <query> [--json] [config]")?;
            let mut rest: Vec<String> = args.collect();
            let requested_json = take_json_flag(&mut rest);
            if rest.len() > 1 {
                bail!("usage: wayexpand search <query> [--json] [config]");
            }
            let path = rest
                .into_iter()
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            let query_lower = query.to_lowercase();
            let config = Config::load(&path).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
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
            let path = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            if args.next().is_some() {
                bail!("usage: wayexpand validate [config]");
            }
            let config = Config::load(&path).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
            })?;
            println!(
                "configuration valid: {} expansion(s), buffer limit {}",
                config.expansion.len(),
                config.settings.max_buffer_chars
            );
        }
        Some("import") => {
            let format = args
                .next()
                .context("usage: wayexpand import espanso <file>")?;
            let source = args
                .next()
                .context("usage: wayexpand import espanso <file>")?;
            if args.next().is_some() || format != "espanso" {
                bail!("usage: wayexpand import espanso <file>");
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
            let trigger = args
                .next()
                .context("usage: wayexpand set-enabled <trigger> <on|off> [config]")?;
            let value = args
                .next()
                .context("usage: wayexpand set-enabled <trigger> <on|off> [config]")?;
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
                bail!("usage: wayexpand set-enabled <trigger> <on|off> [config]");
            }
            let mut config = Config::load(&path).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
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
                anyhow::anyhow!("configuration invalid after edit: {}", error.safe_summary())
            })?;
            config.save_atomic(&path).map_err(|error| {
                anyhow::anyhow!("could not save configuration: {}", error.safe_summary())
            })?;
            println!(
                "{} {}",
                if enabled { "enabled" } else { "disabled" },
                trigger
            );
        }
        Some("set-mode") => {
            let trigger = args
                .next()
                .context("usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]")?;
            let value = args
                .next()
                .context("usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]")?;
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
                bail!("usage: wayexpand set-mode <trigger> <immediate|word-boundary> [config]");
            }
            let mut config = Config::load(&path).map_err(|error| {
                anyhow::anyhow!("configuration invalid: {}", error.safe_summary())
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
                anyhow::anyhow!("configuration invalid after edit: {}", error.safe_summary())
            })?;
            config.save_atomic(&path).map_err(|error| {
                anyhow::anyhow!("could not save configuration: {}", error.safe_summary())
            })?;
            println!("{} {}", value, trigger);
        }
        Some("backup") => {
            let source = args.next().map(PathBuf::from).unwrap_or_else(default_config_path);
            let destination = args.next().map(PathBuf::from).unwrap_or_else(|| {
                let mut path = source.clone();
                path.set_extension("toml.bak");
                path
            });
            if args.next().is_some() {
                bail!("usage: wayexpand backup [config] [destination]");
            }
            if destination.exists() {
                bail!("refusing to overwrite existing backup {}", destination.display());
            }
            let metadata = fs::metadata(&source)
                .with_context(|| format!("reading configuration {}", source.display()))?;
            if !metadata.is_file() {
                bail!("configuration is not a regular file: {}", source.display());
            }
            fs::copy(&source, &destination).with_context(|| {
                format!("creating configuration backup {}", destination.display())
            })?;
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o600))?;
            println!("created configuration backup {}", destination.display());
        }
        Some("doctor") => {
            let mut rest: Vec<String> = args.collect();
            let requested_json = take_json_flag(&mut rest);
            if rest.len() > 1 {
                bail!("usage: wayexpand doctor [--json] [config]");
            }
            let config_path = rest
                .into_iter()
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_config_path);
            if requested_json {
                let healthy = print_json_diagnostics(&config_path)?;
                if !healthy {
                    bail!("doctor found configuration or control-socket problems");
                }
                return Ok(());
            }
            println!(
                "Session: {}",
                if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                    "Wayland"
                } else {
                    "not detected"
                }
            );
            let config_ok = print_config_diagnostics(&config_path);
            let control_socket_ok = print_control_socket_diagnostics();
            let capture_ready = print_backend_diagnostics();
            if !config_ok || !control_socket_ok || !capture_ready {
                bail!("doctor found configuration, runtime, or backend problems");
            }
        }
        Some("backend") => {
            print_backend_diagnostics();
        }
        Some(requested @ ("status" | "reload" | "pause" | "resume" | "stop")) => {
            let status_argument = args.next();
            let requested_json = status_argument.as_deref() == Some("--json");
            if status_argument.is_some() && !requested_json {
                bail!("usage: wayexpand {requested} [--json]");
            }
            if args.next().is_some() {
                bail!("usage: wayexpand {requested} [--json]");
            }
            let path = std::env::var_os("WAYEXPAND_SOCKET")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock")))
                .context("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required")?;
            let mut stream = UnixStream::connect(&path)
                .with_context(|| format!("connecting to {}", path.display()))?;
            stream.set_read_timeout(Some(CONTROL_IO_TIMEOUT))?;
            stream.set_write_timeout(Some(CONTROL_IO_TIMEOUT))?;
            writeln!(stream, "{requested}")?;
            let mut response = Vec::with_capacity(MAX_CONTROL_RESPONSE_BYTES);
            stream
                .take((MAX_CONTROL_RESPONSE_BYTES + 1) as u64)
                .read_to_end(&mut response)?;
            if response.len() > MAX_CONTROL_RESPONSE_BYTES {
                bail!("daemon control response exceeded {MAX_CONTROL_RESPONSE_BYTES} bytes");
            }
            let response = String::from_utf8(response)
                .context("daemon returned a non-UTF-8 control response")?;
            if requested_json {
                println!("{}", status_as_json(&response)?);
            } else {
                print!("{response}");
            }
        }
        Some("help") | Some("--help") | Some("-h") | None => println!(
            "WayExpand {} — secure Wayland text expansion\n\nusage: wayexpand <command> [options]\n\ncommands:\n  test <text> [config]                         Simulate input and print a match\n  test-hotkey <chord> [--json] [config]       Resolve a hotkey without executing it\n  preview <trigger> [--json] [config]          Preview a replacement\n  list [--json] [config]                       List configured expansions and hotkeys\n  search <query> [--json] [config]             Search triggers, descriptions, and tags\n  validate [config]                            Validate configuration\n  import espanso <file>                        Import an Espanso YAML file\n  set-enabled <trigger> <on|off> [config]     Enable or disable an expansion\n  set-mode <trigger> <mode> [config]           Set immediate or word-boundary matching\n  backup [config] [destination]                Create a non-overwriting config backup\n  doctor [--json] [config]                     Diagnose configuration and backends\n  backend                                      Show backend availability\n  status|reload|pause|resume|stop [--json]     Control a running daemon\n  help                                         Show this help\n  version                                      Print the installed version\n\nEnvironment: WAYEXPAND_CONFIG, WAYEXPAND_SOCKET, XDG_CONFIG_HOME, XDG_RUNTIME_DIR\nDefault config: {}",
            env!("CARGO_PKG_VERSION"),
            default_config_path().display()
        ),
        Some(command) => bail!("unknown command {command:?}; try `wayexpand help`"),
    }
    Ok(())
}

fn print_backend_diagnostics() -> bool {
    let mut capture_ready = false;
    for status in discover_backends() {
        println!("{:28} {:?} ({})", status.kind, status.state, status.detail);
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match WlrootsInjector::probe() {
            Ok(_) => println!("wlroots probe: virtual keyboard globals available"),
            Err(error) => println!("wlroots probe: unavailable ({error})"),
        }
        match InputMethodSource::probe() {
            Ok(_) => {
                capture_ready = true;
                println!("input-method-v2 probe: manager and seat connection succeeded")
            }
            Err(error) => println!("input-method-v2 probe: unavailable ({error})"),
        }
        if !capture_ready {
            println!(
                "Capture readiness: NOT READY (no supported global input source was detected)"
            );
            println!(
                "Next step: use a compositor with input-method-v2 support, or fall back to \
                 `--source=evdev` (requires `input` group membership; see SECURITY.md for the \
                 sensitive-field tradeoff) paired with `--backend=wlroots` or `--backend=libei`."
            );
        } else {
            println!("Capture readiness: READY");
        }
    }
    // Doctor is also used in CI and for validating a config outside a desktop
    // session. In that context there is no capture claim to validate.
    !std::env::var_os("WAYLAND_DISPLAY").is_some() || capture_ready
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
    let backends: Vec<_> = discover_backends()
        .into_iter()
        .map(|status| {
            serde_json::json!({
                "kind": status.kind.to_string(),
                "state": format!("{:?}", status.state),
                "detail": status.detail,
            })
        })
        .collect();
    let healthy = config_ok && (socket_path.is_none() || socket_exists);
    println!(
        "{}",
        serde_json::json!({
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
            },
            "backends": backends,
        })
    );
    Ok(healthy)
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
        }
        Err(error) => {
            println!("Control socket existing path: unreadable ({error})");
            valid = false;
        }
    }
    valid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_json_preserves_types_and_ignores_banner() {
        let value =
            status_as_json("running\nsource=stdin\npaused=true\nconfig_state=ok\n").unwrap();
        assert_eq!(value["response"], "running");
        assert_eq!(value["source"], "stdin");
        assert_eq!(value["paused"], true);
        assert_eq!(value["config_state"], "ok");
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
    fn exit_codes_classify_known_error_shapes() {
        assert_eq!(
            exit_code_for(&anyhow::anyhow!(
                "usage: wayexpand test <text> [--json] [config]"
            )),
            EXIT_USAGE
        );
        assert_eq!(
            exit_code_for(&anyhow::anyhow!(
                "unknown command \"bogus\"; try `wayexpand help`"
            )),
            EXIT_USAGE
        );
        assert_eq!(
            exit_code_for(&anyhow::anyhow!("configuration invalid: parse error")),
            EXIT_CONFIG
        );
        assert_eq!(
            exit_code_for(&anyhow::anyhow!(
                "connecting to /run/user/1000/wayexpand.sock"
            )),
            EXIT_DAEMON
        );
        assert_eq!(
            exit_code_for(&anyhow::anyhow!("some other unexpected failure")),
            1
        );
    }
}
