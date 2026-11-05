//! `wayexpand setup --backend ibus [--terminals]`: configure the IBus route
//! end to end and verify every link, so users never wire IBus by hand.
//!
//! Each step is idempotent and checks the live result rather than trusting a
//! command's exit status (`ibus engine` exits 1 after a successful switch to
//! an engine whose layout is `default`).

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
    /// Also cover terminals with a terminal-only evdev daemon.
    pub(crate) terminals: bool,
}

/// Run `program` with a deadline and return its stdout when it succeeds.
fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
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
fn route_kde_to_ibus() -> Result<()> {
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
    let kwriteconfig = ["kwriteconfig6", "kwriteconfig5"]
        .into_iter()
        .find(|program| command_output("sh", &["-c", &format!("command -v {program}")]).is_some())
        .ok_or_else(|| anyhow::anyhow!("kwriteconfig6 is not installed; set System Settings → Keyboard → Virtual Keyboard → \"IBus Wayland\" yourself, then rerun setup"))?;
    let base = [
        "--notify",
        "--file",
        "kwinrc",
        "--group",
        "Wayland",
        "--key",
        "InputMethod",
    ];
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
    let Some(installed) = engine_binary().and_then(|path| fs::canonicalize(path).ok()) else {
        return;
    };
    let uid = unsafe { libc::geteuid() };
    let Ok(entries) = fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let proc_dir = entry.path();
        let owned = fs::metadata(&proc_dir)
            .is_ok_and(|metadata| std::os::unix::fs::MetadataExt::uid(&metadata) == uid);
        let is_engine = fs::read_to_string(proc_dir.join("comm"))
            .is_ok_and(|comm| comm.trim() == "wayexpand-ibus");
        if !owned || !is_engine {
            continue;
        }
        let running = fs::read_link(proc_dir.join("exe")).unwrap_or_default();
        if running != installed {
            step("Stopping an IBus engine from an older installation");
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
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
fn ensure_replacement_consent(options: IbusSetupOptions) -> Result<()> {
    let path = default_config_path();
    let loaded = Config::load_versioned(&path).map_err(|error| config_load_error(&path, error))?;
    if loaded.config.settings.compatibility_mode != wayexpand_core::CompatibilityMode::Off {
        return Ok(());
    }
    println!("  IBus checks that the text before the cursor is your trigger, then deletes it and inserts");
    println!("  the snippet as two separate steps. compatibility_mode = \"off\" refuses that, so nothing would expand.");
    if options.assume_yes {
        println!("  Not changed by --yes. Enable it with compatibility_mode = \"tracked-focus\" under [settings] in {}", path.display());
        return Ok(());
    }
    if !prompt_yes_no(
        "  Allow verified two-step replacement (compatibility_mode = \"tracked-focus\")? [Y/n] ",
        true,
    )? {
        println!(
            "  Left compatibility_mode off; IBus will recognize triggers but not replace them."
        );
        return Ok(());
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
    Ok(())
}

/// Cover terminals with an evdev daemon limited to `terminal_apps`.
fn configure_terminals() -> Result<()> {
    let apps = ensure_terminal_apps()?;
    let marker = evdev_consent_marker()
        .context("cannot locate the evdev consent marker: HOME is not set")?;
    if !marker.exists() {
        println!("  Terminals were not enabled: terminal coverage uses evdev, which reads every keystroke (it only expands in terminal_apps).");
        println!("  After reviewing docs/BACKENDS_SENSITIVE_FIELDS.md, accept that with:");
        println!(
            "    install -d -m 0700 {}\n    install -m 0600 /dev/null {}",
            marker.parent().unwrap_or(&marker).display(),
            marker.display()
        );
        println!("  then rerun `wayexpand setup --backend ibus --terminals`.");
        return Ok(());
    }
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
        "Limiting wayexpand-evdev.service to terminal_apps ({} app IDs)",
        apps.len()
    ));
    enable_user_service("wayexpand-evdev.service")?;
    let _ = run_setup_command(
        "systemctl",
        &["--user", "restart", "wayexpand-evdev.service"],
    );
    if !wait_until(Duration::from_secs(5), || {
        service_active("wayexpand-evdev.service")
    }) {
        println!("  wayexpand-evdev.service did not stay running; see `journalctl --user -u wayexpand-evdev.service` (it needs /dev/input access: scripts/install-evdev-permissions.sh)");
    }
    Ok(())
}

/// Stop capture daemons that would expand everywhere next to IBus.
fn stop_global_capture(options: IbusSetupOptions) -> Result<()> {
    let drop_in_present = terminal_drop_in().is_some_and(|path| path.exists());
    for service in ["wayexpand-evdev.service", "wayexpand-input-method.service"] {
        if !service_active(service) {
            continue;
        }
        if service == "wayexpand-evdev.service" && drop_in_present {
            continue;
        }
        println!("  {service} captures keys in every app; with IBus active, snippets would expand twice.");
        let stop = options.assume_yes
            || prompt_yes_no(&format!("  Stop and disable {service}? [Y/n] "), true)?;
        if stop {
            let _ = run_setup_command("systemctl", &["--user", "disable", "--now", service]);
            step(&format!("Stopped {service}"));
        } else {
            println!(
                "  Left {service} running; use `--terminals` to limit evdev to terminals instead."
            );
        }
    }
    Ok(())
}

pub(crate) fn configure_ibus(options: IbusSetupOptions) -> Result<()> {
    if command_output("sh", &["-c", "command -v ibus"]).is_none() {
        bail!("IBus is not installed; install your distribution's `ibus` package and rerun setup");
    }
    if engine_binary().is_none() {
        bail!("`wayexpand-ibus` is not on PATH; reinstall WayExpand (user installs need ~/.local/bin on PATH)");
    }
    println!("Configuring the WayExpand IBus route...");
    install_component()?;
    let desktop = IbusDesktop::detect();
    match desktop {
        IbusDesktop::KdeWayland => route_kde_to_ibus()?,
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
        step("Restarting IBus so it loads the WayExpand component");
        let _ = run_setup_command("ibus", &["restart"]);
        if !wait_until(
            Duration::from_secs(15),
            wayexpand_backend_ibus::engine_available,
        ) {
            bail!("IBus did not load the WayExpand engine; run `wayexpand ibus status` to see which link is broken");
        }
    }
    add_to_preload_engines();
    ensure_replacement_consent(options)?;
    restart_stale_engine();
    step("Selecting WayExpand as the active input method");
    // The exit status is meaningless for `default`-layout engines; read back.
    let _ = run_setup_command("ibus", &["engine", "wayexpand"]);
    if !wait_until(
        Duration::from_secs(5),
        wayexpand_backend_ibus::global_engine_selected,
    ) {
        bail!("could not select the WayExpand IBus engine; check that `ibus list-engine --name-only` lists `wayexpand`");
    }
    if options.terminals {
        configure_terminals()?;
    } else {
        stop_global_capture(options)?;
    }
    println!();
    let report = chain::diagnose();
    let status = crate::doctor::status::read_daemon_status()
        .ok()
        .and_then(|response| crate::doctor::status::status_as_json(&response).ok());
    let overlap = crate::commands::ibus::RouteOverlap::from_status(status.as_ref());
    let ready = crate::commands::ibus::print_report(&report, &overlap);
    println!();
    if ready {
        println!("WayExpand IBus integration is active.");
        println!("Check any app with `wayexpand ibus watch`: switch to it and type a trigger.");
        if !options.terminals {
            println!("Terminals cannot use IBus; add `--terminals` to cover them as well.");
        }
        Ok(())
    } else {
        bail!("the IBus route is not ready yet; fix the first ✗ above and rerun setup")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
