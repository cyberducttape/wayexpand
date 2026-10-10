mod args;
mod backup;
mod build_info;
mod commands;
mod doctor;
mod errors;
mod setup;
mod sync;

use anyhow::{bail, Context, Error, Result};
use backup::{create_backup, default_backup_destination};
use commands::config::{
    backup_command, edit_command, fleet_command, import_command, list_command, pack_command,
    preview_command, schema_command, search_command, set_enabled_command, set_mode_command,
    stats_command, sync_command, test_command, test_hotkey_command, validate_command,
};
use commands::daemon::{control_command, insert_command};
use commands::system::{
    backend_command, certify_command, doctor_command, explain_backend_command, explain_command,
    portal_command, setup_command,
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
use doctor::support::support_bundle;
use errors::{
    config_error, config_load_error, daemon_error, exit_code_for, normalize_error, usage_error,
};
use setup::{
    configure_setup_backend, libei_portal_candidate, prompt_mode_choice, recommended_setup_backend,
    setup_backend_for_mode,
};
use std::{
    env, fs,
    io::{self, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use unicode_segmentation::UnicodeSegmentation;

use wayexpand_backend_ibus::engine_available as ibus_engine_available;
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_libei::{portal_token_path, reset_portal_token};
use wayexpand_backend_selection::{
    explain_auto_selection, probe_capabilities, recommended_route, route_allowed_by_policy,
    setup_backend_allowed,
};
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{
    all_capabilities, default_config_path, discover_backends, import_pack, inspect_pack,
    BackendKind, Config, ExpansionEngine, FleetConfig, InputEvent, MatchMode, OrganizationPolicy,
    CONTROL_STATUS_SCHEMA,
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
        Some("schema") => schema_command(args)?,
        Some("validate") => validate_command(args)?,
        Some("import") => import_command(args)?,
        Some("pack") => pack_command(args)?,
        Some("set-enabled") => set_enabled_command(args)?,
        Some("set-mode") => set_mode_command(args)?,
        Some("backup") => backup_command(args)?,
        Some("edit") => edit_command(args)?,
        Some("setup") => setup_command(args)?,
        Some("doctor") => doctor_command(args)?,
        Some("explain") => explain_command(args)?,
        Some("stats") => stats_command(args)?,
        Some("sync") => sync_command(args)?,
        Some("certify") => certify_command(args)?,
        Some("support-bundle") => support_bundle(args)?,
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
            (
                "explain <text> [--json] [--offline] [--app ID] [config]",
                "Explain why typed text would or would not expand",
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
                "sync [init [--remote URL]|status] [--json] [config]",
                "Synchronize the snippet library with Git (optional)",
            ),
            (
                "stats [--json] [--days N] [--clear] [config]",
                "Local usage: expansions, keystrokes saved, unused snippets",
            ),
            (
                "search <query> [--json] [config]",
                "Search triggers, descriptions, and tags",
            ),
            (
                "schema",
                "Print the stable configuration schema for authoring tools",
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
                "import espanso <file> [--strict] [--report-json]",
                "Convert an Espanso YAML file to TOML on stdout",
            ),
            (
                "pack inspect|import|verify|sign <directory> [--signers F] [--key F]",
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
                "support-bundle [--output FILE]",
                "Write redacted diagnostic JSON for support",
            ),
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
        build_info::VERSION
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

fn control_operation(operation: wayexpand_core::DaemonOperation) -> Result<String> {
    wayexpand_core::DaemonClient::from_environment()
        .and_then(|client| client.execute(operation))
        .map_err(|error| daemon_error(error.to_string()))
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
