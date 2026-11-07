mod args;
mod backup;
mod build_info;
mod commands;
mod diagnostics;
mod doctor;
mod errors;
mod gnome_extension;
mod gnome_input_sources;
mod services;
mod setup;
mod setup_ibus;
mod sync;

use anyhow::{bail, Context, Error, Result};
use backup::{create_backup, default_backup_destination};
use commands::config::{
    backup_command, edit_command, fleet_command, import_command, list_command, pack_command,
    preview_command, schema_command, search_command, set_enabled_command, set_mode_command,
    stats_command, sync_command, test_command, test_hotkey_command, validate_command,
};
use commands::context::context_command;
use commands::daemon::{control_command, insert_command};
use commands::metrics::metrics_command;
use commands::new::new_command;
use commands::profile::profile_command;
use commands::system::{
    backend_command, certify_command, doctor_command, explain_backend_command, explain_command,
    policy_command, portal_command, setup_command, upgrade_status_command,
};
use commands::Args;
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
    config_error, config_load_error, daemon_error, exit_code_for, exit_quietly_when_stdout_closes,
    normalize_error, usage_error,
};
#[cfg(test)]
use setup::recommended_setup_backend;
use setup::{
    configure_setup_backend, libei_portal_candidate, prompt_mode_choice,
    recommended_setup_backend_with_ibus, setup_backend_for_mode,
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

use wayexpand_backend_ibus::{
    engine_available as ibus_engine_available, engine_installed as ibus_engine_installed,
};
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
    exit_quietly_when_stdout_closes();
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
        Some("new") => new_command(args)?,
        Some("workflow") => commands::workflow::workflow_command(args)?,
        Some("list") => list_command(args)?,
        Some("search") => search_command(args)?,
        Some("schema") => schema_command(args)?,
        Some("validate") => validate_command(args)?,
        Some("import") => import_command(args)?,
        Some("pack") => pack_command(args)?,
        Some("profile") => profile_command(args)?,
        Some("context") => context_command(args)?,
        Some("library") => commands::library::library_command(args)?,
        Some("set-enabled") => set_enabled_command(args)?,
        Some("set-mode") => set_mode_command(args)?,
        Some("backup") => backup_command(args)?,
        Some("edit") => edit_command(args)?,
        Some("setup") => setup_command(args)?,
        Some("install-check") => commands::install::install_check(args)?,
        Some("repair") => commands::install::repair(args)?,
        Some("reset") => commands::reset::reset_command(args)?,
        Some("restore") => commands::restore::restore_command(args)?,
        Some("health") => commands::health::health_command(args)?,
        Some("compatibility-report") => commands::compatibility::compatibility_report(args)?,
        Some("check") => commands::check::check_command(args)?,
        Some("service") => commands::service::service_command(args)?,
        Some("config") => commands::config_inspect::config_command(args)?,
        Some("doctor") => doctor_command(args)?,
        Some("debug") => commands::debug::debug_command(args)?,
        Some("policy") => policy_command(args)?,
        Some("upgrade-status") => upgrade_status_command(args)?,
        Some("upgrade") => commands::system::upgrade_command(args)?,
        Some("explain") => explain_command(args)?,
        Some("stats") => stats_command(args)?,
        Some("sync") => sync_command(args)?,
        Some("certify") => certify_command(args)?,
        Some("support-bundle") => support_bundle(args)?,
        Some("support") => support_bundle(args)?,
        Some("backend") => backend_command(args)?,
        Some("explain-backend") => explain_backend_command(args)?,
        Some("fleet") => fleet_command(args)?,
        Some("metrics") => metrics_command(args)?,
        Some("portal") => portal_command(args)?,
        Some("ibus") => commands::ibus::ibus_command(args)?,
        Some(requested @ ("status" | "reload" | "pause" | "resume" | "stop")) => {
            control_command(requested, args)?
        }
        Some("insert") => insert_command(args)?,
        Some("help") => help_command(args)?,
        Some("--help") | Some("-h") | None => print_help(),
        Some(command) => usage_bail!("unknown command {command:?}; try `wayexpand help`"),
    }
    Ok(())
}

fn help_command(mut args: Args) -> Result<()> {
    match args.next().as_deref() {
        None => print_help(),
        Some("finding") => {
            let id = args
                .next()
                .ok_or_else(|| usage_error("usage: wayexpand help finding <WX-ID>"))?;
            if args.next().is_some() {
                usage_bail!("usage: wayexpand help finding <WX-ID>");
            }
            let (summary, action, doc) = match id.as_str() {
                "WX-PORTAL-001" => ("Desktop input-emulation permission is required.", "Run `wayexpand backend test --injection` in a terminal and approve the portal prompt only if expected.", "docs/BACKENDS.md#libei-and-the-remotedesktop-portal"),
                "WX-IBUS-001" => ("The WayExpand IBus component is not registered.", "Run `wayexpand install-check` then `wayexpand setup --backend ibus`.", "docs/IBUS_ENGINE.md"),
                "WX-IBUS-002" => ("The IBus engine is registered but not selected.", "Select WayExpand in the desktop input-method menu, then run `wayexpand ibus status`.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-IBUS-003" => ("The selected IBus engine has not observed text from a normal field.", "Run `wayexpand ibus watch --seconds 30`, focus a plain text field, and type harmless text; this reports counters only.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-IBUS-004" => ("The IBus daemon is not responding, so browser and other IBus clients cannot reach WayExpand.", "Restore the desktop-managed IBus session, then run `wayexpand ibus status --json` and confirm the daemon check passes.", "docs/TROUBLESHOOTING.md#browser-text-fields-do-not-expand"),
                "WX-IBUS-RELOAD-001" => ("IBus could not install a configuration generation consistently across its contexts.", "Check `wayexpand ibus status --json` for published/requested generations and stale-context count. Restart the WayExpand IBus engine from the desktop input-source controls, then rerun `wayexpand ibus status`.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-IBUS-005" => ("The IBus engine reported an output failure; target text may be partial.", "Inspect the target field and do not retry blindly. After recovery, run `wayexpand debug --test 'YOUR_TRIGGER'` with a harmless trigger and verify the visible result.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-IBUS-006" => ("IBus emitted replacement actions, but the target application did not acknowledge the resulting text.", "Inspect the target field and confirm the exact result manually; WayExpand cannot verify this client-side effect.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-IBUS-007" => ("No replacement has been observed in the current IBus engine session.", "Focus a supported text field and run `wayexpand debug --test 'YOUR_TRIGGER'` with a harmless enabled trigger, then inspect the field.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-IBUS-SETUP-001" => ("IBus setup changed configuration, but a required desktop-route check failed.", "Run `wayexpand ibus status --json`, repair the first failed check, and rerun the status command before testing a real field.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-IBUS-SETUP-002" => ("IBus is selected while a global WayExpand capture route also expands the same applications.", "Limit the evdev route to configured terminals with `wayexpand setup --backend ibus --terminals`, or stop the overlapping capture service, then rerun `wayexpand ibus status`.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-IBUS-TOOLKIT-001" => ("Legacy GTK/Qt environment overrides conflict with Plasma's native IBus Wayland route.", "Remove GTK_IM_MODULE and QT_IM_MODULE from Plasma login files, run `systemctl --user unset-environment GTK_IM_MODULE QT_IM_MODULE`, log out of Plasma and back in, then run `wayexpand ibus status --json`.", "docs/TROUBLESHOOTING.md#browser-text-fields-do-not-expand"),
                "WX-KWIN-IBUS-001" => ("KWin's IBus Wayland bridge is not running, so native Wayland applications cannot send text to WayExpand.", "Select IBus Wayland in System Settings → Keyboard → Virtual Keyboard, log out of Plasma and back in, then run `wayexpand ibus status --json`. If the helper keeps crashing, update or repair the system IBus package and inspect `coredumpctl info ibus-ui-gtk3`.", "docs/TROUBLESHOOTING.md#browser-text-fields-do-not-expand"),
                "WX-KWIN-IBUS-002" => ("The IBus Wayland helper crashed recently. It may have restarted, so check the live route and retest a real application.", "Inspect `coredumpctl info <PID>`, install the latest IBus package supported by your distribution, then log out of Plasma and back in. Run `wayexpand ibus status --json` and verify a harmless expansion in the affected application. If the crash recurs, report the backtrace and exact IBus package version to your distribution or IBus maintainers.", "docs/TROUBLESHOOTING.md#browser-text-fields-do-not-expand"),
                "WX-POLICY-001" => ("Replacement is refused by local compatibility settings or organization policy.", "Run `wayexpand ibus status`; only if you accept the non-atomic IBus replacement behavior, authorize it through interactive `wayexpand setup --backend ibus`. Run `wayexpand policy check --json` to inspect organization restrictions.", "docs/TROUBLESHOOTING.md#wx-policy-001"),
                "WX-GNOME-IM-001" => ("WayExpand is missing from GNOME's configured input sources.", "Run `wayexpand repair --gnome-input-source`, then `wayexpand doctor --gnome` to verify registration and activation.", "docs/TROUBLESHOOTING.md#wx-gnome-im-001"),
                "WX-GNOME-IM-002" => ("GNOME's live input-source manager has not confirmed WayExpand as active.", "Run `wayexpand repair --gnome-input-source`, then `wayexpand doctor --gnome` to independently verify the active source and IBus engine.", "docs/TROUBLESHOOTING.md#wx-gnome-im-001"),
                "WX-GNOME-IM-003" => ("No safe-field input activity was observed from the GNOME IBus engine.", "Run `wayexpand ibus watch --seconds 30`, select WayExpand in GNOME, and type harmless text in a normal text field.", "docs/IBUS_ENGINE.md#diagnostics"),
                "WX-GNOME-IM-004" => ("The GNOME test did not establish a successful expansion in the target application.", "Run `wayexpand debug --test 'YOUR_TRIGGER'` with a harmless enabled trigger, then confirm the exact visible result; do not retry uncertain output.", "docs/TROUBLESHOOTING.md#wx-gnome-im-001"),
                "WX-GNOME-IM-005" => ("The test could not verify the final text in the target application.", "Inspect the target field manually and do not automatically replay the insertion.", "docs/TROUBLESHOOTING.md#wx-gnome-im-001"),
                "WX-CONFIG-001" => ("The active configuration could not be loaded.", "Run `wayexpand validate` and correct the reported issue; back up before manual edits.", "docs/TROUBLESHOOTING.md#configuration-invalid"),
                "WX-INPUT-001" => ("The active capture route cannot currently match text.", "Run `wayexpand debug` and inspect layout, capture, policy, and service findings.", "docs/TROUBLESHOOTING.md#wx-input-001"),
                "WX-OUTPUT-001" => ("The output route is unavailable or stalled.", "Inspect `wayexpand status`; do not retry an insertion with an uncertain result.", "docs/TROUBLESHOOTING.md#wx-output-001"),
                "WX-SERVICE-001" | "WX-SERVICE-002" => ("A required user service is missing or not responding.", "Run `wayexpand install-check` and `wayexpand setup --check`; restart only the selected installed unit.", "docs/OPERATIONS.md#operational-health-and-setup-deadlines"),
                "WX-VERSION-001" => ("The CLI and running daemon versions differ.", "Run `wayexpand upgrade --check`; apply only after reviewing configuration validity.", "docs/OPERATIONS.md#operational-health-and-setup-deadlines"),
                _ => bail!("unknown diagnostic finding ID {id:?}; see `wayexpand debug --json` for current findings"),
            };
            println!("{id} · {summary}\nNext step: {action}\nReference: {doc}");
        }
        _ => usage_bail!("usage: wayexpand help [finding <WX-ID>]"),
    }
    Ok(())
}

/// `(usage, summary)` rows for `wayexpand help`, grouped by section.
const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Getting started",
        &[
            (
                "setup --check [--json] | --apply [--mode recommended|maximum|experimental] [--terminals] [--yes] [--accept-non-atomic-replacement] [--install-system-integration]",
                "Check setup readiness or configure a reviewed input route",
            ),
            ("install-check [--json]", "Audit installed WayExpand components"),
            ("health [--json]", "Report operational readiness with a stable monitoring exit code"),
            ("compatibility-report [--json]", "Generate a reproducible desktop compatibility report"),
            ("check --all [--json] [config]", "Run static configuration, template, file, policy, and trigger checks without executing commands"),
            ("service status [--json] | restart --yes", "Inspect managed services or restart only the active route"),
            ("config show|validate [--json] [config] | diff [--json] OLD NEW", "Inspect, validate, or compare configuration files"),
            ("repair --check|--apply [--json] | --gnome-input-source [--json]", "Inspect installation repairs or register and activate the GNOME IBus source"),
            ("reset --integration|--services|--preferences [--check|--apply --yes] [--json]", "Preview and apply a scoped recovery reset"),
            ("restore <backup> [--check | --apply --yes --expected-current=HASH --expected-backup=HASH] [--json] [config]", "Review and safely restore a configuration backup"),
            (
                "doctor [--json] [config] | --gnome [--json]",
                "Diagnose configuration, backends and the running daemon",
            ),
            ("debug [--json] [--trigger TEXT] [--live [--seconds 1..300]] | --test TRIGGER [--application-class CLASS --focus-confirmed [--observe-only]] | --gnome --interactive", "Explain runtime readiness or guide a harmless insertion test"),
            (
                "policy inspect|check [--file PATH] [--allow-audit-only] [--json]",
                "Inspect policy provenance or fail CI when restrictions are not enforced",
            ),
            (
                "doctor --security [--profile workstation|hardened|strict] [--json] [config]",
                "Inspect sandbox readiness against an explicit protection profile",
            ),
            (
                "doctor --certify [--json]",
                "Run isolated live insertion checks in a private KWin",
            ),
            (
                "upgrade-status [--json]",
                "Check whether the running daemon matches this CLI",
            ),
            ("upgrade --check|--apply|--rollback [--json]", "Verify or complete a staged runtime upgrade"),
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
        "Terminal context",
        &[
            (
                "context integration bash|zsh|fish",
                "Print an opt-in shell prompt integration",
            ),
            (
                "context report|status|clear",
                "Report and inspect fresh terminal, remote, project, and runtime context",
            ),
            (
                "context render <trigger> [--set NAME=VALUE]... [--json]",
                "Render text for the current shell and SSH context; never execute it",
            ),
        ],
    ),
    (
        "Snippets",
        &[
            (
                "list [--json] [config]",
                "List configured expansions and hotkeys",
            ),
            (
                "sync [init [--remote URL]|status] [--json] [--allow-executable --approved-revision FULL_COMMIT] [config]",
                "Synchronize a static snippet library with Git (optional)",
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
                "new [<name> [--set NAME=VALUE]... [-o FILE|--stdout] [--gui] [--force] [--validate]]",
                "Fill in a file template (Dockerfile, playbook...) and write it",
            ),
            (
                "workflow insert|generate|run ...",
                "Choose explicit insertion, reviewed file generation, or an approved broker action",
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
                "insert [--verified] <trigger>",
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
                "validate [--fleet] [--json] [--deny-risks] [config]",
                "Validate configuration",
            ),
            (
                "backup [config] [destination]",
                "Create a non-overwriting config backup",
            ),
            (
                "import <espanso|autokey> <file-or-directory> [--strict] [--report-json] [--add [--config PATH]]",
                "Convert to TOML, or merge into a library after smoke-testing imported triggers",
            ),
            (
                "pack lint|test|diff|search|inspect|install|outdated|update|lock|sync|list|path|remove|rollback|import|verify|sign ...",
                "Lint, test, review, sign, distribute, and roll back local snippet packs",
            ),
            (
                "profile list|create|switch|remove|assign-app|unassign-app ...",
                "Switch validated configuration profiles, or assign them to apps",
            ),
            (
                "library add [--approved-revision FULL_COMMIT]|list|path|search|remove ...",
                "Browse and pin offline Git snippet catalogs",
            ),
        ],
    ),
    (
        "Backends and operations",
        &[
            ("backend list [--json] | profile [ID|--current] [--json] | test --capture|--injection", "Inspect live route assessments, desktop guidance, or run an explicit one-shot route test"),
            ("explain-backend", "Explain automatic backend selection"),
            ("certify [--json]", "Run local compatibility certification"),
            (
                "support [--gnome] [--output FILE]",
                "Write a sanitized GNOME-focused support report",
            ),
            (
                "support-bundle [--output FILE]",
                "Write redacted diagnostic JSON for support",
            ),
            (
                "fleet status [--json]",
                "Show merged fleet configuration status",
            ),
            (
                "metrics [--output FILE] [config]",
                "Write content-free Prometheus metrics (textfile collector)",
            ),
            (
                "portal status|reset",
                "Inspect or remove the libei portal token",
            ),
            (
                "ibus status [--json]|watch [--seconds N]",
                "Check the IBus route link by link; explain why a field expands or not",
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

pub(crate) fn prompt_yes_no(prompt: &str, default: bool) -> Result<bool> {
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

/// Make untrusted text safe to show in a terminal: control characters other
/// than tab are shown as escapes so content under review cannot move the
/// cursor, clear lines, or otherwise hide what it contains.
pub(crate) fn terminal_safe(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() && character != '\t' {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
