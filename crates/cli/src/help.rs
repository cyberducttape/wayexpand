//! The command registry behind `wayexpand help`, per-command help, typo
//! suggestions, and shell completions.
//!
//! Every command the CLI dispatches has one entry here. The main help shows
//! only names and one-line summaries, grouped by task; options live in each
//! command's own help (`wayexpand help <command>` or `<command> --help`).

use std::io::IsTerminal;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Group {
    Start,
    Snippets,
    Service,
    Troubleshoot,
    Desktop,
    Organization,
}

impl Group {
    const ORDER: [Group; 6] = [
        Group::Start,
        Group::Snippets,
        Group::Service,
        Group::Troubleshoot,
        Group::Desktop,
        Group::Organization,
    ];

    fn title(self) -> &'static str {
        match self {
            Group::Start => "Start here",
            Group::Snippets => "Snippets",
            Group::Service => "Background service",
            Group::Troubleshoot => "Troubleshooting",
            Group::Desktop => "Desktop integration",
            Group::Organization => "Teams and automation",
        }
    }
}

pub(crate) struct Command {
    pub(crate) name: &'static str,
    pub(crate) group: Group,
    pub(crate) summary: &'static str,
    /// Usage lines without the leading `wayexpand `.
    pub(crate) usage: &'static [&'static str],
    /// A short explanation of what the command does and its main options.
    pub(crate) details: &'static str,
    pub(crate) examples: &'static [(&'static str, &'static str)],
    /// Other names that run the same command.
    pub(crate) aliases: &'static [&'static str],
    /// Shown by `wayexpand help --all` but not in the main help.
    pub(crate) advanced: bool,
}

const fn command(
    name: &'static str,
    group: Group,
    summary: &'static str,
    usage: &'static [&'static str],
) -> Command {
    Command {
        name,
        group,
        summary,
        usage,
        details: "",
        examples: &[],
        aliases: &[],
        advanced: false,
    }
}

impl Command {
    const fn details(mut self, details: &'static str) -> Self {
        self.details = details;
        self
    }
    const fn examples(mut self, examples: &'static [(&'static str, &'static str)]) -> Self {
        self.examples = examples;
        self
    }
    const fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }
    const fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }
}

use Group::*;

pub(crate) const COMMANDS: &[Command] = &[
    // Start here
    command("setup", Start, "Check or configure how WayExpand reads and types text", &[
        "setup [--check [--json]]",
        "setup --apply [--mode recommended|maximum|experimental] [--backend ibus|input-method|evdev] [--terminals | --evdev-apps] [--yes] [--accept-non-atomic-replacement] [--install-system-integration]",
    ])
    .details("Without options, setup shows the compatibility modes, its recommendation and the ranked input routes, then asks before applying the plan. --check reports installed components, desktop integration, runtime readiness, and target insertion separately without changing anything; it never claims that text was verified in an application. --apply configures the chosen route; --mode picks how much compatibility to trade for safety, and --backend picks the input route directly. --terminals or --evdev-apps limit raw keyboard capture to the apps listed in settings. Setup never grants raw keyboard access or portal consent without asking.")
    .examples(&[
        ("wayexpand setup --check", "Check setup prerequisites and integration"),
        ("wayexpand setup --apply", "Configure the recommended route"),
        ("wayexpand setup --apply --backend ibus --terminals", "IBus for apps, evdev only in terminals"),
    ]),
    command("doctor", Start, "Diagnose your configuration, desktop and service", &[
        "doctor [--json] [config]",
        "doctor --gnome [--json]",
        "doctor --security [--profile workstation|hardened|strict] [--json] [config]",
        "doctor --certify [--json]",
    ])
    .details("Checks the configuration, the detected backends and the running service, and prints the fix for each problem it finds. --gnome checks the GNOME Shell extension and input source. --security compares sandboxing with a protection profile. --certify runs live insertion checks in a private, headless KWin that never touches your desktop.")
    .examples(&[
        ("wayexpand doctor", "Diagnose this installation"),
        ("wayexpand doctor --json", "The same, for scripts"),
    ]),
    command("edit", Start, "Open the snippet editor", &["edit [config]"])
        .details("Opens the graphical editor (wayexpand-gui) on your library or on the given configuration file."),
    // Snippets
    command("list", Snippets, "Show your snippets and hotkeys", &["list [--json] [config]"]),
    command("search", Snippets, "Find snippets by trigger, description or tag", &["search <query> [--json] [config]"])
        .examples(&[("wayexpand search signature", "Find snippets about signatures")]),
    command("insert", Snippets, "Type a snippet at the cursor", &["insert [--verified] <trigger>"])
        .details("Asks the running service to type the snippet for <trigger> into the focused application, as if you had typed the trigger. Bind it to a desktop shortcut for quick insertion. --verified ties the insert to the window focused now; the service refuses to type if focus has moved.")
        .examples(&[("wayexpand insert ';sig'", "Type your signature where the cursor is")]),
    command("shortcut", Snippets, "Give the Quick Picker a keyboard shortcut", &[
        "shortcut [status] [--json]",
        "shortcut set [KEYS]",
        "shortcut remove",
    ])
    .details("On KDE Plasma and GNOME, assigns a global shortcut that opens the Quick Picker (wayexpand-gui --picker) in any application; KEYS defaults to Ctrl+Alt+Space. A combination another action already uses is refused, never taken over. On other desktops, prints the command to bind in your keyboard settings.")
    .examples(&[
        ("wayexpand shortcut set", "Open the Quick Picker with Ctrl+Alt+Space"),
        ("wayexpand shortcut set Super+Period", "Use a different combination"),
    ]),
    command("new", Snippets, "Fill in a file template and write it", &[
        "new [--json] [--config FILE]",
        "new <name> [--set NAME=VALUE]... [-o PATH|--stdout] [--gui] [--force] [--validate] [--review] [--config FILE]",
        "new --show NAME [--json] [--config FILE]",
    ])
    .details("Without a name, lists the available templates (Dockerfile, Compose, Kubernetes, systemd units, nginx, playbooks, CI workflows and your own). With a name, asks for each field (or takes them from --set) and writes the file; --stdout prints it instead, and --force overwrites an existing file.")
    .examples(&[
        ("wayexpand new", "List templates"),
        ("wayexpand new playbook", "Fill in an Ansible playbook interactively"),
    ]),
    command("import", Snippets, "Bring in Espanso or AutoKey snippets", &[
        "import <espanso|autokey> <file-or-directory> [--strict] [--report-json] [--add [--config PATH]]",
    ])
    .details("Converts another expander's library to WayExpand's TOML and prints it. --add merges it into your library after smoke-testing the imported triggers; --strict stops at the first item that cannot be converted exactly.")
    .examples(&[("wayexpand import espanso ~/.config/espanso --add", "Merge your Espanso library")]),
    command("stats", Snippets, "Your usage: expansions, keystrokes saved, unused snippets", &["stats [--json] [--days N] [--clear] [config]"]),
    command("set-enabled", Snippets, "Turn a snippet on or off", &["set-enabled <trigger> <on|off> [config]"]).advanced(),
    command("set-mode", Snippets, "Expand immediately or only at a word boundary", &["set-mode <trigger> <immediate|word-boundary> [config]"]).advanced(),
    command("test", Snippets, "Show what typed text would expand to", &["test <text> [--json] [config]"]).advanced(),
    command("preview", Snippets, "Preview a snippet's replacement", &["preview <trigger> [--preview-app APP] [--json] [config]"]).advanced(),
    command("test-hotkey", Snippets, "Resolve a hotkey without running it", &["test-hotkey <chord> [--json] [config]"]).advanced(),
    command("validate", Snippets, "Validate a configuration file", &["validate [--fleet] [--json] [--deny-risks] [config]"]).advanced(),
    command("check", Snippets, "Run every static check without executing commands", &["check --all [--json] [config]"]).advanced(),
    command("config", Snippets, "Show, validate or compare configuration files", &[
        "config show [--json] [config]",
        "config validate [--json] [config]",
        "config diff [--json] OLD NEW",
    ]).advanced(),
    command("backup", Snippets, "Back up your configuration", &["backup [config] [destination]"])
        .details("Writes a copy of the configuration next to it, or to destination; it never overwrites an existing file."),
    command("restore", Snippets, "Review and restore a configuration backup", &[
        "restore <backup> [--check] [--json] [config]",
            "restore <backup> --apply --yes --expected-current=HASH --expected-backup=HASH [--json] [config]",
    ])
    .details("--check shows what would change. --apply needs the hashes --check printed, so a file that changed in between is never overwritten.")
    .advanced(),
    command("sync", Snippets, "Synchronize your library with Git", &[
        "sync [init [--remote URL]|status] [--json] [--allow-executable --approved-revision FULL_COMMIT] [config]",
    ]).advanced(),
    command("profile", Snippets, "Switch between configuration profiles", &[
        "profile list [config]",
        "profile create|switch|remove <name> [config]",
        "profile assign-app <app-id> <name> [config]",
        "profile unassign-app <app-id> [config]",
    ]).advanced(),
    command("pack", Snippets, "Lint, test, sign, install and roll back snippet packs", &[
        "pack lint|test|diff|search|inspect|install|outdated|update ...",
            "pack lock|sync|list|path|remove|rollback|import|verify|sign ...",
    ])
    .details("Run `wayexpand pack <action>` without further arguments to see that action's usage.")
    .advanced(),
    command("library", Snippets, "Browse and pin offline Git snippet catalogs", &[
        "library add <git-url> [--name NAME] [--branch BRANCH] [--approved-revision FULL_COMMIT] [--json]",
        "library list|path|search|remove ...",
    ]).advanced(),
    command("schema", Snippets, "Print the configuration schema for editors and tools", &["schema"]).advanced(),
    command("context", Snippets, "Shell context for snippets that adapt to host and project", &[
        "context integration bash|zsh|fish",
        "context report|status|clear ...",
        "context render <trigger> [--remote-host HOST] [--environment NAME] [--kubernetes-context NAME] [--set NAME=VALUE]... [--json]",
    ])
    .details("Prints an opt-in shell prompt hook that reports the current host, project and Kubernetes context, so snippets can render for it. render shows the text for a trigger in the current context; it never executes anything.")
    .advanced(),
    command("workflow", Snippets, "Explicit insertion, file generation or an approved broker action", &[
        "workflow insert <trigger>",
        "workflow generate <template> [--set NAME=VALUE]... [-o PATH] [--force] [--config FILE]",
        "workflow run <ACTION_ID> [--set NAME=VALUE]... [--approve-action ACTION_ID] [--config FILE]",
    ]).advanced(),
    // Background service
    command("status", Service, "Show whether the service is running and expanding", &["status [--json]"]),
    command("pause", Service, "Pause expansion", &["pause [--json]"]),
    command("resume", Service, "Resume expansion", &["resume [--json]"]),
    command("reload", Service, "Reload the configuration now", &["reload [--json]"]),
    command("stop", Service, "Stop the running service", &["stop [--json]"]),
    command("service", Service, "Inspect the installed services, or restart the active one", &["service status [--json]", "service restart --yes"]),
    command("upgrade", Service, "Finish, check or roll back a staged upgrade", &["upgrade --check|--apply|--rollback [--json]"]),
    command("upgrade-status", Service, "Check whether the running service matches this CLI", &["upgrade-status [--json]"]).advanced(),
    // Troubleshooting
    command("why", Troubleshoot, "Find the likely cause and next step for a missed trigger", &["why [--json]"])
        .details("Reports one likely cause and one suggested next action from the running service and IBus engine's content-free status. It explains whether matching is on, whether typing arrives, and what happened to recent expansions. No typed text is recorded or shown.")
        .examples(&[("wayexpand why", "Type a trigger that did not expand, then run this")]),
    command("explain", Troubleshoot, "Explain why text would or would not expand", &["explain <text> [--json] [--offline] [--app ID] [config]"])
        .details("Runs the text through the same matcher the service uses and reports each check: trigger, word boundary, app filter, policy. --app tests it as if that application had focus; --offline skips the running service.")
        .examples(&[("wayexpand explain 'Thanks ;sig'", "Why ;sig does or does not expand here")]),
    command("debug", Troubleshoot, "Explain live readiness, or guide a harmless insertion test", &[
        "debug [--json] [--trigger TEXT] [--live [--seconds 1..300]]",
        "debug --focused-app-id [--json]",
        "debug --test [TRIGGER] [--application-class CLASS --focus-confirmed [--observe-only]]",
        "debug --gnome --interactive",
    ])
    .details("--live watches the service for up to 300 seconds and reports counts only, never typed text. --focused-app-id prints the app ID snippet filters see for the focused window. --test without a trigger uses only a known harmless tutorial snippet; otherwise preview a snippet and pass its trigger explicitly.")
    .examples(&[
        ("wayexpand debug --focused-app-id", "Which app ID does this window have?"),
        ("wayexpand debug --live --seconds 30", "Watch the service while you type"),
    ]),
    command("health", Troubleshoot, "Report readiness with a stable monitoring exit code", &["health [--json]"]).advanced(),
    command("install-check", Troubleshoot, "Check that WayExpand's components are installed", &["install-check [--json] [--config PATH]"])
        .details("Read-only audit of the installed files, desktop integration, services, and active configuration. It identifies missing components without changing the installation; use `repair --check` to review safe repairs.")
        .examples(&[("wayexpand install-check", "Check whether the installation is complete")]).advanced(),
    command("repair", Troubleshoot, "Show or apply installation repairs", &["repair --check|--apply [--json]", "repair --gnome-input-source [--json]"]),
    command("reset", Troubleshoot, "Preview and apply a scoped recovery reset", &["reset --integration|--services|--preferences [--check|--apply --yes] [--json]"]).advanced(),
    command("support-bundle", Troubleshoot, "Write a redacted report to attach to a bug report", &["support-bundle [--gnome] [--output FILE]"])
        .details("Collects an installation audit, versions, backend state and recent findings. Installation component states omit local file paths; snippet text is excluded. Its first step prioritizes missing components or installation identity mismatches before route advice. GNOME integration checks are included automatically in a GNOME session; --gnome requests them on other desktops.")
        .aliases(&["support"]),
    command("compatibility-report", Troubleshoot, "Generate a reproducible desktop compatibility report", &["compatibility-report [--json]"]).advanced(),
    command("help", Troubleshoot, "Show help for WayExpand or one command", &[
        "help [<command> | --all]",
        "help finding <WX-ID>",
    ])
    .details("help finding explains a diagnostic finding ID such as WX-IBUS-002 and its next step.")
    .advanced(),
    // Desktop integration
    command("ibus", Desktop, "Check the IBus route, watch fields, or restart IBus", &[
        "ibus [status [--json]]",
        "ibus watch [--seconds N]",
        "ibus restart",
    ])
    .details("status checks the IBus route link by link and prints the fix for the first broken link. watch explains, per focused field, why triggers do or do not expand; it reports counts and states only. restart restarts IBus the way your desktop expects (on KDE Plasma, through KWin)."),
    command("backend", Desktop, "Inspect input routes, or test capture and output once", &[
        "backend list [--json]",
        "backend select --explain",
        "backend profile [ID|--current] [--json]",
        "backend test --capture|--injection",
    ]),
    command("portal", Desktop, "Inspect or remove the remote-desktop portal token", &["portal status|reset"]).advanced(),
    command("certify", Desktop, "Run local compatibility certification", &["certify [--json]"]).advanced(),
    command("explain-backend", Desktop, "Explain the automatic backend choice", &["explain-backend"]).advanced(),
    // Teams and automation
    command("policy", Organization, "Inspect organization policy, or fail CI when it is not enforced", &["policy inspect|check [--file PATH] [--allow-audit-only] [--json]"]).advanced(),
    command("fleet", Organization, "Show the merged fleet configuration", &["fleet status [--json]"]).advanced(),
    command("metrics", Organization, "Write content-free Prometheus metrics", &["metrics [--output FILE] [config]"]).advanced(),
    command("completions", Organization, "Print shell completions", &["completions bash|zsh|fish"])
        .details("Prints a completion script for your shell. Load it from your shell's startup file.")
        .examples(&[
            (
                "source <(wayexpand completions bash)",
                "bash: add this line to ~/.bashrc",
            ),
            ("wayexpand completions zsh > ~/.zfunc/_wayexpand", "zsh (with ~/.zfunc in fpath)"),
            ("wayexpand completions fish > ~/.config/fish/completions/wayexpand.fish", "fish"),
        ])
        .advanced(),
    command("version", Organization, "Print the installed version", &["version"]).advanced(),
];

/// Look up a command by name or alias.
pub(crate) fn find(name: &str) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|command| command.name == name || command.aliases.contains(&name))
}

/// The closest command name to a mistyped one, if any is close enough.
pub(crate) fn suggestion(typed: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .flat_map(|command| std::iter::once(command.name).chain(command.aliases.iter().copied()))
        .map(|name| (edit_distance(typed, name), name))
        .filter(|(distance, name)| *distance <= (name.len() / 3).max(1))
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, name)| find(name).map_or(name, |command| command.name))
}

/// Edit distance where swapping two neighbouring letters ("lsit") counts as
/// one edit, the most common typing slip.
fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut table = vec![vec![0usize; right.len() + 1]; left.len() + 1];
    for (row, cells) in table.iter_mut().enumerate() {
        cells[0] = row;
    }
    for (column, cell) in table[0].iter_mut().enumerate() {
        *cell = column;
    }
    for row in 1..=left.len() {
        for column in 1..=right.len() {
            let cost = usize::from(left[row - 1] != right[column - 1]);
            let mut best = (table[row - 1][column] + 1)
                .min(table[row][column - 1] + 1)
                .min(table[row - 1][column - 1] + cost);
            if row > 1
                && column > 1
                && left[row - 1] == right[column - 2]
                && left[row - 2] == right[column - 1]
            {
                best = best.min(table[row - 2][column - 2] + 1);
            }
            table[row][column] = best;
        }
    }
    table[left.len()][right.len()]
}

pub(crate) struct Style {
    color: bool,
}

impl Style {
    pub(crate) fn stdout() -> Self {
        Self {
            color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }
    #[cfg(test)]
    pub(crate) fn plain() -> Self {
        Self { color: false }
    }
    pub(crate) fn heading(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[1m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
}

/// Wrap `text` at `width` columns, indenting every line by `indent`.
pub(crate) fn wrap(text: &str, indent: usize, width: usize) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && indent + line.chars().count() + 1 + word.chars().count() > width {
            out.push_str(&" ".repeat(indent));
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push_str(&" ".repeat(indent));
        out.push_str(&line);
        out.push('\n');
    }
    out
}

pub(crate) const WIDTH: usize = 80;
const SERVICE_CONTROLS: [&str; 5] = ["status", "pause", "resume", "reload", "stop"];
const NAME_COLUMN: usize = 18;

/// One `name  summary` row, the summary wrapped under its own column. A name
/// too long for the column gets a line of its own rather than pushing the
/// summary out of line with the rows around it.
pub(crate) fn summary_row(name: &str, summary: &str) -> String {
    summary_row_at(name, summary, NAME_COLUMN)
}

/// [`summary_row`] with the name column `column` characters wide; at least
/// two spaces always separate a name from its summary.
pub(crate) fn summary_row_at(name: &str, summary: &str, column: usize) -> String {
    if summary.is_empty() {
        return format!("  {name}\n");
    }
    let wrapped = wrap(summary, column + 2, WIDTH);
    if name.chars().count() + 2 > column {
        return format!("  {name}\n{wrapped}");
    }
    let name = format!("  {name:<column$}");
    // The first wrapped line replaces the indentation with the name.
    format!("{name}{}", &wrapped[column + 2..])
}

fn main_help(style: &Style, version: &str, all: bool) -> String {
    // Development builds carry a long build hash; `wayexpand version` has it.
    let version = version.split('+').next().unwrap_or(version);
    let mut help = format!(
        "WayExpand {version} — text expansion for Wayland\n\n{}  wayexpand <command> [options]\n",
        style.heading("Usage:")
    );
    for group in Group::ORDER {
        let commands: Vec<&Command> = COMMANDS
            .iter()
            .filter(|command| command.group == group && (all || !command.advanced))
            .collect();
        if commands.is_empty() {
            continue;
        }
        help.push_str(&format!(
            "\n{}\n",
            style.heading(&format!("{}:", group.title()))
        ));
        // The five service controls read best as one row in the short help.
        let combine_controls = group == Group::Service && !all;
        if combine_controls {
            help.push_str(&summary_row(
                "status",
                "Show whether the service is running and expanding",
            ));
            help.push_str(&summary_row(
                "pause, resume",
                "Pause or resume expansion; reload or stop the service",
            ));
            help.push_str(&summary_row("reload, stop", ""));
        }
        for command in commands
            .into_iter()
            .filter(|command| !combine_controls || !SERVICE_CONTROLS.contains(&command.name))
        {
            help.push_str(&summary_row(command.name, command.summary));
            if all {
                for alias in command.aliases {
                    help.push_str(&summary_row(alias, &format!("Same as `{}`", command.name)));
                }
            }
        }
    }
    if !all {
        let more: Vec<&str> = COMMANDS
            .iter()
            .filter(|command| command.advanced)
            .map(|command| command.name)
            .collect();
        help.push_str(&format!("\n{}\n", style.heading("More commands:")));
        help.push_str(&wrap(&more.join(", "), 2, WIDTH));
    }
    help.push_str(&format!("\n{}\n", style.heading("Examples:")));
    for (example, meaning) in [
        ("wayexpand setup --check", "Is everything ready?"),
        ("wayexpand why", "Why didn't my last trigger expand?"),
    ] {
        help.push_str(&format!("  {example:<34}{meaning}\n"));
    }
    help.push_str(&format!(
        "\nRun `wayexpand help 'COMMAND'` for a command's options{}.\n",
        if all { "" } else { "; `help --all` lists all" }
    ));
    help
}

fn command_help(style: &Style, command: &Command, default_config: &str) -> String {
    let mut help = wrap(
        &format!("wayexpand {} — {}", command.name, command.summary),
        0,
        WIDTH,
    );
    help.push_str(&format!("\n{}\n", style.heading("Usage:")));
    for usage in command.usage {
        help.push_str(&wrap_usage(usage));
    }
    if !command.details.is_empty() {
        help.push('\n');
        help.push_str(&wrap(command.details, 2, WIDTH));
    }
    if command.usage.iter().any(|usage| usage.contains("[config]")) {
        help.push('\n');
        help.push_str(&wrap(
            &format!("[config] is the configuration file; it defaults to {default_config}."),
            2,
            WIDTH,
        ));
    }
    if !command.aliases.is_empty() {
        help.push_str(&format!(
            "\n  Also available as: {}\n",
            command.aliases.join(", ")
        ));
    }
    if !command.examples.is_empty() {
        help.push_str(&format!("\n{}\n", style.heading("Examples:")));
        for (example, meaning) in command.examples {
            help.push_str(&format!("  {example}\n"));
            help.push_str(&wrap(meaning, 6, WIDTH));
        }
    }
    help
}

/// Wrap a usage line at option boundaries, continuing under the command.
fn wrap_usage(usage: &str) -> String {
    let prefix = "  wayexpand ";
    let mut out = String::new();
    let mut line = String::from(prefix);
    let continuation =
        " ".repeat(prefix.len() + usage.split(' ').next().map_or(0, |word| word.len() + 1));
    for token in usage_tokens(usage) {
        if line.trim().len() > prefix.trim().len()
            && line.chars().count() + 1 + token.chars().count() > WIDTH
        {
            out.push_str(line.trim_end());
            out.push('\n');
            line = continuation.clone();
        } else if !line.ends_with(' ') {
            line.push(' ');
        }
        line.push_str(&token);
    }
    out.push_str(line.trim_end());
    out.push('\n');
    out
}

/// Split a usage line into words, keeping each bracketed group whole.
fn usage_tokens(usage: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for character in usage.chars() {
        match character {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if character == ' ' && depth == 0 {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub(crate) fn print_main_help(version: &str, all: bool) {
    print!("{}", main_help(&Style::stdout(), version, all));
}

pub(crate) fn print_command_help(command: &Command, default_config: &str) {
    print!(
        "{}",
        command_help(&Style::stdout(), command, default_config)
    );
}

#[cfg(test)]
pub(crate) fn plain_main_help(version: &str, all: bool) -> String {
    main_help(&Style::plain(), version, all)
}

#[cfg(test)]
pub(crate) fn plain_command_help(command: &Command, default_config: &str) -> String {
    command_help(&Style::plain(), command, default_config)
}

/// Shell completion scripts generated from the registry.
pub(crate) fn completions(shell: &str) -> Option<String> {
    let names: Vec<&str> = COMMANDS
        .iter()
        .flat_map(|command| std::iter::once(command.name).chain(command.aliases.iter().copied()))
        .collect();
    let words = names.join(" ");
    match shell {
        "bash" => Some(format!(
            "# bash completion for wayexpand\n_wayexpand() {{\n    if [ \"$COMP_CWORD\" -eq 1 ]; then\n        COMPREPLY=($(compgen -W \"{words}\" -- \"${{COMP_WORDS[1]}}\"))\n    else\n        COMPREPLY=($(compgen -f -- \"${{COMP_WORDS[COMP_CWORD]}}\"))\n    fi\n}}\ncomplete -F _wayexpand wayexpand\n"
        )),
        "zsh" => {
            let described: Vec<String> = COMMANDS
                .iter()
                .map(|command| format!("    '{}:{}'", command.name, command.summary.replace('\'', "'\\''").replace(':', "\\:")))
                .collect();
            Some(format!(
                "#compdef wayexpand\n_wayexpand() {{\n  local -a commands\n  commands=(\n{}\n  )\n  if (( CURRENT == 2 )); then\n    _describe 'command' commands\n  else\n    _files\n  fi\n}}\n_wayexpand \"$@\"\n",
                described.join("\n")
            ))
        }
        "fish" => {
            let mut script = String::from("# fish completion for wayexpand\ncomplete -c wayexpand -f\n");
            for command in COMMANDS {
                script.push_str(&format!(
                    "complete -c wayexpand -n '__fish_use_subcommand' -a {} -d '{}'\n",
                    command.name,
                    command.summary.replace('\'', "\\'")
                ));
            }
            Some(script)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_help_fits_an_80_column_terminal() {
        for all in [false, true] {
            let help = plain_main_help("1.3.5", all);
            for line in help.lines() {
                assert!(line.chars().count() <= WIDTH, "too wide: {line:?}");
            }
        }
        // One terminal screen.
        assert!(plain_main_help("1.3.5", false).lines().count() <= 48);
    }

    #[test]
    fn every_command_help_fits_an_80_column_terminal() {
        for command in COMMANDS {
            let help = plain_command_help(command, "~/.config/wayexpand/expansions.toml");
            for line in help.lines() {
                assert!(
                    line.chars().count() <= WIDTH || !line.contains(' '),
                    "{}: too wide: {line:?}",
                    command.name
                );
            }
        }
    }

    #[test]
    fn names_and_aliases_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for command in COMMANDS {
            for name in std::iter::once(command.name).chain(command.aliases.iter().copied()) {
                assert!(seen.insert(name), "duplicate command name {name}");
            }
        }
    }

    #[test]
    fn typos_suggest_the_intended_command() {
        assert_eq!(suggestion("lsit"), Some("list"));
        assert_eq!(suggestion("dcotor"), Some("doctor"));
        assert_eq!(suggestion("reolad"), Some("reload"));
        assert_eq!(suggestion("supprt"), Some("support-bundle"));
        assert_eq!(suggestion("xyzzy"), None);
    }

    #[test]
    fn completions_cover_every_command() {
        for shell in ["bash", "zsh", "fish"] {
            let script = completions(shell).expect(shell);
            for command in COMMANDS {
                assert!(
                    script.contains(command.name),
                    "{shell} lacks {}",
                    command.name
                );
            }
        }
        assert!(completions("tcsh").is_none());
    }

    #[test]
    fn long_usage_lines_wrap_at_option_boundaries() {
        let setup = find("setup").unwrap();
        let help = plain_command_help(setup, "x");
        assert!(
            help.contains("[--mode recommended|maximum|experimental]"),
            "{help}"
        );
        assert!(
            help.lines().all(|line| line.chars().count() <= WIDTH),
            "{help}"
        );
    }
}
