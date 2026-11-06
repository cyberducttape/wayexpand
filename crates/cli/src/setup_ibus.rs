//! `wayexpand setup --backend ibus [--terminals]`: configure the IBus route
//! end to end and verify every link, so users never wire IBus by hand.
//!
//! Each step is idempotent and checks the live result rather than trusting a
//! command's exit status (`ibus engine` exits 1 after a successful switch to
//! an engine whose layout is `default`).

use crate::services::{ServiceInventory, UnitState, UserService};
use crate::setup::{enable_user_service, evdev_consent_marker, run_setup_command};
use crate::*;
use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use wayexpand_backend_ibus::chain::{
    self, kwin_input_method, kwin_input_method_available, IbusCheckState, IbusDaemonProcess,
    IbusDesktop, KWIN_IBUS_INPUT_METHOD,
};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const COMPONENT_TEMPLATE: &str = include_str!("../../../desktop/wayexpand-ibus.xml");
const COMMAND_OUTPUT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_COMMAND_OUTPUT: u64 = 64 * 1024;
const TERMINAL_DROP_IN: &str = "30-terminal-apps-only.conf";
/// Common terminal app IDs (normalized), used when `terminal_apps` is empty.
pub(crate) const DEFAULT_TERMINAL_APPS: &[&str] = &[
    "org.kde.konsole",
    "org.kde.yakuake",
    "org.gnome.console",
    "org.gnome.terminal",
    "org.gnome.ptyxis",
    "com.mitchellh.ghostty",
    "kitty",
    "alacritty",
    "foot",
    "footclient",
    "org.wezfurlong.wezterm",
    "xterm",
];

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct IbusSetupOptions {
    pub(crate) assume_yes: bool,
    /// Explicit user acceptance of IBus's verified but non-atomic replacement.
    /// This is intentionally separate from `assume_yes`.
    pub(crate) accept_non_atomic_replacement: bool,
    /// Also cover terminals with a terminal-only evdev daemon.
    pub(crate) terminals: bool,
}

/// Run `program` with a deadline and return its stdout when it succeeds.
pub(crate) fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_process_group(&mut command);
    let child = command.spawn().ok()?;
    let mut supervisor = ChildSupervisor::new(child);
    let mut stdout = supervisor.take_stdout()?;
    let reader = thread::spawn(move || {
        let mut output = String::new();
        let _ = (&mut stdout)
            .take(MAX_COMMAND_OUTPUT)
            .read_to_string(&mut output);
        output
    });
    let deadline = Instant::now() + COMMAND_OUTPUT_TIMEOUT;
    loop {
        match supervisor.has_exited() {
            Ok(true) => break,
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return None;
            }
        }
    }
    supervisor.kill_group();
    let status = supervisor.reap().ok()?;
    let output = reader.join().ok()?;
    status.success().then_some(output)
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn step(message: &str) {
    println!("• {message}");
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The component manifest with an absolute engine path, so IBus finds the
/// engine however the desktop starts it (its PATH may lack ~/.local/bin).
pub(crate) fn component_manifest(engine: &Path) -> String {
    COMPONENT_TEMPLATE.replace(
        "<exec>wayexpand-ibus</exec>",
        &format!("<exec>{}</exec>", xml_escape(&engine.display().to_string())),
    )
}

fn engine_binary() -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join("wayexpand-ibus"))
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleEngineProcess {
    pub(crate) pid: i32,
    pub(crate) executable: PathBuf,
}

/// Find this user's WayExpand IBus engine processes that still map an older
/// engine executable. IBus owns these processes, so restarting a WayExpand
/// systemd unit cannot refresh them.
pub(crate) fn stale_engine_processes() -> Vec<StaleEngineProcess> {
    let Some(installed) = engine_binary().and_then(|path| fs::canonicalize(path).ok()) else {
        return Vec::new();
    };
    stale_engine_processes_at(Path::new("/proc"), unsafe { libc::geteuid() }, &installed)
}

fn stale_engine_processes_at(
    proc_root: &Path,
    uid: u32,
    installed: &Path,
) -> Vec<StaleEngineProcess> {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut processes: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<i32>().ok()?;
            let proc_dir = entry.path();
            let owned = fs::metadata(&proc_dir)
                .is_ok_and(|metadata| std::os::unix::fs::MetadataExt::uid(&metadata) == uid);
            if !owned {
                return None;
            }
            let executable = fs::read_link(proc_dir.join("exe")).ok()?;
            // `comm` is useful, but can differ from argv[0] when an IBus
            // launcher or wrapper starts the engine. Check all three kernel
            // views so the upgrade report still identifies the process that
            // IBus must restart.
            let comm_matches = fs::read_to_string(proc_dir.join("comm"))
                .is_ok_and(|comm| comm.trim() == "wayexpand-ibus");
            let exe_matches = executable
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "wayexpand-ibus" || name == "wayexpand-ibus (deleted)");
            let argv_matches = fs::read(proc_dir.join("cmdline"))
                .ok()
                .and_then(|cmdline| {
                    cmdline
                        .split(|byte| *byte == 0)
                        .find(|argument| !argument.is_empty())
                        .map(|argument| {
                            argument
                                .rsplit(|byte| *byte == b'/')
                                .next()
                                .is_some_and(|name| name == b"wayexpand-ibus")
                        })
                })
                .unwrap_or(false);
            if !(comm_matches || exe_matches || argv_matches) {
                return None;
            }
            (executable != installed).then_some(StaleEngineProcess { pid, executable })
        })
        .collect();
    processes.sort_by_key(|process| process.pid);
    processes
}

/// Install the component where the running ibus-daemon reads components.
/// Writing there needs administrator rights, so this asks through pkexec.
fn install_component() -> Result<()> {
    let report = chain::diagnose();
    let component_ok = report
        .check("component")
        .is_some_and(|check| check.state == IbusCheckState::Pass);
    let manifest_ok = report
        .check("manifest")
        .is_some_and(|check| check.state == IbusCheckState::Pass);
    if component_ok && manifest_ok {
        step("The IBus component is installed where IBus reads it");
        return Ok(());
    }
    let engine = engine_binary().ok_or_else(|| {
        anyhow::anyhow!("`wayexpand-ibus` is not on PATH; reinstall WayExpand (user installs need ~/.local/bin on PATH)")
    })?;
    let directory = chain::component_install_directory();
    let target = directory.join("wayexpand.xml");
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| anyhow::anyhow!("XDG_RUNTIME_DIR is not set"))?;
    let staged = runtime.join(format!(
        "wayexpand-ibus-component-{}.xml",
        std::process::id()
    ));
    fs::write(&staged, component_manifest(&engine))?;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o644))?;
    step(&format!(
        "Installing the IBus component to {} (IBus never reads per-user component folders; you will be asked for an administrator password)",
        target.display()
    ));
    let staged_arg = staged.display().to_string();
    let target_arg = target.display().to_string();
    let installed = run_setup_command(
        "pkexec",
        &[
            "/usr/bin/install",
            "-D",
            "-m",
            "0644",
            &staged_arg,
            &target_arg,
        ],
    )
    .is_ok_and(|status| status.success());
    if !installed {
        let manual = format!("sudo install -D -m 0644 {staged_arg} {target_arg}");
        println!("  Could not install it automatically. Run this, then rerun setup:");
        println!("    {manual}");
        bail!("the IBus component is not installed where IBus reads it");
    }
    let _ = fs::remove_file(&staged);
    Ok(())
}

/// On KDE Plasma Wayland, make KWin launch IBus as its input method; only
/// then do Wayland apps send keys to IBus.
fn route_kde_to_ibus(snapshot: &mut SessionSnapshot) -> Result<()> {
    let configured =
        kwin_input_method().is_some_and(|value| value.to_ascii_lowercase().contains("ibus"));
    if configured && kwin_input_method_available() == Some(true) {
        step("KWin already sends input to IBus");
        return Ok(());
    }
    if !Path::new(KWIN_IBUS_INPUT_METHOD).exists() {
        bail!(
            "{KWIN_IBUS_INPUT_METHOD} is missing; install the IBus GTK 3 panel (for example the `ibus-gtk3` package) and rerun setup"
        );
    }
    // A hand-started ibus-daemon has no Wayland input-method connection and
    // would keep KWin's own instance from starting.
    if let Some(daemon) = IbusDaemonProcess::find() {
        if !chain::started_by(daemon.pid, "kwin_wayland") {
            step("Stopping an ibus-daemon that was not started by KWin");
            let _ = run_setup_command("ibus", &["exit"]);
            wait_until(Duration::from_secs(5), || {
                IbusDaemonProcess::find().is_none()
            });
        }
    }
    step("Setting KWin's Virtual Keyboard to IBus Wayland");
    // KWin applies the setting from a change notification, and
    // kwriteconfig6 sends none when the stored value is unchanged.
    let kwriteconfig = kwriteconfig().ok_or_else(|| anyhow::anyhow!("kwriteconfig6 is not installed; set System Settings → Keyboard → Virtual Keyboard → \"IBus Wayland\" yourself, then rerun setup"))?;
    snapshot.kwin_input_method = Some(kwin_input_method());
    let base = KWIN_INPUT_METHOD_KEY;
    let mut delete = base.to_vec();
    delete.push("--delete");
    let _ = run_setup_command(kwriteconfig, &delete);
    let mut write = base.to_vec();
    write.push(KWIN_IBUS_INPUT_METHOD);
    if !run_setup_command(kwriteconfig, &write).is_ok_and(|status| status.success()) {
        bail!("could not update kwinrc; set System Settings → Keyboard → Virtual Keyboard → \"IBus Wayland\" yourself");
    }
    if !wait_until(Duration::from_secs(15), || {
        kwin_input_method_available() == Some(true) && IbusDaemonProcess::find().is_some()
    }) {
        bail!("KWin did not start IBus; reselect \"IBus Wayland\" in System Settings → Keyboard → Virtual Keyboard, or log out and back in, then rerun setup");
    }
    Ok(())
}

/// After an upgrade IBus keeps running the old engine binary. Stop engine
/// processes that do not run the installed binary, so selecting the engine
/// starts the current one.
fn restart_stale_engine() {
    for process in stale_engine_processes() {
        step("Stopping an IBus engine from an older installation");
        unsafe {
            libc::kill(process.pid, libc::SIGTERM);
        }
    }
}

/// Parse `gsettings get` output for a string list such as `['a', 'b']` or
/// `@as []`.
pub(crate) fn parse_string_list(output: &str) -> Option<Vec<String>> {
    let trimmed = output.trim().trim_start_matches("@as").trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    inner
        .split(',')
        .map(|item| {
            let item = item.trim();
            item.strip_prefix('\'')
                .and_then(|item| item.strip_suffix('\''))
                .map(str::to_owned)
        })
        .collect()
}

pub(crate) fn format_string_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("'{item}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Add WayExpand to IBus' input methods so it appears in the IBus menu and
/// survives restarts.
fn add_to_preload_engines() {
    let Some(output) = command_output(
        "gsettings",
        &["get", "org.freedesktop.ibus.general", "preload-engines"],
    ) else {
        println!("  (could not read IBus' input-method list; add WayExpand in the IBus preferences if it is missing)");
        return;
    };
    let Some(mut engines) = parse_string_list(&output) else {
        return;
    };
    if engines.iter().any(|engine| engine == "wayexpand") {
        return;
    }
    if engines.is_empty() {
        // An empty list means "the default keyboard"; keep one so switching
        // away from WayExpand still leaves a plain keyboard layout.
        engines.push("xkb:us::eng".into());
    }
    engines.push("wayexpand".into());
    step("Adding WayExpand to IBus' input methods");
    let _ = run_setup_command(
        "gsettings",
        &[
            "set",
            "org.freedesktop.ibus.general",
            "preload-engines",
            &format_string_list(&engines),
        ],
    );
}

fn service_active(service: &str) -> bool {
    run_setup_command("systemctl", &["--user", "is-active", "--quiet", service])
        .is_ok_and(|status| status.success())
}

fn companion_readiness_error(service: UserService, status: &serde_json::Value) -> Option<String> {
    let expected_source = match service {
        UserService::Evdev => "evdev",
        UserService::Control => "stdin",
        UserService::InputMethod => return Some("IBus is not a companion service".into()),
    };
    let field = |name: &str| status.get(name).and_then(serde_json::Value::as_str);
    if !crate::doctor::status::status_schema_compatible(status) {
        return Some("daemon status schema is incompatible".into());
    }
    if !crate::doctor::status::daemon_commit_matches(field("daemon_commit")) {
        return Some("daemon build identity does not match the setup CLI".into());
    }
    if field("source") != Some(expected_source) {
        return Some(format!(
            "expected source {expected_source}, found {:?}",
            field("source")
        ));
    }
    if field("backend") != Some("libei") {
        return Some(format!(
            "expected libei output, found {:?}",
            field("backend")
        ));
    }
    if field("config_state") != Some("ok")
        || status
            .get("config_generation")
            .and_then(serde_json::Value::as_u64)
            .is_none_or(|generation| generation == 0)
        || field("config_active_path") != Some(default_config_path().to_string_lossy().as_ref())
    {
        return Some("expected configuration is not loaded and healthy".into());
    }
    if field("output_connection_state") != Some("connected") || field("output_health") != Some("ok")
    {
        return Some("libei output is not operational".into());
    }
    match service {
        UserService::Evdev
            if field("state") != Some("connected")
                || status.get("route_available") != Some(&serde_json::Value::Bool(true))
                || status.get("matching_available") != Some(&serde_json::Value::Bool(true))
                || field("input_connection_state") != Some("connected") =>
        {
            Some("evdev capture or matching is not operational".into())
        }
        UserService::Control if field("state") != Some("running") => {
            Some("control service is not running".into())
        }
        _ => None,
    }
}

fn companion_status_error(service: UserService) -> Result<()> {
    let response = crate::doctor::status::read_daemon_status()
        .map_err(|error| anyhow::anyhow!("daemon control status is unavailable: {error}"))?;
    let status = crate::doctor::status::status_as_json(&response)?;
    if let Some(problem) = companion_readiness_error(service, &status) {
        bail!("{problem}");
    }
    Ok(())
}

fn terminal_drop_in() -> Option<PathBuf> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(
        config_home
            .join("systemd/user/wayexpand-evdev.service.d")
            .join(TERMINAL_DROP_IN),
    )
}

/// Make sure `settings.terminal_apps` lists terminals; returns the list.
fn ensure_terminal_apps() -> Result<Vec<String>> {
    let path = default_config_path();
    let loaded = Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    if !loaded.config.settings.terminal_apps.is_empty() {
        return Ok(loaded.config.settings.terminal_apps);
    }
    let mut config = loaded.config;
    config.settings.terminal_apps = DEFAULT_TERMINAL_APPS
        .iter()
        .map(|app| (*app).to_owned())
        .collect();
    config.validate().map_err(|error| {
        config_error(format!(
            "configuration invalid after edit: {}",
            error.safe_summary()
        ))
    })?;
    config
        .save_atomic_if_revision_matches(&path, &loaded.revision)
        .map_err(|error| {
            config_error(format!(
                "could not save configuration: {}",
                error.safe_summary()
            ))
        })?;
    step(&format!(
        "Listed common terminals in settings.terminal_apps ({}); edit that list in {} to add yours",
        DEFAULT_TERMINAL_APPS.len(),
        path.display()
    ));
    Ok(config.settings.terminal_apps)
}

/// IBus replacement is verified but not atomic, which the default
/// `compatibility_mode = "off"` refuses. Ask for that consent explicitly;
/// `--yes` never grants it on the user's behalf.
fn replacement_authorization_required(mode: wayexpand_core::CompatibilityMode) -> bool {
    mode == wayexpand_core::CompatibilityMode::Off
}

fn ensure_replacement_consent(options: IbusSetupOptions) -> Result<bool> {
    let path = default_config_path();
    let loaded = Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    if !replacement_authorization_required(loaded.config.settings.compatibility_mode) {
        return Ok(false);
    }
    println!("  IBus checks that the text before the cursor is your trigger, then deletes it and inserts");
    println!("  the snippet as two separate steps. compatibility_mode = \"off\" refuses that, so nothing would expand.");
    if !options.accept_non_atomic_replacement && options.assume_yes {
        println!("  Not changed by --yes. Enable it with compatibility_mode = \"tracked-focus\" under [settings] in {}", path.display());
        return Ok(true);
    }
    if !options.accept_non_atomic_replacement && !prompt_yes_no(
        "  Allow verified two-step replacement (compatibility_mode = \"tracked-focus\")? [Y/n] ",
        true,
    )? {
        println!(
            "  Left compatibility_mode off; IBus will recognize triggers but not replace them."
        );
        return Ok(true);
    }
    let mut config = loaded.config;
    config.settings.compatibility_mode = wayexpand_core::CompatibilityMode::TrackedFocus;
    config.validate().map_err(|error| {
        config_error(format!(
            "configuration invalid after edit: {}",
            error.safe_summary()
        ))
    })?;
    config
        .save_atomic_if_revision_matches(&path, &loaded.revision)
        .map_err(|error| {
            config_error(format!(
                "could not save configuration: {}",
                error.safe_summary()
            ))
        })?;
    step("Set compatibility_mode = \"tracked-focus\"");
    Ok(false)
}

/// What setup may change in the running session, recorded before it changes
/// it, so a failed activation can put the session back.
#[derive(Debug, Default)]
struct SessionSnapshot {
    /// `ibus engine` before setup, if IBus was running.
    global_engine: Option<String>,
    /// Raw `gsettings get` value of IBus' input-method list.
    preload_engines: Option<String>,
    /// KWin's previous `[Wayland] InputMethod`, recorded when setup changed it.
    kwin_input_method: Option<Option<String>>,
    /// GNOME's typed input-source list and active source, when applicable.
    gnome_input_sources: Option<crate::gnome_input_sources::Snapshot>,
    /// Original service state for every route that setup may affect.
    services: Vec<OriginalServiceState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OriginalServiceState {
    service: UserService,
    installed: bool,
    enabled: bool,
    active: bool,
}

impl OriginalServiceState {
    fn capture(service: UserService, state: UnitState) -> Self {
        Self {
            service,
            installed: state.installed,
            enabled: state.enabled,
            active: state.active,
        }
    }
}

impl SessionSnapshot {
    fn capture(inventory: &ServiceInventory) -> Self {
        Self {
            global_engine: command_output("ibus", &["engine"])
                .map(|engine| engine.trim().to_owned())
                .filter(|engine| !engine.is_empty()),
            preload_engines: command_output(
                "gsettings",
                &["get", "org.freedesktop.ibus.general", "preload-engines"],
            )
            .map(|value| value.trim().to_owned()),
            gnome_input_sources: (IbusDesktop::detect() == IbusDesktop::Gnome)
                .then(crate::gnome_input_sources::Snapshot::capture)
                .and_then(Result::ok),
            services: UserService::ALL
                .into_iter()
                .map(|service| OriginalServiceState::capture(service, inventory.state(service)))
                .collect(),
            ..Self::default()
        }
    }

    /// Undo session changes in reverse order. Prepared, inert changes (the
    /// component file, configuration consent) are kept.
    fn rollback(&self) {
        let mut failures = self.rollback_with(kwriteconfig(), |program, arguments| {
            run_setup_command(program, arguments).is_ok_and(|status| status.success())
        });
        let restored = ServiceInventory::probe();
        for original in &self.services {
            if !original.installed {
                continue;
            }
            let actual = restored.state(original.service);
            if actual.installed != original.installed
                || actual.enabled != original.enabled
                || actual.active != original.active
            {
                failures.push(format!(
                    "{} expected installed={}, enabled={}, active={} but found installed={}, enabled={}, active={}",
                    original.service.unit(),
                    original.installed,
                    original.enabled,
                    original.active,
                    actual.installed,
                    actual.enabled,
                    actual.active
                ));
            }
        }
        if !failures.is_empty() {
            eprintln!(
                "Setup rollback could not restore every service state: {}",
                failures.join(", ")
            );
        }
    }

    fn rollback_with(
        &self,
        kwriteconfig_program: Option<&str>,
        mut run: impl FnMut(&str, &[&str]) -> bool,
    ) -> Vec<String> {
        println!("Setup did not complete; restoring your previous input-method setup…");
        let mut failures = Vec::new();
        for original in &self.services {
            if !original.installed {
                continue;
            }
            let unit = original.service.unit();
            let enable_action = if original.enabled {
                "enable"
            } else {
                "disable"
            };
            if !run("systemctl", &["--user", enable_action, unit]) {
                failures.push(format!("{unit} enabled={}", original.enabled));
            }
            let active_action = if original.active { "start" } else { "stop" };
            if !run("systemctl", &["--user", active_action, unit]) {
                failures.push(format!("{unit} active={}", original.active));
            }
        }
        if let (Some(previous), Some(kwriteconfig)) =
            (&self.kwin_input_method, kwriteconfig_program)
        {
            let mut arguments = KWIN_INPUT_METHOD_KEY.to_vec();
            match previous {
                Some(value) => arguments.push(value),
                None => arguments.push("--delete"),
            }
            if !run(kwriteconfig, &arguments) {
                failures.push("KWin input method".into());
            }
        }
        if let Some(engines) = &self.preload_engines {
            if !run(
                "gsettings",
                &[
                    "set",
                    "org.freedesktop.ibus.general",
                    "preload-engines",
                    engines,
                ],
            ) {
                failures.push("IBus preloaded engines".into());
            }
        }
        if let Some(gnome) = &self.gnome_input_sources {
            if let Err(error) = gnome.restore_with(|key, value| {
                run(
                    "gsettings",
                    &["set", "org.gnome.desktop.input-sources", key, value],
                )
            }) {
                failures.push(format!("GNOME input sources ({error})"));
            }
        }
        if let Some(engine) = self
            .global_engine
            .as_deref()
            .filter(|engine| *engine != "wayexpand")
        {
            if !run("ibus", &["engine", engine]) {
                failures.push("IBus selected engine".into());
            }
        }
        failures
    }
}

const KWIN_INPUT_METHOD_KEY: [&str; 7] = [
    "--notify",
    "--file",
    "kwinrc",
    "--group",
    "Wayland",
    "--key",
    "InputMethod",
];

fn kwriteconfig() -> Option<&'static str> {
    ["kwriteconfig6", "kwriteconfig5"]
        .into_iter()
        .find(|program| command_output("sh", &["-c", &format!("command -v {program}")]).is_some())
}

/// Which daemon unit runs beside IBus after setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompanionService {
    /// The terminal-only evdev daemon (it also serves the control socket).
    Terminals,
    /// The control-only daemon.
    Control,
    /// No companion unit is installed; IBus still expands.
    None,
}

/// Phase 1: everything setup needs, checked before anything changes.
fn preflight(options: IbusSetupOptions, inventory: &ServiceInventory) -> Result<CompanionService> {
    if command_output("sh", &["-c", "command -v ibus"]).is_none() {
        bail!("IBus is not installed; install your distribution's `ibus` package and rerun setup");
    }
    if engine_binary().is_none() {
        bail!("`wayexpand-ibus` is not on PATH; reinstall WayExpand (user installs need ~/.local/bin on PATH)");
    }
    let path = default_config_path();
    Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    if IbusDesktop::detect() == IbusDesktop::KdeWayland
        && !kwin_input_method().is_some_and(|value| value.to_ascii_lowercase().contains("ibus"))
    {
        if !Path::new(KWIN_IBUS_INPUT_METHOD).exists() {
            bail!(
                "{KWIN_IBUS_INPUT_METHOD} is missing; install the IBus GTK 3 panel (for example the `ibus-gtk3` package) and rerun setup"
            );
        }
        if kwriteconfig().is_none() {
            bail!("kwriteconfig6 is not installed; set System Settings → Keyboard → Virtual Keyboard → \"IBus Wayland\" yourself, then rerun setup");
        }
    }
    // Terminal coverage set up earlier stays, unless --terminals is asked
    // for anew: enabling the control unit would stop it (Conflicts=).
    let keep_terminals = !options.terminals
        && terminal_drop_in().is_some_and(|path| path.exists())
        && inventory.state(UserService::Evdev).enabled;
    if options.terminals || keep_terminals {
        if !inventory.installed(UserService::Evdev) {
            bail!("wayexpand-evdev.service is not installed; reinstall WayExpand, then rerun setup with --terminals");
        }
        let marker = evdev_consent_marker()
            .context("cannot locate the evdev consent marker: HOME is not set")?;
        if !marker.exists() {
            bail!(
                "terminal coverage uses evdev, which reads every keystroke (it only expands in terminal_apps).\n\
                 After reviewing docs/BACKENDS_SENSITIVE_FIELDS.md, accept that with:\n  \
                 install -d -m 0700 {}\n  install -m 0600 /dev/null {}\n\
                 then rerun `wayexpand setup --backend ibus --terminals`. Nothing was changed.",
                marker.parent().unwrap_or(&marker).display(),
                marker.display()
            );
        }
        if keep_terminals {
            step("Keeping the terminal coverage set up earlier (wayexpand-evdev.service)");
        }
        return Ok(CompanionService::Terminals);
    }
    if inventory.installed(UserService::Control) {
        Ok(CompanionService::Control)
    } else {
        println!("  Note: wayexpand-control.service is not installed, so quick insert and hotkeys");
        println!("  will not run beside IBus. Reinstall WayExpand to add it; IBus expansion works without it.");
        Ok(CompanionService::None)
    }
}

/// Phase 2 (part): the terminal list and the drop-in that limits the evdev
/// daemon to it. Inert until the unit is started.
fn prepare_terminals() -> Result<()> {
    let apps = ensure_terminal_apps()?;
    let drop_in = terminal_drop_in().context("cannot locate the systemd user directory")?;
    if let Some(parent) = drop_in.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &drop_in,
        "# Written by `wayexpand setup --backend ibus --terminals`.\n\
         # IBus expands in apps; this evdev daemon expands only in settings.terminal_apps.\n\
         [Service]\nEnvironment=WAYEXPAND_TERMINAL_APPS_ONLY=1\n",
    )?;
    step(&format!(
        "Prepared wayexpand-evdev.service to expand only in terminal_apps ({} app IDs)",
        apps.len()
    ));
    Ok(())
}

/// Enable and (re)start `service`, recording it for rollback when setup is
/// what enabled it.
fn start_companion(service: UserService) -> Result<()> {
    let unit = service.unit();
    enable_user_service(unit)?;
    // Pick up a changed drop-in or binary in an already running unit. A
    // successful ActiveState probe alone does not prove this restart took
    // effect or that the intended daemon build/config/backend is serving.
    let restart = run_setup_command("systemctl", &["--user", "restart", unit])?;
    if !restart.success() {
        bail!("systemd failed to restart {unit}; see `journalctl --user -u {unit} -n 50`");
    }
    let mut readiness_detail = "service has not become active".to_owned();
    if !wait_until(Duration::from_secs(8), || {
        if !service_active(unit) {
            readiness_detail = "service is not active".to_owned();
            return false;
        }
        match companion_status_error(service) {
            Ok(()) => true,
            Err(error) => {
                readiness_detail = error.to_string();
                false
            }
        }
    }) {
        bail!(
            "{unit} restart completed, but daemon readiness verification failed: {readiness_detail}; see `journalctl --user -u {unit} -n 50`{}",
            if service == UserService::Evdev {
                " (it needs /dev/input access: scripts/install-evdev-permissions.sh)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

/// Stop capture daemons that would expand everywhere next to IBus.
fn stop_global_capture(options: IbusSetupOptions, companion: CompanionService) -> Result<()> {
    for service in [UserService::Evdev, UserService::InputMethod] {
        let unit = service.unit();
        if !service_active(unit) {
            continue;
        }
        // The terminal-only evdev daemon is the companion, not a conflict.
        if service == UserService::Evdev && companion == CompanionService::Terminals {
            continue;
        }
        println!(
            "  {unit} captures keys in every app; with IBus active, snippets would expand twice."
        );
        let stop = options.assume_yes
            || prompt_yes_no(&format!("  Stop and disable {unit}? [Y/n] "), true)?;
        if stop {
            let result = run_setup_command("systemctl", &["--user", "disable", "--now", unit])?;
            if !result.success() {
                bail!("could not stop and disable {unit}; refusing to activate IBus beside an active capture route");
            }
            let after = ServiceInventory::probe().state(service);
            if after.active || after.enabled {
                bail!("{unit} did not become inactive and disabled; refusing to activate IBus beside an active capture route");
            }
            step(&format!("Stopped {unit}"));
        } else {
            bail!("{unit} would expand snippets a second time beside IBus; stop it or use `--terminals`");
        }
    }
    Ok(())
}

/// Phases 3 and 4: change the session, then verify it.
fn activate_and_verify(
    options: IbusSetupOptions,
    companion: CompanionService,
    snapshot: &mut SessionSnapshot,
) -> Result<()> {
    if IbusDesktop::detect() == IbusDesktop::Gnome {
        // Capture must succeed before mutating the GNOME list; this snapshot
        // is also used if any later activation step fails.
        if snapshot.gnome_input_sources.is_none() {
            snapshot.gnome_input_sources =
                Some(crate::gnome_input_sources::Snapshot::capture().context(
                    "GNOME input-source settings are unavailable; refusing partial IBus setup",
                )?);
        }
    }
    match IbusDesktop::detect() {
        IbusDesktop::KdeWayland => route_kde_to_ibus(snapshot)?,
        _ => {
            if IbusDaemonProcess::find().is_none() {
                step("Starting IBus");
                let _ = run_setup_command("ibus", &["start"]);
                wait_until(Duration::from_secs(10), || {
                    IbusDaemonProcess::find().is_some()
                });
            }
        }
    }
    if !wayexpand_backend_ibus::engine_available() {
        // IBus reads components only at startup; there is no reload call.
        step("Restarting IBus so it loads the WayExpand component (input methods pause for a moment)");
        let _ = run_setup_command("ibus", &["restart"]);
        if !wait_until(
            Duration::from_secs(15),
            wayexpand_backend_ibus::engine_available,
        ) {
            bail!("IBus did not load the WayExpand engine; run `wayexpand ibus status` to see which link is broken");
        }
    }
    add_to_preload_engines();
    restart_stale_engine();
    step("Selecting WayExpand as the active input method");
    if IbusDesktop::detect() == IbusDesktop::Gnome {
        snapshot
            .gnome_input_sources
            .as_ref()
            .expect("captured above")
            .register_and_activate()?;
        step("Registered and activated WayExpand in GNOME input sources");
    }
    // The exit status is meaningless for `default`-layout engines; read back.
    let _ = run_setup_command("ibus", &["engine", "wayexpand"]);
    if !wait_until(
        Duration::from_secs(5),
        wayexpand_backend_ibus::global_engine_selected,
    ) {
        bail!("could not select the WayExpand IBus engine; check that `ibus list-engine --name-only` lists `wayexpand`");
    }
    stop_global_capture(options, companion)?;
    match companion {
        CompanionService::Terminals => {
            step("Starting the terminal-only evdev daemon");
            start_companion(UserService::Evdev)?;
        }
        CompanionService::Control => {
            step("Starting wayexpand-control.service for quick insert and hotkeys");
            start_companion(UserService::Control)?;
        }
        CompanionService::None => {}
    }
    // Phase 4: verify with the same checks `wayexpand ibus status` runs.
    let report = chain::diagnose();
    if let Some(failure) = report.first_failure() {
        bail!(
            "verification failed at \"{}\": {}{}",
            failure.title,
            failure.detail,
            failure
                .fix
                .as_deref()
                .map(|fix| format!(" (fix: {fix})"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

/// Repair only GNOME's persistent WayExpand source and activate it. This is
/// deliberately separate from full IBus setup and records the old values so
/// selection failure cannot leave a partial GNOME source mutation behind.
pub(crate) fn repair_gnome_input_source() -> Result<()> {
    if IbusDesktop::detect() != IbusDesktop::Gnome {
        bail!("GNOME input-source repair must run inside a GNOME session");
    }
    if !wayexpand_backend_ibus::engine_available() {
        bail!("the WayExpand IBus engine is not registered; run `wayexpand setup --backend ibus` first");
    }
    // `ibus engine` can print the active default-layout engine and still exit
    // nonzero; use the route probe for WayExpand before relying on exit status.
    let previous_engine = if wayexpand_backend_ibus::global_engine_selected() {
        "wayexpand".to_owned()
    } else {
        command_output("ibus", &["engine"])
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("could not read the currently selected IBus engine"))?
    };
    let snapshot = SessionSnapshot {
        global_engine: Some(previous_engine),
        gnome_input_sources: Some(
            crate::gnome_input_sources::Snapshot::capture()
                .context("could not capture GNOME input-source settings; no changes made")?,
        ),
        ..SessionSnapshot::default()
    };
    let activation = (|| -> Result<()> {
        snapshot
            .gnome_input_sources
            .as_ref()
            .expect("snapshot captured")
            .register_and_activate()?;
        let _ = run_setup_command("ibus", &["engine", "wayexpand"]);
        if !wait_until(
            Duration::from_secs(5),
            wayexpand_backend_ibus::global_engine_selected,
        ) {
            bail!("IBus did not report WayExpand as the active engine");
        }
        Ok(())
    })();
    if let Err(error) = activation {
        let failures = snapshot.rollback_with(None, |program, arguments| {
            run_setup_command(program, arguments).is_ok_and(|status| status.success())
        });
        bail!(
            "GNOME source repair failed ({error}); prior settings were restored{}",
            if failures.is_empty() {
                String::new()
            } else {
                format!(" with rollback issues: {}", failures.join(", "))
            }
        );
    }
    Ok(())
}

pub(crate) fn configure_ibus(options: IbusSetupOptions) -> Result<()> {
    let inventory = ServiceInventory::probe();
    println!("Checking what the IBus route needs…");
    let companion = preflight(options, &inventory)?;
    step("Preflight passed; nothing has been changed yet");

    // Phase 2: inert preparation. None of this changes the active input
    // method or running services.
    install_component()?;
    let replacement_authorization_required = ensure_replacement_consent(options)?;
    if companion == CompanionService::Terminals && options.terminals {
        prepare_terminals()?;
    }

    // Phases 3–5: change the session, verify, or put it back.
    let service_snapshot = ServiceInventory::probe();
    let mut snapshot = SessionSnapshot::capture(&service_snapshot);
    if let Err(error) = activate_and_verify(options, companion, &mut snapshot) {
        snapshot.rollback();
        return Err(error.context("the previous input-method setup was restored"));
    }

    println!();
    let report = chain::diagnose();
    let status = crate::doctor::status::read_daemon_status()
        .ok()
        .and_then(|response| crate::doctor::status::status_as_json(&response).ok());
    let overlap = crate::commands::ibus::RouteOverlap::from_status(status.as_ref());
    crate::commands::ibus::print_report(&report, &overlap);
    println!();
    if replacement_authorization_required {
        println!("SETUP INCOMPLETE: WX-POLICY-001");
        println!("IBus was configured and activated, but text replacement is disabled by policy.");
        println!("Required user action: review and authorize compatibility_mode = \"tracked-focus\" under [settings], then rerun `wayexpand setup --backend ibus`.");
        bail!("WX-POLICY-001: IBus is active, but replacement authorization is required before expansions can work")
    }
    println!("WayExpand IBus setup is configured; a real target-app insertion has not yet been verified.");
    println!("To complete the first-run check, run `wayexpand ibus watch`, switch to a test application, and type a harmless trigger.");
    if companion != CompanionService::Terminals {
        println!("Terminals cannot use IBus; add `--terminals` to cover them as well.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_off_is_reported_as_incomplete_setup() {
        assert!(replacement_authorization_required(
            wayexpand_core::CompatibilityMode::Off
        ));
        assert!(!replacement_authorization_required(
            wayexpand_core::CompatibilityMode::TrackedFocus
        ));
    }

    fn healthy_companion_status(source: &str) -> serde_json::Value {
        serde_json::json!({
            "status_schema": crate::CONTROL_STATUS_SCHEMA,
            "daemon_commit": crate::build_info::COMMIT,
            "source": source,
            "backend": "libei",
            "config_state": "ok",
            "config_generation": 1,
            "config_active_path": default_config_path().display().to_string(),
            "output_connection_state": "connected",
            "output_health": "ok",
            "state": if source == "evdev" { "connected" } else { "running" },
            "route_available": source == "evdev",
            "matching_available": source == "evdev",
            "input_connection_state": if source == "evdev" { "connected" } else { "unknown" },
        })
    }

    #[test]
    fn companion_readiness_checks_build_schema_config_backend_and_runtime() {
        let mut status = healthy_companion_status("evdev");
        assert_eq!(companion_readiness_error(UserService::Evdev, &status), None);

        status["daemon_commit"] = "older-build".into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("build identity"));
        status = healthy_companion_status("evdev");
        status["status_schema"] = (crate::CONTROL_STATUS_SCHEMA - 1).into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("schema"));
        status = healthy_companion_status("evdev");
        status["backend"] = "other".into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("expected libei"));
        status = healthy_companion_status("evdev");
        status["config_generation"] = 0.into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("configuration"));
        status = healthy_companion_status("evdev");
        status["matching_available"] = false.into();
        assert!(companion_readiness_error(UserService::Evdev, &status)
            .unwrap()
            .contains("capture or matching"));
    }

    #[test]
    fn control_companion_requires_healthy_output_but_not_text_capture() {
        let status = healthy_companion_status("stdin");
        assert_eq!(
            companion_readiness_error(UserService::Control, &status),
            None
        );
    }

    #[test]
    fn gsettings_string_lists_round_trip() {
        assert_eq!(parse_string_list("@as []\n"), Some(Vec::new()));
        assert_eq!(
            parse_string_list("['xkb:us::eng', 'wayexpand']\n"),
            Some(vec!["xkb:us::eng".to_owned(), "wayexpand".to_owned()])
        );
        assert_eq!(parse_string_list("not a list"), None);
        assert_eq!(
            format_string_list(&["xkb:us::eng".into(), "wayexpand".into()]),
            "['xkb:us::eng', 'wayexpand']"
        );
    }

    #[test]
    fn installed_manifest_names_the_engine_by_absolute_path() {
        let manifest = component_manifest(Path::new("/home/a&b/.local/bin/wayexpand-ibus"));
        assert!(manifest.contains("<exec>/home/a&amp;b/.local/bin/wayexpand-ibus</exec>"));
        assert!(manifest.contains("<name>org.freedesktop.IBus.WayExpand</name>"));
    }

    #[cfg(unix)]
    #[test]
    fn stale_engine_scan_lists_only_older_wayexpand_ibus_processes() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wayexpand-stale-ibus-test-{}-{unique}",
            std::process::id()
        ));
        let proc_root = root.join("proc");
        fs::create_dir_all(&proc_root).unwrap();
        let installed = root.join("current-engine");
        let old = root.join("previous-engine");
        fs::write(&installed, b"current").unwrap();
        fs::write(&old, b"previous").unwrap();

        for (pid, executable, comm) in [
            (101, &installed, "wayexpand-ibus"),
            (202, &old, "wayexpand-ibus"),
            (303, &old, "other-process"),
            (404, &old, "ibus-wrapper"),
        ] {
            let process = proc_root.join(pid.to_string());
            fs::create_dir(&process).unwrap();
            fs::write(process.join("comm"), format!("{comm}\n")).unwrap();
            symlink(executable, process.join("exe")).unwrap();
        }
        fs::write(
            proc_root.join("404/cmdline"),
            b"/usr/libexec/wayexpand-ibus\0--ibus\0",
        )
        .unwrap();

        let stale = stale_engine_processes_at(
            &proc_root,
            unsafe { libc::geteuid() },
            &fs::canonicalize(&installed).unwrap(),
        );
        assert_eq!(
            stale,
            vec![
                StaleEngineProcess {
                    pid: 202,
                    executable: old.clone(),
                },
                StaleEngineProcess {
                    pid: 404,
                    executable: old.clone(),
                }
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_terminal_apps_are_valid_terminal_app_ids() {
        let mut config =
            Config::parse("[[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\n").unwrap();
        config.settings.terminal_apps = DEFAULT_TERMINAL_APPS
            .iter()
            .map(|app| (*app).to_owned())
            .collect();
        config.validate().unwrap();
    }

    #[test]
    fn rollback_restores_service_enablement_and_activity_independently() {
        let inventory = ServiceInventory::from_states(vec![
            (
                UserService::Evdev,
                UnitState {
                    installed: true,
                    enabled: false,
                    active: true,
                    failed: false,
                },
            ),
            (
                UserService::Control,
                UnitState {
                    installed: true,
                    enabled: true,
                    active: false,
                    failed: false,
                },
            ),
        ]);
        let snapshot = SessionSnapshot {
            global_engine: Some("xkb:us::eng".into()),
            preload_engines: Some("['xkb:us::eng']".into()),
            kwin_input_method: Some(Some("old-input-method.so".into())),
            gnome_input_sources: None,
            services: UserService::ALL
                .into_iter()
                .map(|service| OriginalServiceState::capture(service, inventory.state(service)))
                .collect(),
        };
        let mut actions = Vec::<(String, Vec<String>)>::new();
        let failures = snapshot.rollback_with(Some("kwriteconfig6"), |program, arguments| {
            actions.push((
                program.to_owned(),
                arguments
                    .iter()
                    .map(|argument| (*argument).to_owned())
                    .collect(),
            ));
            true
        });
        assert!(failures.is_empty());

        assert_eq!(
            &actions[..4],
            [
                (
                    "systemctl".into(),
                    vec!["--user", "disable", "wayexpand-evdev.service"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "systemctl".into(),
                    vec!["--user", "start", "wayexpand-evdev.service"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "systemctl".into(),
                    vec!["--user", "enable", "wayexpand-control.service"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "systemctl".into(),
                    vec!["--user", "stop", "wayexpand-control.service"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
            ]
        );
        assert_eq!(
            &actions[4..],
            [
                (
                    "kwriteconfig6".into(),
                    KWIN_INPUT_METHOD_KEY
                        .into_iter()
                        .chain(["old-input-method.so"])
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "gsettings".into(),
                    vec![
                        "set",
                        "org.freedesktop.ibus.general",
                        "preload-engines",
                        "['xkb:us::eng']",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                ),
                (
                    "ibus".into(),
                    vec!["engine", "xkb:us::eng"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
            ]
        );
    }
}
