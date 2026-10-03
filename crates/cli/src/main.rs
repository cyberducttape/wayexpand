mod args;
mod backup;
mod build_info;
mod commands;
mod doctor;
mod errors;
mod setup;

use anyhow::{bail, Context, Error, Result};
use backup::{create_backup, default_backup_destination};
use commands::config::{
    backup_command, edit_command, fleet_command, import_command, list_command, pack_command,
    preview_command, search_command, set_enabled_command, set_mode_command, test_command,
    test_hotkey_command, validate_command,
};
use commands::daemon::{control_command, insert_command};
use commands::system::{
    backend_command, certify_command, doctor_command, explain_backend_command, portal_command,
    setup_command,
};
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
// Path-importable, so command modules declared above can use it.
pub(crate) use usage_bail;

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
        Some("test") => test_command(args)?,
        Some("test-hotkey") => test_hotkey_command(args)?,
        Some("preview") => preview_command(args)?,
        Some("list") => list_command(args)?,
        Some("search") => search_command(args)?,
        Some("validate") => validate_command(args)?,
        Some("import") => import_command(args)?,
        Some("pack") => pack_command(args)?,
        Some("set-enabled") => set_enabled_command(args)?,
        Some("set-mode") => set_mode_command(args)?,
        Some("backup") => backup_command(args)?,
        Some("edit") => edit_command(args)?,
        Some("setup") => setup_command(args)?,
        Some("doctor") => doctor_command(args)?,
        Some("certify") => certify_command(args)?,
        Some("backend") => backend_command(args)?,
        Some("explain-backend") => explain_backend_command(args)?,
        Some("fleet") => fleet_command(args)?,
        Some("portal") => portal_command(args)?,
        Some(requested @ ("status" | "reload" | "pause" | "resume" | "stop")) => {
            control_command(requested, args)?
        }
        Some("insert") => insert_command(args)?,
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
