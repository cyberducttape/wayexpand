//! End-to-end checks of the IBus route, one link at a time.
//!
//! Each check names the link that failed and the exact fix, so a user (or
//! `wayexpand setup`) never needs a D-Bus monitor to find out why the IBus
//! engine is not expanding. Checks read the *running* ibus-daemon's own
//! environment where it matters: IBus loads components from its own
//! `IBUS_COMPONENT_PATH`, not from the caller's.

use crate::{
    availability::{
        component_directories_for, executable_in_path, ibus_registry_contains_engine,
        COMPONENT_FILE_NAMES, ENGINE_NAME,
    },
    global_engine_selected,
    service::BUS_NAME,
    telemetry::{read_status, IbusStatusReading},
};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::Read,
    os::unix::{ffi::OsStrExt, fs::MetadataExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_ENVIRON_BYTES: u64 = 256 * 1024;
/// The KWin Virtual Keyboard entry that launches IBus with Wayland support.
pub const KWIN_IBUS_INPUT_METHOD: &str =
    "/usr/share/applications/org.freedesktop.IBus.Panel.Wayland.Gtk3.desktop";

/// Outcome of one check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IbusCheckState {
    Pass,
    /// Works, but needs attention (for example not selected yet).
    Warn,
    /// This link is broken; IBus expansion cannot work until it is fixed.
    Fail,
    /// Not applicable here, or not checked because an earlier link failed.
    Skip,
    /// The available evidence cannot establish this condition.
    #[default]
    Unknown,
}

impl IbusCheckState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Skip => "skip",
            Self::Unknown => "unknown",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
            Self::Skip => "·",
            Self::Unknown => "?",
        }
    }
}

/// One link in the IBus route.
#[derive(Debug, Clone)]
pub struct IbusCheck {
    /// Stable identifier for JSON consumers.
    pub id: &'static str,
    pub title: &'static str,
    pub state: IbusCheckState,
    /// Stable support identifier for a GNOME input-method failure.
    pub problem_id: Option<&'static str>,
    pub detail: String,
    /// A command or action that fixes this link, when one is known.
    pub fix: Option<String>,
}

/// How the desktop routes input methods, which decides who starts IBus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IbusDesktop {
    KdeWayland,
    Gnome,
    OtherWayland,
    X11,
    Unknown,
}

impl IbusDesktop {
    pub fn label(self) -> &'static str {
        match self {
            Self::KdeWayland => "kde-wayland",
            Self::Gnome => "gnome",
            Self::OtherWayland => "other-wayland",
            Self::X11 => "x11",
            Self::Unknown => "unknown",
        }
    }

    pub fn detect() -> Self {
        let desktop = env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let wayland = env::var_os("WAYLAND_DISPLAY").is_some()
            || env::var("XDG_SESSION_TYPE").is_ok_and(|kind| kind == "wayland");
        if desktop.split(':').any(|part| part == "gnome") {
            Self::Gnome
        } else if wayland && desktop.split(':').any(|part| part == "kde") {
            Self::KdeWayland
        } else if wayland {
            Self::OtherWayland
        } else if env::var_os("DISPLAY").is_some() {
            Self::X11
        } else {
            Self::Unknown
        }
    }
}

/// The result of [`diagnose`].
#[derive(Debug, Clone)]
pub struct IbusChainReport {
    pub desktop: IbusDesktop,
    pub checks: Vec<IbusCheck>,
    /// The engine's published activity, if any.
    pub activity: Option<IbusStatusReading>,
}

/// The separate conditions that "the IBus route works" is made of. Each
/// one is evidence for the next, but none implies the ones after it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct IbusReadiness {
    /// The configured engine and GNOME source are present and policy allows
    /// replacement. This does not prove a client has sent input.
    pub configuration: IbusCheckState,
    /// Live route and safe-field input evidence. Unknown is never success.
    pub operational: IbusCheckState,
    /// The component manifest is where the running IBus reads it.
    pub component_installed: bool,
    /// IBus' live registry lists the engine.
    pub registered: bool,
    /// An ibus-daemon is running for this user.
    pub ibus_running: bool,
    /// WayExpand is the active (global) input method.
    pub selected: bool,
    /// The engine process is running and publishing activity.
    pub engine_running: bool,
    /// At least one recent field sent keys and reported text. IBus telemetry
    /// does not identify the application, so this is not target-specific proof.
    pub text_received: bool,
    /// At least one expansion was committed since the engine started.
    pub expanded: bool,
}

impl IbusReadiness {
    /// Stable names, in order, for display.
    pub fn stages(&self) -> [(&'static str, bool); 7] {
        [
            ("component_installed", self.component_installed),
            ("registered", self.registered),
            ("ibus_running", self.ibus_running),
            ("selected", self.selected),
            ("engine_running", self.engine_running),
            ("text_received", self.text_received),
            ("expanded", self.expanded),
        ]
    }
}

impl IbusChainReport {
    /// Each readiness condition on its own, from the checks and the engine's
    /// published activity.
    pub fn readiness(&self) -> IbusReadiness {
        let passed = |id: &str| {
            self.check(id)
                .is_some_and(|check| check.state == IbusCheckState::Pass)
        };
        let live = self.activity.as_ref().filter(|reading| reading.live);
        IbusReadiness {
            configuration: self.configuration_readiness(),
            operational: self.operational_readiness(),
            component_installed: passed("component") && passed("manifest"),
            registered: passed("registered"),
            ibus_running: passed("daemon"),
            selected: passed("selected"),
            engine_running: live.is_some(),
            text_received: passed("field-input"),
            expanded: live.is_some_and(|reading| reading.status.totals.expansions > 0),
        }
    }

    /// True only when operational evidence is complete and successful.
    pub fn ready(&self) -> bool {
        self.operational_readiness() == IbusCheckState::Pass
    }

    /// Configuration can be complete while end-to-end operation is still
    /// unknown until a normal text field has sent input to the engine.
    pub fn configuration_readiness(&self) -> IbusCheckState {
        self.aggregate(&[
            "tools",
            "component",
            "manifest",
            "manifest-scope",
            "registered",
            "consent",
            "gnome-session",
            "gnome-source-registered",
        ])
    }

    pub fn operational_readiness(&self) -> IbusCheckState {
        let mut ids = vec![
            "daemon",
            "selected",
            "engine",
            "gnome-source-active",
            "field-input",
            "target-insertion",
        ];
        // GNOME is verified by explicit session/source and field-input
        // evidence. Its desktop name alone is never a routing pass.
        if self.desktop != IbusDesktop::Gnome {
            ids.push("desktop");
        }
        if self.desktop == IbusDesktop::KdeWayland {
            ids.push("toolkit-environment");
            ids.push("desktop-shell-environment");
        }
        self.aggregate(&ids)
    }

    fn aggregate(&self, ids: &[&str]) -> IbusCheckState {
        let mut state = IbusCheckState::Pass;
        let mut found = false;
        for check in self.checks.iter().filter(|check| ids.contains(&check.id)) {
            found = true;
            state = match (state, check.state) {
                (_, IbusCheckState::Fail) | (IbusCheckState::Fail, _) => IbusCheckState::Fail,
                (_, IbusCheckState::Warn) | (IbusCheckState::Warn, _) => IbusCheckState::Warn,
                (_, IbusCheckState::Unknown) | (IbusCheckState::Unknown, _) => {
                    IbusCheckState::Unknown
                }
                (_, IbusCheckState::Skip) | (IbusCheckState::Skip, _) => IbusCheckState::Unknown,
                _ => IbusCheckState::Pass,
            };
        }
        if found {
            state
        } else {
            IbusCheckState::Unknown
        }
    }

    /// The first broken link, which is the one to fix first.
    pub fn first_failure(&self) -> Option<&IbusCheck> {
        self.checks
            .iter()
            .find(|check| check.state == IbusCheckState::Fail)
    }

    pub fn check(&self, id: &str) -> Option<&IbusCheck> {
        self.checks.iter().find(|check| check.id == id)
    }

    /// Whether any WayExpand IBus component or binary is present at all, so
    /// callers can stay quiet on systems that never installed it.
    pub fn installed_anywhere(&self) -> bool {
        self.check("tools")
            .is_some_and(|check| check.state != IbusCheckState::Fail)
            || self
                .check("component")
                .is_some_and(|check| check.state == IbusCheckState::Pass)
    }
}

/// A running ibus-daemon owned by this user.
#[derive(Debug, Clone)]
pub struct IbusDaemonProcess {
    pub pid: u32,
    /// The daemon's own environment; `None` when it cannot be read.
    environment: Option<Vec<(OsString, OsString)>>,
}

impl IbusDaemonProcess {
    pub fn find() -> Option<Self> {
        let uid = unsafe { libc::geteuid() };
        let entries = fs::read_dir("/proc").ok()?;
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let proc_dir = entry.path();
            if fs::metadata(&proc_dir).map(|m| m.uid()).ok() != Some(uid) {
                continue;
            }
            if fs::read_to_string(proc_dir.join("comm"))
                .is_ok_and(|comm| comm.trim() == "ibus-daemon")
            {
                return Some(Self {
                    pid,
                    environment: read_environment(&proc_dir.join("environ")),
                });
            }
        }
        None
    }

    fn var(&self, name: &str) -> Option<&OsStr> {
        self.environment.as_ref()?.iter().find_map(|(key, value)| {
            (key.as_os_str() == OsStr::new(name)).then_some(value.as_os_str())
        })
    }

    /// Component directories this daemon loads, and whether that answer
    /// came from its real environment.
    pub fn component_directories(&self) -> (Vec<PathBuf>, bool) {
        match &self.environment {
            Some(_) => (
                component_directories_for(self.var("IBUS_COMPONENT_PATH")),
                true,
            ),
            None => (
                component_directories_for(env::var_os("IBUS_COMPONENT_PATH").as_deref()),
                false,
            ),
        }
    }

    fn path(&self) -> Option<OsString> {
        match &self.environment {
            Some(_) => self.var("PATH").map(OsStr::to_os_string),
            None => env::var_os("PATH"),
        }
    }
}

fn read_environment(path: &Path) -> Option<Vec<(OsString, OsString)>> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    use std::io::Read;
    file.take(MAX_ENVIRON_BYTES).read_to_end(&mut bytes).ok()?;
    Some(
        bytes
            .split(|byte| *byte == 0)
            .filter_map(|entry| {
                let separator = entry.iter().position(|byte| *byte == b'=')?;
                Some((
                    OsStr::from_bytes(&entry[..separator]).to_os_string(),
                    OsStr::from_bytes(&entry[separator + 1..]).to_os_string(),
                ))
            })
            .collect(),
    )
}

/// Whether `pid` descends from a process named `ancestor` (for example
/// whether KWin started this ibus-daemon), looking a few levels up.
pub fn started_by(pid: u32, ancestor: &str) -> bool {
    let mut current = pid;
    for _ in 0..6 {
        let Ok(stat) = fs::read_to_string(format!("/proc/{current}/stat")) else {
            return false;
        };
        // Fields after the parenthesized command: state, then ppid.
        let Some(parent) = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|ppid| ppid.parse::<u32>().ok())
        else {
            return false;
        };
        if parent <= 1 {
            return false;
        }
        if fs::read_to_string(format!("/proc/{parent}/comm"))
            .is_ok_and(|comm| comm.trim() == ancestor)
        {
            return true;
        }
        current = parent;
    }
    false
}

/// Where setup should install the component so the running (or next)
/// ibus-daemon loads it: its first component directory.
pub fn component_install_directory() -> PathBuf {
    let directories = IbusDaemonProcess::find()
        .map(|daemon| daemon.component_directories().0)
        .unwrap_or_else(|| {
            component_directories_for(env::var_os("IBUS_COMPONENT_PATH").as_deref())
        });
    directories
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("/usr/share/ibus/component"))
}

/// A component manifest found on disk.
#[derive(Debug, Clone)]
pub struct IbusManifest {
    pub path: PathBuf,
    pub name: Option<String>,
    pub exec: Option<String>,
}

fn element(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim().to_owned())
}

impl IbusManifest {
    fn read(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
            return None;
        }
        let text = fs::read_to_string(path).ok()?;
        Some(Self {
            path: path.to_path_buf(),
            // The first <name> is the component's; engine names follow.
            name: element(&text, "name"),
            exec: element(&text, "exec"),
        })
    }

    /// Find a WayExpand manifest in `directories`, in IBus' search order.
    pub fn find_in(directories: &[PathBuf]) -> Option<Self> {
        directories.iter().find_map(|directory| {
            COMPONENT_FILE_NAMES
                .iter()
                .map(|name| directory.join(name))
                .find(|path| path.is_file())
                .and_then(|path| Self::read(&path))
        })
    }
}

/// A manifest in the per-user data directory, which ibus-daemon never reads.
pub fn ignored_user_manifest() -> Option<PathBuf> {
    let data_home = env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
    let directory = data_home.join("ibus/component");
    COMPONENT_FILE_NAMES
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.exists())
}

fn executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn resolve_exec(exec: &str, search_path: Option<&OsStr>) -> Option<PathBuf> {
    let program = exec.split_whitespace().next()?;
    let program = Path::new(program);
    if program.is_absolute() {
        return executable(program).then(|| program.to_path_buf());
    }
    env::split_paths(search_path?)
        .map(|directory| directory.join(program))
        .find(|candidate| executable(candidate))
}

fn shared_manifest_uses_user_home(
    manifest_path: &Path,
    resolved_exec: &Path,
    homes: &[PathBuf],
) -> bool {
    // A manifest in a shared IBus directory must not bind all desktop users
    // to an executable installed in one account's home directory. Ignore a
    // per-user manifest path; it is not a shared registration.
    !homes.iter().any(|home| manifest_path.starts_with(home))
        && homes.iter().any(|home| resolved_exec.starts_with(home))
}

fn known_user_homes() -> Vec<PathBuf> {
    const MAX_PASSWD_BYTES: u64 = 1024 * 1024;
    let mut homes = Vec::new();
    if let Ok(metadata) = fs::metadata("/etc/passwd") {
        if metadata.is_file() && metadata.len() <= MAX_PASSWD_BYTES {
            if let Ok(contents) = fs::read_to_string("/etc/passwd") {
                homes.extend(contents.lines().take(4096).filter_map(|line| {
                    let mut fields = line.split(':');
                    let _name = fields.next()?;
                    let _password = fields.next()?;
                    let _uid = fields.next()?;
                    let _gid = fields.next()?;
                    let _gecos = fields.next()?;
                    let home = PathBuf::from(fields.next()?);
                    (home.is_absolute() && home != Path::new("/") && home.is_dir()).then_some(home)
                }));
            }
        }
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        homes.push(home);
    }
    homes = homes
        .into_iter()
        .map(|home| fs::canonicalize(&home).unwrap_or(home))
        .collect();
    homes.sort();
    homes.dedup();
    homes
}

/// Read KWin's configured input method (`[Wayland] InputMethod`).
pub fn kwin_input_method() -> Option<String> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let text = fs::read_to_string(config_home.join("kwinrc")).ok()?;
    parse_kwin_input_method(&text)
}

fn parse_kwin_input_method(text: &str) -> Option<String> {
    let mut in_wayland = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_wayland = line == "[Wayland]";
        } else if in_wayland {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            // KConfig appends [$e] when a value uses environment expansion.
            // Compare the base key so an ordinary `InputMethod=` fixture and
            // KWin's persisted `InputMethod[$e]=...` form mean the same thing.
            let key = key.trim();
            if key.split_once('[').map_or(key, |(base, _)| base) == "InputMethod" {
                return Some(value.trim().to_owned());
            }
        }
    }
    None
}

/// Ask KWin whether its virtual-keyboard (input method) slot is running.
/// `None` when KWin does not answer within a second.
pub fn kwin_input_method_available() -> Option<bool> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("wayexpand-kwin-im-probe".into())
        .spawn(move || {
            let answer = (|| -> zbus::Result<bool> {
                let connection = zbus::blocking::Connection::session()?;
                let reply = connection.call_method(
                    Some("org.kde.KWin"),
                    "/VirtualKeyboard",
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("org.kde.kwin.VirtualKeyboard", "available"),
                )?;
                let value: zbus::zvariant::OwnedValue = reply.body().deserialize()?;
                bool::try_from(value).map_err(zbus::Error::from)
            })();
            let _ = sender.send(answer.ok());
        })
        .ok()?;
    receiver.recv_timeout(Duration::from_secs(1)).ok().flatten()
}

/// Whether the user's configuration lets IBus replace triggers.
fn replacement_consent_check() -> IbusCheck {
    const TITLE: &str = "Replacements are allowed";
    let path = wayexpand_core::default_config_path();
    match wayexpand_core::Config::load(&path) {
        Ok(config) if config.settings.compatibility_mode == wayexpand_core::CompatibilityMode::Off => check(
            "consent",
            TITLE,
            IbusCheckState::Fail,
            "compatibility_mode is \"off\", so every IBus replacement is refused (IBus deletes the verified trigger and inserts the snippet as two steps)",
            Some("run `wayexpand setup --backend ibus` and accept, or set compatibility_mode = \"tracked-focus\" under [settings] (Settings > Engine in wayexpand-gui)".into()),
        ),
        Ok(config) => check(
            "consent",
            TITLE,
            IbusCheckState::Pass,
            format!("compatibility_mode is \"{}\"", config.settings.compatibility_mode.as_str()),
            None,
        ),
        Err(error) => check(
            "consent",
            TITLE,
            IbusCheckState::Warn,
            format!("could not read {}: {}", path.display(), error.safe_summary()),
            Some("run `wayexpand validate`".into()),
        ),
    }
}

fn check(
    id: &'static str,
    title: &'static str,
    state: IbusCheckState,
    detail: impl Into<String>,
    fix: Option<String>,
) -> IbusCheck {
    IbusCheck {
        id,
        title,
        state,
        problem_id: (state != IbusCheckState::Pass && id == "gnome-session")
            .then_some("WX-GNOME-SESSION-001")
            .or_else(|| {
                (state != IbusCheckState::Pass && id == "gnome-source-registered")
                    .then_some("WX-GNOME-IM-001")
            })
            .or_else(|| {
                (state != IbusCheckState::Pass && id == "gnome-source-active")
                    .then_some("WX-GNOME-IM-002")
            })
            .or_else(|| {
                (state == IbusCheckState::Fail
                    && matches!(id, "component" | "manifest" | "registered" | "tools"))
                .then_some("WX-IBUS-001")
            })
            .or_else(|| {
                (state != IbusCheckState::Pass && id == "manifest-scope")
                    .then_some("WX-IBUS-MULTIUSER-001")
            })
            .or_else(|| {
                (state == IbusCheckState::Fail && id == "selected").then_some("WX-IBUS-002")
            })
            .or_else(|| (state == IbusCheckState::Fail && id == "daemon").then_some("WX-IBUS-004"))
            .or_else(|| {
                (state != IbusCheckState::Pass && id == "field-input").then_some("WX-IBUS-003")
            })
            .or(match (id, state) {
                ("target-insertion", IbusCheckState::Fail) => Some("WX-IBUS-005"),
                ("target-insertion", IbusCheckState::Warn) => Some("WX-IBUS-006"),
                ("target-insertion", IbusCheckState::Unknown) => Some("WX-IBUS-007"),
                _ => None,
            })
            .or_else(|| {
                (state != IbusCheckState::Pass
                    && matches!(id, "toolkit-environment" | "desktop-shell-environment"))
                .then_some("WX-IBUS-TOOLKIT-001")
            })
            .or_else(|| {
                (state == IbusCheckState::Fail && id == "consent").then_some("WX-POLICY-001")
            }),
        detail: detail.into(),
        fix,
    }
}

fn skipped(id: &'static str, title: &'static str, because: &str) -> IbusCheck {
    check(
        id,
        title,
        IbusCheckState::Skip,
        format!("not checked: {because}"),
        None,
    )
}

/// Fix text for getting IBus started by the desktop.
fn start_ibus_fix(desktop: IbusDesktop) -> String {
    match desktop {
        IbusDesktop::KdeWayland => "System Settings → Keyboard → Virtual Keyboard → \"IBus Wayland\", or run `wayexpand setup --backend ibus`".into(),
        IbusDesktop::Gnome => "GNOME starts IBus itself; log out and back in, or run `ibus start`".into(),
        _ => "start IBus with `ibus start` (or your desktop's input-method settings)".into(),
    }
}

/// Run every check. Cheap enough for each `doctor` run: the slowest steps
/// are two bounded `ibus` calls and a one-second KWin probe.
pub fn diagnose() -> IbusChainReport {
    let desktop = IbusDesktop::detect();
    let mut checks = Vec::new();
    let activity = read_status();

    // 1. Programs.
    let ibus_cli = executable_in_path("ibus");
    let engine_binary = executable_in_path("wayexpand-ibus");
    checks.push(match (ibus_cli, engine_binary) {
        (true, true) => check(
            "tools",
            "IBus and the WayExpand engine are installed",
            IbusCheckState::Pass,
            "`ibus` and `wayexpand-ibus` are on PATH",
            None,
        ),
        (false, _) => check(
            "tools",
            "IBus and the WayExpand engine are installed",
            IbusCheckState::Fail,
            "the `ibus` command is not installed",
            Some("install your distribution's `ibus` package".into()),
        ),
        (true, false) => check(
            "tools",
            "IBus and the WayExpand engine are installed",
            IbusCheckState::Fail,
            "`wayexpand-ibus` is not on PATH",
            Some(
                "reinstall WayExpand, and make sure ~/.local/bin is on PATH for user installs"
                    .into(),
            ),
        ),
    });

    // 2. The daemon, as started by the desktop.
    let daemon = IbusDaemonProcess::find();
    checks.push(ibus_daemon_check(daemon.as_ref(), desktop));

    // 3. Desktop routing: who sends keys to IBus.
    checks.push(match desktop {
        IbusDesktop::KdeWayland => {
            let configured = kwin_input_method();
            let ibus_configured = configured
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains("ibus"));
            if !ibus_configured {
                kwin_unconfigured_input_method(configured.as_deref())
            } else {
                kwin_input_method_check(
                    kwin_input_method_available(),
                    ibus_wayland_helper_running(),
                    recent_ibus_wayland_segv(),
                )
            }
        }
        IbusDesktop::Gnome => skipped(
            "desktop",
            "GNOME input-method routing",
            "reported by the dedicated GNOME session, source, and field-input checks below",
        ),
        IbusDesktop::OtherWayland => check("desktop", "The desktop sends input to IBus", IbusCheckState::Warn, "this compositor must route input methods to IBus; WayExpand cannot verify it", Some("use your compositor's input-method support for IBus (for example `ibus-daemon --panel disable` with input-method-v2), then run `wayexpand ibus watch`".into())),
        IbusDesktop::X11 => check("desktop", "Applications use IBus", IbusCheckState::Warn, "X11 applications reach IBus through GTK_IM_MODULE/QT_IM_MODULE/XMODIFIERS", Some("set GTK_IM_MODULE=ibus QT_IM_MODULE=ibus XMODIFIERS=@im=ibus for your session".into())),
        IbusDesktop::Unknown => check("desktop", "The desktop sends input to IBus", IbusCheckState::Skip, "no graphical session detected", None),
    });
    if desktop == IbusDesktop::KdeWayland {
        let activation_environment = bounded_command_output(
            "systemctl",
            &["--user", "show-environment"],
            Duration::from_secs(2),
        );
        checks.push(plasma_toolkit_environment_check(
            activation_environment.as_deref(),
        ));
        checks.push(plasma_shell_environment_check(plasma_shell_environments()));
    }

    // GNOME keeps a separate typed input-source list; IBus preload/selection
    // alone does not make the engine survive GNOME source switching.
    if desktop == IbusDesktop::Gnome {
        checks.push(gnome_session_check());
        let (registered, active) = gnome_input_source_checks();
        checks.push(registered);
        checks.push(active);
    }

    // 4. The component manifest, where *this daemon* reads components.
    let (directories, from_daemon) = daemon
        .as_ref()
        .map(IbusDaemonProcess::component_directories)
        .unwrap_or_else(|| {
            (
                component_directories_for(env::var_os("IBUS_COMPONENT_PATH").as_deref()),
                false,
            )
        });
    let shown = directories
        .iter()
        .map(|directory| directory.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = IbusManifest::find_in(&directories);
    let source = if from_daemon {
        "the running IBus reads"
    } else {
        "IBus reads by default"
    };
    checks.push(match &manifest {
        Some(found) => check("component", "IBus can find the WayExpand component", IbusCheckState::Pass, format!("{} (in a directory {source})", found.path.display()), None),
        None => {
            let ignored = ignored_user_manifest()
                .map(|path| format!("; {} exists but IBus never reads per-user component directories", path.display()))
                .unwrap_or_default();
            check("component", "IBus can find the WayExpand component", IbusCheckState::Fail, format!("no wayexpand.xml or wayexpand-ibus.xml in {shown} ({source}){ignored}"), Some("run `wayexpand setup --backend ibus` (installs it system-wide after an administrator prompt)".into()))
        }
    });

    // 5. A shared system manifest must not be tied to one user's executable.
    checks.push(match &manifest {
        None => skipped(
            "manifest-scope",
            "Shared IBus registration is multi-user safe",
            "no component found",
        ),
        Some(found) => {
            let exec = found.exec.as_deref().unwrap_or("");
            let search_path = daemon
                .as_ref()
                .and_then(IbusDaemonProcess::path)
                .or_else(|| env::var_os("PATH"));
            let resolved = resolve_exec(exec, search_path.as_deref());
            let homes = known_user_homes();
            let resolved_real = resolved
                .as_deref()
                .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
            match resolved_real.as_deref() {
                Some(program) if shared_manifest_uses_user_home(&found.path, program, &homes) => {
                    check(
                        "manifest-scope",
                        "Shared IBus registration is multi-user safe",
                        IbusCheckState::Warn,
                        format!(
                            "{} is a shared component, but its executable resolves into one user's home directory",
                            found.path.display()
                        ),
                        Some("run `wayexpand setup --backend ibus --install-system-integration` to install the shared engine, or install WayExpand using the distribution package".into()),
                    )
                }
                Some(_) if !homes.is_empty() => check(
                    "manifest-scope",
                    "Shared IBus registration is multi-user safe",
                    IbusCheckState::Pass,
                    "the shared component does not resolve its engine into the current user's home directory",
                    None,
                ),
                _ => check(
                    "manifest-scope",
                    "Shared IBus registration is multi-user safe",
                    IbusCheckState::Unknown,
                    "the IBus executable or current home directory could not be resolved",
                    Some("run `wayexpand doctor --gnome` after restoring the IBus engine executable and HOME environment".into()),
                ),
            }
        }
    });

    // 6. The manifest describes this engine.
    checks.push(match &manifest {
        None => skipped("manifest", "The component matches this engine", "no component found"),
        Some(found) => {
            let name_ok = found.name.as_deref() == Some(BUS_NAME);
            let exec = found.exec.as_deref().unwrap_or("");
            let search_path = daemon.as_ref().and_then(IbusDaemonProcess::path).or_else(|| env::var_os("PATH"));
            let resolved = resolve_exec(exec, search_path.as_deref());
            match (name_ok, &resolved) {
                (true, Some(program)) => check("manifest", "The component matches this engine", IbusCheckState::Pass, format!("{BUS_NAME}, runs {}", program.display()), None),
                (false, _) => check("manifest", "The component matches this engine", IbusCheckState::Fail, format!("the component is named {:?} but the engine registers {BUS_NAME}; IBus would time out selecting it", found.name.as_deref().unwrap_or("")), Some("reinstall the component with `wayexpand setup --backend ibus`".into())),
                (true, None) => check("manifest", "The component matches this engine", IbusCheckState::Fail, format!("IBus cannot run {exec:?}: not found on the daemon's PATH"), Some("run `wayexpand setup --backend ibus`, which writes an absolute engine path".into())),
            }
        }
    });

    // 6. The live registry.
    checks.push(if daemon.is_none() {
        skipped(
            "registered",
            "IBus has loaded the engine",
            "IBus is not running",
        )
    } else if !ibus_cli {
        skipped(
            "registered",
            "IBus has loaded the engine",
            "the `ibus` command is missing",
        )
    } else if ibus_registry_contains_engine() {
        check(
            "registered",
            "IBus has loaded the engine",
            IbusCheckState::Pass,
            format!("`ibus list-engine` lists {ENGINE_NAME}"),
            None,
        )
    } else {
        check(
            "registered",
            "IBus has loaded the engine",
            IbusCheckState::Fail,
            format!("`ibus list-engine` does not list {ENGINE_NAME}"),
            Some(if manifest.is_some() {
                "restart IBus so it reloads components: `ibus restart`".into()
            } else {
                "install the component first (see above)".into()
            }),
        )
    });

    // 7. Selected as the active input method.
    let registered = checks
        .last()
        .is_some_and(|last| last.state == IbusCheckState::Pass);
    checks.push(if !registered {
        skipped(
            "selected",
            "WayExpand is the active input method",
            "the engine is not registered",
        )
    } else if global_engine_selected() {
        check(
            "selected",
            "WayExpand is the active input method",
            IbusCheckState::Pass,
            "`ibus engine` reports wayexpand",
            None,
        )
    } else {
        check(
            "selected",
            "WayExpand is the active input method",
            IbusCheckState::Warn,
            "another IBus input method is active",
            Some("select WayExpand in the IBus menu, or run `ibus engine wayexpand`".into()),
        )
    });

    // 8. Consent for verified, non-atomic replacement. IBus deletes the
    // trigger and commits the replacement as two signals, so with the
    // default `compatibility_mode = "off"` every replacement is refused.
    checks.push(replacement_consent_check());

    // 9. The engine process itself.
    checks.push(match &activity {
        Some(reading) if reading.live => check(
            "engine",
            "The engine is running",
            IbusCheckState::Pass,
            format!(
                "pid {}, {} engine object(s), {} expansion(s) since start",
                reading.status.pid, reading.status.engines, reading.status.totals.expansions
            ),
            None,
        ),
        _ if !registered => skipped(
            "engine",
            "The engine is running",
            "the engine is not registered",
        ),
        _ => check(
            "engine",
            "The engine is running",
            IbusCheckState::Warn,
            "IBus has not started the engine (it starts when WayExpand is selected)",
            Some("`ibus engine wayexpand`".into()),
        ),
    });

    checks.push(field_input_check(activity.as_ref()));
    checks.push(target_insertion_check(activity.as_ref()));

    IbusChainReport {
        desktop,
        checks,
        activity,
    }
}

fn kwin_input_method_check(
    available: Option<bool>,
    helper_running: bool,
    recent_helper_segv_pid: Option<u32>,
) -> IbusCheck {
    match available {
        Some(true) if recent_helper_segv_pid.is_some() => {
            kwin_input_method_crash(recent_helper_segv_pid.unwrap(), helper_running)
        }
        Some(true) => check(
            "desktop",
            "KWin sends input to IBus",
            IbusCheckState::Pass,
            "KWin's Virtual Keyboard is IBus Wayland and running",
            None,
        ),
        Some(false) if recent_helper_segv_pid.is_some() => {
            kwin_input_method_crash(recent_helper_segv_pid.unwrap(), helper_running)
        }
        Some(false) => kwin_input_method_failure(
            "KWin reports that its configured IBus Wayland input method is not running",
        ),
        None if !helper_running && recent_helper_segv_pid.is_some() => {
            kwin_input_method_crash(recent_helper_segv_pid.unwrap(), false)
        }
        None if recent_helper_segv_pid.is_some() => {
            kwin_input_method_crash(recent_helper_segv_pid.unwrap(), true)
        }
        None if !helper_running => kwin_input_method_failure(
            "KWin is configured for IBus, but its IBus Wayland helper is not running and KWin's status interface could not be queried",
        ),
        None => {
            let mut result = check(
                "desktop",
                "KWin sends input to IBus",
                IbusCheckState::Unknown,
                "KWin is configured for IBus and the helper process exists, but KWin's live input-method state could not be queried",
                Some("open System Settings → Keyboard → Virtual Keyboard and confirm IBus Wayland is selected, then run `wayexpand ibus status --json` again".into()),
            );
            result.problem_id = Some("WX-KWIN-IBUS-001");
            result
        }
    }
}

fn kwin_unconfigured_input_method(configured: Option<&str>) -> IbusCheck {
    let detail = match configured {
        Some("") | None => {
            "KWin has no Virtual Keyboard (input method) configured, so Wayland apps never reach IBus"
        }
        Some(other) => return kwin_input_method_failure(format!(
            "KWin's Virtual Keyboard is {other}, not IBus"
        )),
    };
    kwin_input_method_failure(detail)
}

fn kwin_input_method_crash(pid: u32, helper_running: bool) -> IbusCheck {
    let mut result = check(
        "desktop",
        "KWin sends input to IBus",
        if helper_running { IbusCheckState::Warn } else { IbusCheckState::Fail },
        format!(
            "coredumpctl recorded a recent SIGSEGV for ibus-ui-gtk3 (PID {pid}); the IBus Wayland helper {}",
            if helper_running { "is running again, but has crashed recently" } else { "is not running" }
        ),
        Some(format!("inspect `coredumpctl info {pid}`, install the latest IBus package supported by your distribution, then log out of Plasma and back in. Rerun `wayexpand ibus status --json` and test a real application; if the crash recurs, report the backtrace and exact package version to your distribution or IBus maintainers")),
    );
    result.problem_id = Some("WX-KWIN-IBUS-002");
    result
}

fn kwin_input_method_failure(detail: impl Into<String>) -> IbusCheck {
    let mut result = check(
        "desktop",
        "KWin sends input to IBus",
        IbusCheckState::Fail,
        detail,
        Some("In System Settings → Keyboard → Virtual Keyboard, select IBus Wayland. Then log out of Plasma and back in, and run `wayexpand ibus status --json`. If KWin's IBus helper keeps crashing, update or repair the system IBus package and inspect `coredumpctl info ibus-ui-gtk3`.".into()),
    );
    result.problem_id = Some("WX-KWIN-IBUS-001");
    result
}

/// KWin launches this process to provide the Wayland input-method bridge.
/// An installed/selected IBus engine cannot receive native Wayland client
/// input while this bridge is absent.
fn ibus_wayland_helper_running() -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    let Some(uid) = fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
    else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|name| name.parse::<u32>().ok()) else {
            return false;
        };
        let proc_dir = PathBuf::from("/proc").join(pid.to_string());
        fs::metadata(&proc_dir).is_ok_and(|metadata| metadata.uid() == uid)
            && fs::read_to_string(proc_dir.join("comm"))
                .is_ok_and(|comm| comm.trim() == "ibus-ui-gtk3")
    })
}

#[derive(serde::Deserialize)]
struct CoredumpRecord {
    pid: u32,
    uid: u32,
    sig: u32,
    exe: String,
}

fn recent_ibus_wayland_segv() -> Option<u32> {
    let uid = unsafe { libc::geteuid() };
    let uid_filter = coredump_uid_filter(uid);
    let output = bounded_command_output(
        "coredumpctl",
        &[
            "--no-pager",
            "--json=short",
            "--since",
            "7 days ago",
            "--reverse",
            "-n",
            "20",
            "list",
            "COREDUMP_COMM=ibus-ui-gtk3",
            uid_filter.as_str(),
        ],
        Duration::from_secs(2),
    )?;
    recent_ibus_wayland_segv_from_json(&output, uid)
}

fn coredump_uid_filter(uid: u32) -> String {
    format!("COREDUMP_UID={uid}")
}

fn recent_ibus_wayland_segv_from_json(output: &str, current_uid: u32) -> Option<u32> {
    let records: Vec<CoredumpRecord> = serde_json::from_str(output).ok()?;
    records.into_iter().find_map(|record| {
        (record.uid == current_uid
            && record.sig == libc::SIGSEGV as u32
            && Path::new(&record.exe).file_name() == Some(OsStr::new("ibus-ui-gtk3")))
        .then_some(record.pid)
    })
}

/// The native KWin/IBus Wayland route must not be forced through the legacy
/// GTK/Qt IBus modules. They are useful for some X11 clients but conflict with
/// IBus's compositor-managed Wayland integration.
fn plasma_toolkit_environment_check(manager_environment: Option<&str>) -> IbusCheck {
    let Some(manager_environment) = manager_environment else {
        return check(
            "toolkit-environment",
            "Legacy GTK/Qt overrides do not conflict with IBus Wayland",
            IbusCheckState::Unknown,
            "the user activation environment could not be inspected",
            Some("run `systemctl --user unset-environment GTK_IM_MODULE QT_IM_MODULE`, remove those overrides from Plasma login files, log out and back in, then rerun `wayexpand ibus status --json`".into()),
        );
    };

    const FORBIDDEN: [&str; 2] = ["GTK_IM_MODULE", "QT_IM_MODULE"];
    let configured: Vec<_> = FORBIDDEN
        .iter()
        .filter_map(|name| {
            let actual = manager_environment
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='));
            actual
                .is_some_and(|value| !value.is_empty())
                .then_some(*name)
        })
        .collect();
    if configured.is_empty() {
        check(
            "toolkit-environment",
            "Legacy GTK/Qt overrides do not conflict with IBus Wayland",
            IbusCheckState::Pass,
            "the user activation environment does not force GTK_IM_MODULE or QT_IM_MODULE; native Wayland clients use KWin's input-method bridge",
            None,
        )
    } else {
        check(
            "toolkit-environment",
            "Legacy GTK/Qt overrides do not conflict with IBus Wayland",
            IbusCheckState::Warn,
            format!(
                "the user activation environment sets legacy Wayland input-method overrides: {}",
                configured.join(", ")
            ),
            Some("remove GTK_IM_MODULE and QT_IM_MODULE from Plasma login files, run `systemctl --user unset-environment GTK_IM_MODULE QT_IM_MODULE`, log out and back in, then rerun `wayexpand ibus status --json`".into()),
        )
    }
}

/// Check the running Plasma shell too: it can carry legacy overrides even
/// after the systemd manager environment has been corrected.
fn plasma_shell_environments() -> Option<Vec<Vec<(OsString, OsString)>>> {
    let uid = unsafe { libc::geteuid() };
    let processes = fs::read_dir("/proc").ok()?;
    let mut environments = Vec::new();
    for entry in processes.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let proc_dir = PathBuf::from("/proc").join(pid.to_string());
        if fs::metadata(&proc_dir).map(|metadata| metadata.uid()).ok() != Some(uid) {
            continue;
        }
        if !fs::read_to_string(proc_dir.join("comm")).is_ok_and(|comm| comm.trim() == "plasmashell")
        {
            continue;
        }
        environments.push(read_environment(&proc_dir.join("environ"))?);
    }
    Some(environments)
}

fn plasma_shell_environment_check(
    environments: Option<Vec<Vec<(OsString, OsString)>>>,
) -> IbusCheck {
    let Some(environments) = environments else {
        return check(
            "desktop-shell-environment",
            "Plasma shell does not force legacy IBus modules on Wayland",
            IbusCheckState::Unknown,
            "the current Plasma shell environment could not be inspected",
            Some("log out of Plasma and back in, then rerun `wayexpand ibus status --json`".into()),
        );
    };
    if environments.is_empty() {
        return check(
            "desktop-shell-environment",
            "Plasma shell does not force legacy IBus modules on Wayland",
            IbusCheckState::Unknown,
            "no running plasmashell process was found to inspect its input-method environment",
            Some("log out of Plasma and back in, then rerun `wayexpand ibus status --json`".into()),
        );
    }

    const FORBIDDEN: [&str; 2] = ["GTK_IM_MODULE", "QT_IM_MODULE"];
    let configured = FORBIDDEN
        .iter()
        .filter_map(|name| {
            environments
                .iter()
                .any(|environment| {
                    environment
                        .iter()
                        .any(|(key, value)| key == OsStr::new(name) && !value.is_empty())
                })
                .then_some(*name)
        })
        .collect::<Vec<_>>();
    if configured.is_empty() {
        check(
            "desktop-shell-environment",
            "Plasma shell does not force legacy IBus modules on Wayland",
            IbusCheckState::Pass,
            "the inspected Plasma shell process(es) do not force GTK_IM_MODULE or QT_IM_MODULE",
            None,
        )
    } else {
        check(
            "desktop-shell-environment",
            "Plasma shell does not force legacy IBus modules on Wayland",
            IbusCheckState::Warn,
            format!(
                "the running Plasma shell inherited legacy Wayland input-method overrides: {}",
                configured.join(", ")
            ),
            Some("remove GTK_IM_MODULE and QT_IM_MODULE from Plasma login files, log out of Plasma and back in, then rerun `wayexpand ibus status --json`".into()),
        )
    }
}

fn gnome_session_check() -> IbusCheck {
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let session_type = env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland_display = env::var_os("WAYLAND_DISPLAY").is_some();
    gnome_session_check_from(
        &desktop,
        &session_type,
        wayland_display,
        gnome_shell_name_has_owner(),
        env::var_os("DISPLAY").is_some(),
    )
}

fn gnome_session_check_from(
    desktop: &str,
    session_type: &str,
    wayland_display: bool,
    shell_owner: Option<bool>,
    x11_display: bool,
) -> IbusCheck {
    let is_gnome = desktop
        .to_ascii_lowercase()
        .split(':')
        .any(|part| part == "gnome");
    if is_gnome
        && session_type.eq_ignore_ascii_case("wayland")
        && wayland_display
        && shell_owner == Some(true)
    {
        check(
            "gnome-session",
            "GNOME Wayland session",
            IbusCheckState::Pass,
            "GNOME desktop and Wayland environment are present, and org.gnome.Shell owns its session-bus name",
            None,
        )
    } else if is_gnome
        && session_type.eq_ignore_ascii_case("wayland")
        && wayland_display
        && shell_owner == Some(false)
    {
        check(
            "gnome-session",
            "GNOME Wayland session",
            IbusCheckState::Fail,
            "GNOME Wayland environment is present, but org.gnome.Shell has no session-bus owner",
            Some("open a GNOME desktop session, then rerun `wayexpand ibus status`".into()),
        )
    } else if is_gnome
        && (session_type.eq_ignore_ascii_case("x11") || (!wayland_display && x11_display))
    {
        check(
            "gnome-session",
            "GNOME Wayland session",
            IbusCheckState::Warn,
            "GNOME is active, but the session is not identified as Wayland",
            None,
        )
    } else if is_gnome && session_type.eq_ignore_ascii_case("wayland") && wayland_display {
        check(
            "gnome-session",
            "GNOME Wayland session",
            IbusCheckState::Unknown,
            "the session environment looks like GNOME Wayland, but the GNOME Shell session-bus owner could not be verified",
            None,
        )
    } else {
        check(
            "gnome-session",
            "GNOME Wayland session",
            IbusCheckState::Unknown,
            "environment evidence is insufficient to confirm a GNOME Wayland session",
            None,
        )
    }
}

/// Query the live session bus without allowing a stalled bus to block doctor.
fn gnome_shell_name_has_owner() -> Option<bool> {
    session_bus_name_has_owner("org.gnome.Shell")
}

fn session_bus_name_has_owner(name: &str) -> Option<bool> {
    let response = bounded_command_output(
        "busctl",
        &[
            "--user",
            "--timeout=1s",
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            "s",
            name,
        ],
        Duration::from_millis(1500),
    )?;
    match response.trim() {
        "b true" => Some(true),
        "b false" => Some(false),
        _ => None,
    }
}

fn ibus_daemon_check(daemon: Option<&IbusDaemonProcess>, desktop: IbusDesktop) -> IbusCheck {
    let responsive = daemon.and_then(|_| ibus_private_bus_responsive());
    ibus_daemon_check_from(daemon, responsive, desktop)
}

/// IBus owns a private D-Bus address. Checking `org.freedesktop.IBus` on the
/// desktop's general session bus is not reliable: a healthy IBus instance can
/// answer `ibus` clients on its own bus without owning that name on the
/// session bus. Probe the address reported by `ibus` itself and call Peer.Ping.
fn ibus_private_bus_responsive() -> Option<bool> {
    let address = bounded_command_output("ibus", &["address"], Duration::from_millis(500))?;
    let address = address.trim();
    if !address.starts_with("unix:") || address.len() > 4096 {
        return Some(false);
    }
    let response = bounded_command_output(
        "gdbus",
        &[
            "call",
            "--address",
            address,
            "--dest",
            "org.freedesktop.IBus",
            "--object-path",
            "/org/freedesktop/IBus",
            "--method",
            "org.freedesktop.DBus.Peer.Ping",
        ],
        Duration::from_millis(750),
    );
    match response {
        Some(response) => Some(response.trim() == "()"),
        // `gdbus` is optional even when IBus is installed. The IBus CLI uses
        // the same private bus, so a successful engine listing is a valid
        // fallback liveness probe without adding a runtime dependency.
        None => bounded_command_output(
            "ibus",
            &["list-engine", "--name-only"],
            Duration::from_millis(750),
        )
        .map(|engines| !engines.is_empty()),
    }
}

fn ibus_daemon_check_from(
    daemon: Option<&IbusDaemonProcess>,
    bus_owner: Option<bool>,
    desktop: IbusDesktop,
) -> IbusCheck {
    let Some(process) = daemon else {
        return check(
            "daemon",
            "IBus daemon responds on its private bus",
            IbusCheckState::Fail,
            "no ibus-daemon process is running for this user",
            Some(start_ibus_fix(desktop)),
        );
    };
    match bus_owner {
        Some(true) => check(
            "daemon",
            "IBus daemon responds on its private bus",
            IbusCheckState::Pass,
            format!(
                "ibus-daemon pid {} answered a D-Bus ping at the address reported by `ibus`",
                process.pid
            ),
            None,
        ),
        Some(false) => check(
            "daemon",
            "IBus daemon responds on its private bus",
            IbusCheckState::Fail,
            format!(
                "ibus-daemon pid {} is present, but the private IBus D-Bus did not answer",
                process.pid
            ),
            Some(start_ibus_fix(desktop)),
        ),
        None => check(
            "daemon",
            "IBus daemon responds on its private bus",
            IbusCheckState::Unknown,
            "the IBus process is present, but its private D-Bus address could not be checked",
            None,
        ),
    }
}

fn gnome_input_source_checks() -> (IbusCheck, IbusCheck) {
    let sources = gnome_setting("sources");
    let active = wayexpand_backend_gnome_window::active_input_source().ok();
    gnome_source_checks(
        sources.as_deref(),
        active
            .as_ref()
            .map(|(kind, id)| (kind.as_str(), id.as_str())),
    )
}

fn gnome_source_checks(
    sources_value: Option<&str>,
    active_source: Option<(&str, &str)>,
) -> (IbusCheck, IbusCheck) {
    let Some(raw_sources) = sources_value else {
        return (
            check(
                "gnome-source-registered",
                "WayExpand registered in GNOME",
                IbusCheckState::Unknown,
                "GNOME input-source settings could not be read",
                Some("wayexpand repair --gnome-input-source".into()),
            ),
            check(
                "gnome-source-active",
                "WayExpand selected in GNOME",
                IbusCheckState::Unknown,
                "active GNOME input source could not be verified",
                None,
            ),
        );
    };
    let sources = match crate::gnome_input_sources::parse_sources(raw_sources) {
        Ok(sources) => sources,
        Err(error) => {
            return (
                check(
                    "gnome-source-registered",
                    "WayExpand registered in GNOME",
                    IbusCheckState::Fail,
                    format!("GNOME returned an invalid input-source tuple array: {error}"),
                    Some("wayexpand repair --gnome-input-source".into()),
                ),
                check(
                    "gnome-source-active",
                    "WayExpand selected in GNOME",
                    IbusCheckState::Unknown,
                    "registration is invalid, so active selection cannot be verified",
                    None,
                ),
            );
        }
    };
    let Some(_) = crate::gnome_input_sources::wayexpand_index(&sources) else {
        return (
            check(
                "gnome-source-registered",
                "WayExpand registered in GNOME",
                IbusCheckState::Fail,
                "GNOME's source list does not contain ('ibus', 'wayexpand')",
                Some("wayexpand repair --gnome-input-source".into()),
            ),
            check(
                "gnome-source-active",
                "WayExpand selected in GNOME",
                IbusCheckState::Unknown,
                "WayExpand is not registered, so active selection cannot be verified",
                None,
            ),
        );
    };
    let registered = check(
        "gnome-source-registered",
        "WayExpand registered in GNOME",
        IbusCheckState::Pass,
        "GNOME's typed source list contains ('ibus', 'wayexpand')",
        None,
    );
    let active = match active_source {
        Some(("ibus", "wayexpand")) => check(
            "gnome-source-active",
            "WayExpand selected in GNOME",
            IbusCheckState::Pass,
            "GNOME Shell's live InputSourceManager reports WayExpand as the active source",
            None,
        ),
        Some(_) => check(
            "gnome-source-active",
            "WayExpand selected in GNOME",
            IbusCheckState::Warn,
            "GNOME Shell reports a different active input source",
            Some("select WayExpand from GNOME's input-source menu or run `wayexpand repair --gnome-input-source`".into()),
        ),
        None => check(
            "gnome-source-active",
            "WayExpand selected in GNOME",
            IbusCheckState::Unknown,
            "GNOME Shell's live input-source bridge is unavailable; install or enable the WayExpand GNOME Shell extension, then log out and back in if it was just updated",
            Some("run `wayexpand setup` to install or enable the GNOME Shell integration".into()),
        ),
    };
    (registered, active)
}

fn gnome_setting(key: &str) -> Option<String> {
    bounded_command_output(
        "gsettings",
        &["get", "org.gnome.desktop.input-sources", key],
        Duration::from_secs(1),
    )
}

fn bounded_command_output(program: &str, arguments: &[&str], timeout: Duration) -> Option<String> {
    const MAX_OUTPUT: u64 = 64 * 1024;
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
        let mut output = Vec::new();
        let _ = (&mut stdout).take(MAX_OUTPUT + 1).read_to_end(&mut output);
        output
    });
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match supervisor.has_exited() {
            Ok(true) => break,
            Ok(false) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10))
            }
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
    if !status.success() || output.len() as u64 > MAX_OUTPUT {
        return None;
    }
    String::from_utf8(output)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn field_input_check(activity: Option<&IbusStatusReading>) -> IbusCheck {
    let safe_field_seen = activity
        .filter(|reading| reading.live)
        .is_some_and(|reading| {
            reading.status.recent_fields.iter().any(|field| {
                field.keys > 0
                    && field.content_type == crate::telemetry::IbusContentTypeState::Normal
                    && field.surrounding_reports > 0
            })
        });
    if safe_field_seen {
        check(
            "field-input",
            "Normal-field input observed",
            IbusCheckState::Pass,
            "at least one recent IBus field sent key events and surrounding text; contents and application identity are not recorded, so this does not verify input from a particular browser",
            None,
        )
    } else {
        check(
            "field-input",
            "Normal-field input observed",
            IbusCheckState::Unknown,
            "no recent normal field has provided verifiable input telemetry; the check cannot identify an application",
            Some("focus the browser field, run `wayexpand ibus watch --seconds 30`, and type harmless text; increased counters confirm activity only, not replacement".into()),
        )
    }
}

fn target_insertion_check(activity: Option<&IbusStatusReading>) -> IbusCheck {
    let Some(reading) = activity.filter(|reading| reading.live) else {
        return check(
            "target-insertion",
            "Target application insertion verified",
            IbusCheckState::Unknown,
            "no live IBus engine outcome is available",
            Some(
                "use `wayexpand debug --test ';;hello'` in a real application's text field".into(),
            ),
        );
    };
    if reading
        .status
        .recent_fields
        .iter()
        .any(|field| field.output_failures > 0)
    {
        return check(
            "target-insertion",
            "Target application insertion verified",
            IbusCheckState::Fail,
            "the IBus engine recorded an output failure; target text may be partial",
            Some("inspect the target field and do not retry blindly; run `wayexpand debug --test ';;hello'` after recovery".into()),
        );
    }
    if reading.status.totals.expansions > 0 {
        check(
            "target-insertion",
            "Target application insertion verified",
            IbusCheckState::Warn,
            "IBus emitted replacement actions, but the client does not acknowledge the resulting target text",
            Some("visually confirm the text or run `wayexpand debug --test ';;hello'` in this application".into()),
        )
    } else {
        check(
            "target-insertion",
            "Target application insertion verified",
            IbusCheckState::Unknown,
            "no replacement has been emitted in the current engine session",
            Some(
                "use `wayexpand debug --test ';;hello'` in a real application's text field".into(),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kwin_input_method_parser_accepts_kconfig_environment_marker() {
        let value = "/usr/share/applications/org.freedesktop.IBus.Panel.Wayland.Gtk3.desktop";
        assert_eq!(
            parse_kwin_input_method(&format!("[Wayland]\nInputMethod[$e]={value}\n")),
            Some(value.to_owned())
        );
        assert_eq!(
            parse_kwin_input_method(&format!("[Wayland]\nInputMethod={value}\n")),
            Some(value.to_owned())
        );
        assert_eq!(
            parse_kwin_input_method(&format!(
                "[Other]\nInputMethod[$e]={value}\n[Wayland]\nVirtualKeyboardEnabled=true\n"
            )),
            None
        );
    }

    #[test]
    fn denied_ibus_replacement_has_the_stable_policy_finding() {
        let denied = check(
            "consent",
            "Replacements are allowed",
            IbusCheckState::Fail,
            "replacement is disabled by policy",
            Some("review and authorize replacement".into()),
        );
        assert_eq!(denied.problem_id, Some("WX-POLICY-001"));
    }

    #[test]
    fn generic_ibus_registration_and_selection_failures_have_stable_ids() {
        let registration = check(
            "registered",
            "Engine registered",
            IbusCheckState::Fail,
            "missing",
            None,
        );
        let selection = check(
            "selected",
            "Engine selected",
            IbusCheckState::Fail,
            "not selected",
            None,
        );
        assert_eq!(registration.problem_id, Some("WX-IBUS-001"));
        assert_eq!(selection.problem_id, Some("WX-IBUS-002"));
    }

    #[test]
    fn generic_ibus_runtime_checks_have_stable_ids_for_each_outcome() {
        let no_input = check(
            "field-input",
            "Normal-field input observed",
            IbusCheckState::Unknown,
            "no safe-field activity observed",
            None,
        );
        assert_eq!(no_input.problem_id, Some("WX-IBUS-003"));

        for (state, expected) in [
            (IbusCheckState::Fail, "WX-IBUS-005"),
            (IbusCheckState::Warn, "WX-IBUS-006"),
            (IbusCheckState::Unknown, "WX-IBUS-007"),
        ] {
            let insertion = check(
                "target-insertion",
                "Target application insertion verified",
                state,
                "target text was not independently verified",
                None,
            );
            assert_eq!(insertion.problem_id, Some(expected));
        }

        for state in [IbusCheckState::Pass, IbusCheckState::Skip] {
            assert_eq!(
                check("target-insertion", "", state, "", None).problem_id,
                None
            );
        }
    }

    #[test]
    fn manifest_parsing_reads_the_component_name_not_the_engine_name() {
        let directory =
            std::env::temp_dir().join(format!("wayexpand-chain-manifest-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("wayexpand-ibus.xml"),
            include_str!("../../../desktop/wayexpand-ibus.xml"),
        )
        .unwrap();
        let manifest = IbusManifest::find_in(std::slice::from_ref(&directory)).unwrap();
        assert_eq!(manifest.name.as_deref(), Some(BUS_NAME));
        assert_eq!(manifest.exec.as_deref(), Some("wayexpand-ibus"));
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn exec_resolution_honors_absolute_paths_and_the_daemon_path() {
        assert_eq!(
            resolve_exec("/bin/sh -c x", None),
            Some(PathBuf::from("/bin/sh"))
        );
        assert_eq!(resolve_exec("/nonexistent/wayexpand-ibus", None), None);
        assert_eq!(
            resolve_exec("sh", Some(OsStr::new("/nonexistent:/bin"))),
            Some(PathBuf::from("/bin/sh"))
        );
        assert_eq!(resolve_exec("sh", Some(OsStr::new("/nonexistent"))), None);
    }

    #[test]
    fn shared_ibus_manifest_cannot_bind_the_engine_to_one_users_home() {
        let homes = vec![PathBuf::from("/home/alice"), PathBuf::from("/home/bob")];
        let shared_manifest = Path::new("/usr/share/ibus/component/wayexpand.xml");
        let alice_engine = Path::new("/home/alice/.local/bin/wayexpand-ibus");
        let bob_engine = Path::new("/home/bob/.local/bin/wayexpand-ibus");
        let shared_engine = Path::new("/usr/local/libexec/wayexpand/wayexpand-ibus");
        let per_user_manifest = Path::new("/home/alice/.local/share/ibus/component/wayexpand.xml");

        // A different account must diagnose Alice's path too.
        assert!(shared_manifest_uses_user_home(
            shared_manifest,
            alice_engine,
            &homes
        ));
        assert!(!shared_manifest_uses_user_home(
            shared_manifest,
            shared_engine,
            &homes
        ));
        assert!(!shared_manifest_uses_user_home(
            per_user_manifest,
            alice_engine,
            &homes
        ));
        assert!(shared_manifest_uses_user_home(
            shared_manifest,
            bob_engine,
            &homes
        ));
    }

    #[test]
    fn shared_ibus_manifest_scope_warning_has_stable_problem_id() {
        let warning = check(
            "manifest-scope",
            "Shared IBus registration is multi-user safe",
            IbusCheckState::Warn,
            "shared manifest points to one user's executable",
            Some("repair the system integration".into()),
        );
        assert_eq!(warning.problem_id, Some("WX-IBUS-MULTIUSER-001"));
    }

    #[test]
    fn environment_parsing_splits_on_the_first_equals_sign() {
        let path = std::env::temp_dir().join(format!("wayexpand-chain-env-{}", std::process::id()));
        fs::write(&path, b"IBUS_COMPONENT_PATH=/a:/b\0WEIRD=x=y\0NOEQUALS\0").unwrap();
        let environment = read_environment(&path).unwrap();
        fs::remove_file(&path).unwrap();
        let process = IbusDaemonProcess {
            pid: 1,
            environment: Some(environment),
        };
        assert_eq!(process.var("WEIRD"), Some(OsStr::new("x=y")));
        assert_eq!(
            process.component_directories(),
            (vec![PathBuf::from("/a"), PathBuf::from("/b")], true)
        );
    }

    #[test]
    fn readiness_reports_each_condition_separately() {
        let report = IbusChainReport {
            desktop: IbusDesktop::Unknown,
            checks: vec![
                check("component", "", IbusCheckState::Pass, "", None),
                check("manifest", "", IbusCheckState::Pass, "", None),
                check("registered", "", IbusCheckState::Pass, "", None),
                check("daemon", "", IbusCheckState::Pass, "", None),
                check("selected", "", IbusCheckState::Warn, "", None),
            ],
            activity: None,
        };
        let readiness = report.readiness();
        // Installed and registered says nothing about selection, a running
        // engine, received text, or expansion.
        assert!(readiness.component_installed && readiness.registered && readiness.ibus_running);
        assert!(!readiness.selected);
        assert!(!readiness.engine_running && !readiness.text_received && !readiness.expanded);
        assert_eq!(readiness.stages().len(), 7);
    }

    #[test]
    fn configured_gnome_route_needs_field_input_and_target_confirmation() {
        let pass = |id| check(id, "", IbusCheckState::Pass, "", None);
        let report = IbusChainReport {
            desktop: IbusDesktop::Gnome,
            checks: vec![
                pass("component"),
                pass("manifest"),
                pass("registered"),
                pass("consent"),
                pass("gnome-session"),
                pass("gnome-source-registered"),
                pass("daemon"),
                pass("selected"),
                pass("engine"),
                pass("gnome-source-active"),
                pass("field-input"),
                check(
                    "target-insertion",
                    "",
                    IbusCheckState::Unknown,
                    "target effect is not observable",
                    None,
                ),
            ],
            activity: None,
        };
        assert_eq!(report.configuration_readiness(), IbusCheckState::Pass);
        assert_eq!(report.operational_readiness(), IbusCheckState::Unknown);
        assert!(!report.ready());
    }

    #[test]
    fn unknown_is_not_aggregated_as_pass() {
        let report = IbusChainReport {
            desktop: IbusDesktop::KdeWayland,
            checks: vec![check(
                "field-input",
                "",
                IbusCheckState::Unknown,
                "not observed",
                None,
            )],
            activity: None,
        };
        assert_eq!(report.operational_readiness(), IbusCheckState::Unknown);
        assert!(!report.ready());
    }

    #[test]
    fn gnome_wayland_session_requires_shell_bus_evidence() {
        let detected = gnome_session_check_from("GNOME", "wayland", true, Some(true), false);
        assert_eq!(detected.state, IbusCheckState::Pass);
        let missing_shell = gnome_session_check_from("GNOME", "wayland", true, Some(false), false);
        assert_eq!(missing_shell.state, IbusCheckState::Fail);
        let unavailable_shell_probe =
            gnome_session_check_from("GNOME", "wayland", true, None, false);
        assert_eq!(unavailable_shell_probe.state, IbusCheckState::Unknown);
        let x11 = gnome_session_check_from("GNOME", "x11", false, Some(true), true);
        assert_eq!(x11.state, IbusCheckState::Warn);
    }

    #[test]
    fn daemon_process_presence_without_bus_evidence_is_not_a_pass() {
        let process = IbusDaemonProcess {
            pid: 123,
            environment: None,
        };
        let unavailable = ibus_daemon_check_from(Some(&process), None, IbusDesktop::Gnome);
        assert_eq!(unavailable.state, IbusCheckState::Unknown);
        assert!(unavailable.title.contains("private bus"));
        let unresponsive = ibus_daemon_check_from(Some(&process), Some(false), IbusDesktop::Gnome);
        assert_eq!(unresponsive.state, IbusCheckState::Fail);
        assert_eq!(unresponsive.problem_id, Some("WX-IBUS-004"));
        assert!(unresponsive.detail.contains("private IBus D-Bus"));
        let responsive = ibus_daemon_check_from(Some(&process), Some(true), IbusDesktop::Gnome);
        assert_eq!(responsive.state, IbusCheckState::Pass);
        assert!(responsive.detail.contains("address reported by `ibus`"));
    }

    #[test]
    fn gnome_registration_and_activation_are_separate_findings() {
        let (registered, active) = gnome_source_checks(
            Some("[('xkb', 'us'), ('ibus', 'wayexpand')]"),
            Some(("ibus", "wayexpand")),
        );
        assert_eq!(registered.state, IbusCheckState::Pass);
        assert_eq!(active.state, IbusCheckState::Pass);

        let (registered, active) =
            gnome_source_checks(Some("[('xkb', 'us')]"), Some(("xkb", "us")));
        assert_eq!(registered.state, IbusCheckState::Fail);
        assert_eq!(
            registered.fix.as_deref(),
            Some("wayexpand repair --gnome-input-source")
        );
        assert_eq!(registered.problem_id, Some("WX-GNOME-IM-001"));
        assert_eq!(active.state, IbusCheckState::Unknown);

        let (registered, active) = gnome_source_checks(
            Some("[('xkb', 'us'), ('ibus', 'wayexpand')]"),
            Some(("xkb", "us")),
        );
        assert_eq!(registered.state, IbusCheckState::Pass);
        assert_eq!(active.state, IbusCheckState::Warn);
    }

    #[test]
    fn deprecated_current_value_cannot_confirm_gnome_activation() {
        // `current` may read back the requested index while GNOME continues
        // using another source. The diagnostic intentionally accepts no
        // setting value as activation evidence.
        let (_, active) = gnome_source_checks(Some("[('xkb', 'us'), ('ibus', 'wayexpand')]"), None);
        assert_eq!(active.state, IbusCheckState::Unknown);
        assert!(active
            .detail
            .contains("live input-source bridge is unavailable"));

        let (_, active) = gnome_source_checks(
            Some("[('xkb', 'us'), ('ibus', 'wayexpand')]"),
            Some(("xkb", "de")),
        );
        assert_eq!(active.state, IbusCheckState::Warn);
    }

    #[test]
    fn field_input_pass_is_aggregate_and_never_claims_app_identity() {
        let telemetry = crate::telemetry::IbusTelemetry::new();
        let field = telemetry.focus_in(None);
        telemetry.capabilities(Some(field), true);
        telemetry.content_type(Some(field), false);
        telemetry.surrounding_text(Some(field), true);
        telemetry.key(Some(field), true);
        let reading = IbusStatusReading {
            status: telemetry.snapshot(),
            live: true,
        };
        assert_eq!(
            field_input_check(Some(&reading)).state,
            IbusCheckState::Pass
        );
        let observed = field_input_check(Some(&reading));
        assert!(observed
            .detail
            .contains("application identity are not recorded"));
        assert!(observed.detail.contains("particular browser"));

        let missing = field_input_check(None);
        assert_eq!(missing.state, IbusCheckState::Unknown);
        assert!(missing.detail.contains("cannot identify an application"));
        let fix = missing
            .fix
            .expect("missing field activity has safe guidance");
        assert!(fix.contains("wayexpand ibus watch --seconds 30"));
        assert!(fix.contains("activity only, not replacement"));
    }

    #[test]
    fn actual_surrounding_reports_prove_field_input_even_if_capability_flag_is_absent() {
        let telemetry = crate::telemetry::IbusTelemetry::new();
        let field = telemetry.focus_in(None);
        telemetry.content_type(Some(field), false);
        telemetry.surrounding_text(Some(field), true);
        telemetry.key(Some(field), true);
        let reading = IbusStatusReading {
            status: telemetry.snapshot(),
            live: true,
        };
        assert_eq!(
            field_input_check(Some(&reading)).state,
            IbusCheckState::Pass
        );
    }

    #[test]
    fn kde_wayland_rejects_legacy_toolkit_overrides() {
        let clean = plasma_toolkit_environment_check(Some("XMODIFIERS=@im=ibus\n"));
        assert_eq!(clean.state, IbusCheckState::Pass);
        assert!(clean.detail.contains("native Wayland clients"));

        let legacy = plasma_toolkit_environment_check(Some(
            "GTK_IM_MODULE=ibus\nQT_IM_MODULE=ibus\nXMODIFIERS=@im=ibus\n",
        ));
        assert_eq!(legacy.state, IbusCheckState::Warn);
        assert_eq!(legacy.problem_id, Some("WX-IBUS-TOOLKIT-001"));
        assert!(legacy.detail.contains("GTK_IM_MODULE"));
        assert!(legacy.detail.contains("QT_IM_MODULE"));
        assert!(!legacy.detail.contains("XMODIFIERS"));

        let unavailable = plasma_toolkit_environment_check(None);
        assert_eq!(unavailable.state, IbusCheckState::Unknown);
        assert_eq!(unavailable.problem_id, Some("WX-IBUS-TOOLKIT-001"));

        let report = IbusChainReport {
            desktop: IbusDesktop::KdeWayland,
            checks: vec![
                check("daemon", "", IbusCheckState::Pass, "", None),
                check("selected", "", IbusCheckState::Pass, "", None),
                check("engine", "", IbusCheckState::Pass, "", None),
                check("desktop", "", IbusCheckState::Pass, "", None),
                check("field-input", "", IbusCheckState::Pass, "", None),
                check("target-insertion", "", IbusCheckState::Pass, "", None),
                check(
                    "desktop-shell-environment",
                    "",
                    IbusCheckState::Pass,
                    "",
                    None,
                ),
                clean,
            ],
            activity: None,
        };
        assert_eq!(report.operational_readiness(), IbusCheckState::Pass);
        assert!(report.ready());

        let mut legacy_toolkit = report;
        legacy_toolkit.checks.pop();
        legacy_toolkit.checks.push(legacy);
        assert_eq!(legacy_toolkit.operational_readiness(), IbusCheckState::Warn);
        assert!(!legacy_toolkit.ready());
    }

    #[test]
    fn plasma_shell_environment_check_rejects_legacy_wayland_overrides() {
        let clean = vec![vec![(
            OsString::from("XMODIFIERS"),
            OsString::from("@im=ibus"),
        )]];
        assert_eq!(
            plasma_shell_environment_check(Some(clean)).state,
            IbusCheckState::Pass
        );
        let legacy = plasma_shell_environment_check(Some(vec![vec![(
            OsString::from("GTK_IM_MODULE"),
            OsString::from("ibus"),
        )]]));
        assert_eq!(legacy.state, IbusCheckState::Warn);
        assert_eq!(legacy.problem_id, Some("WX-IBUS-TOOLKIT-001"));
        assert!(legacy.fix.as_deref().unwrap().contains("log out of Plasma"));
        assert!(legacy.detail.contains("GTK_IM_MODULE"));

        let unavailable = plasma_shell_environment_check(None);
        assert_eq!(unavailable.state, IbusCheckState::Unknown);
    }

    #[test]
    fn missing_kwin_ibus_bridge_is_a_stable_failure() {
        let check = kwin_input_method_check(None, false, None);
        assert_eq!(check.state, IbusCheckState::Fail);
        assert_eq!(check.problem_id, Some("WX-KWIN-IBUS-001"));
        assert!(check.detail.contains("helper is not running"));
        assert!(check.fix.as_deref().unwrap().contains("log out of Plasma"));
    }

    #[test]
    fn missing_kwin_virtual_keyboard_has_a_stable_repair_finding() {
        for configured in [None, Some("")] {
            let check = kwin_unconfigured_input_method(configured);
            assert_eq!(check.state, IbusCheckState::Fail);
            assert_eq!(check.problem_id, Some("WX-KWIN-IBUS-001"));
            assert!(check.detail.contains("no Virtual Keyboard"));
            assert!(check.detail.contains("Wayland apps never reach IBus"));
            assert!(check
                .fix
                .as_deref()
                .unwrap()
                .contains("select IBus Wayland"));
            assert!(check.fix.as_deref().unwrap().contains("log out of Plasma"));
        }

        let other = kwin_unconfigured_input_method(Some("Fcitx 5"));
        assert_eq!(other.state, IbusCheckState::Fail);
        assert_eq!(other.problem_id, Some("WX-KWIN-IBUS-001"));
        assert!(other.detail.contains("Fcitx 5, not IBus"));
    }

    #[test]
    fn inaccessible_kwin_status_with_helper_present_stays_unknown() {
        let check = kwin_input_method_check(None, true, None);
        assert_eq!(check.state, IbusCheckState::Unknown);
        assert_eq!(check.problem_id, Some("WX-KWIN-IBUS-001"));
    }

    #[test]
    fn kwin_active_bridge_is_required_for_a_pass() {
        let active = kwin_input_method_check(Some(true), false, None);
        let inactive = kwin_input_method_check(Some(false), true, None);
        assert_eq!(active.state, IbusCheckState::Pass);
        assert_eq!(inactive.state, IbusCheckState::Fail);
        assert_eq!(inactive.problem_id, Some("WX-KWIN-IBUS-001"));
    }

    #[test]
    fn kwin_helper_segfault_has_specific_repair_finding() {
        let check = kwin_input_method_check(None, false, Some(1234));
        assert_eq!(check.state, IbusCheckState::Fail);
        assert_eq!(check.problem_id, Some("WX-KWIN-IBUS-002"));
        assert!(check.detail.contains("recent SIGSEGV"));
        assert!(check
            .fix
            .as_deref()
            .unwrap()
            .contains("coredumpctl info 1234"));
        let fix = check.fix.as_deref().unwrap();
        assert!(fix.contains("latest IBus package supported by your distribution"));
        assert!(fix.contains("report the backtrace and exact package version"));
        assert!(!fix.contains("5bbe88a"));
        assert!(!fix.contains("surrounding-text crash"));
    }

    #[test]
    fn recovered_kwin_helper_does_not_hide_a_recent_segfault() {
        let check = kwin_input_method_check(Some(true), true, Some(1234));
        assert_eq!(check.state, IbusCheckState::Warn);
        assert_eq!(check.problem_id, Some("WX-KWIN-IBUS-002"));
        assert!(check
            .detail
            .contains("is running again, but has crashed recently"));
        assert!(check
            .fix
            .as_deref()
            .unwrap()
            .contains("coredumpctl info 1234"));
    }

    #[test]
    fn recent_ibus_segv_parser_ignores_other_signals_users_and_programs() {
        let records = r#"[
            {"pid":1,"uid":1000,"sig":6,"exe":"/usr/libexec/ibus-ui-gtk3"},
            {"pid":2,"uid":1001,"sig":11,"exe":"/usr/libexec/ibus-ui-gtk3"},
            {"pid":3,"uid":1000,"sig":11,"exe":"/usr/bin/other"},
            {"pid":4,"uid":1000,"sig":11,"exe":"/usr/libexec/ibus-ui-gtk3"}
        ]"#;
        assert_eq!(recent_ibus_wayland_segv_from_json(records, 1000), Some(4));
        assert_eq!(recent_ibus_wayland_segv_from_json("[]", 1000), None);
        assert_eq!(recent_ibus_wayland_segv_from_json("not json", 1000), None);
    }

    #[test]
    fn coredump_query_limits_results_to_the_current_user_before_the_result_cap() {
        assert_eq!(coredump_uid_filter(1000), "COREDUMP_UID=1000");
    }

    #[test]
    fn emitted_ibus_action_is_not_mistaken_for_target_insertion_success() {
        let telemetry = crate::telemetry::IbusTelemetry::new();
        let field = telemetry.focus_in(None);
        telemetry.expansion(Some(field));
        let reading = IbusStatusReading {
            status: telemetry.snapshot(),
            live: true,
        };
        assert_eq!(
            target_insertion_check(Some(&reading)).state,
            IbusCheckState::Warn
        );

        telemetry.output_failure(Some(field));
        let failed = IbusStatusReading {
            status: telemetry.snapshot(),
            live: true,
        };
        assert_eq!(
            target_insertion_check(Some(&failed)).state,
            IbusCheckState::Fail
        );
    }

    #[test]
    fn report_readiness_follows_failures_only() {
        let report = IbusChainReport {
            desktop: IbusDesktop::Unknown,
            checks: vec![
                check("a", "a", IbusCheckState::Pass, "", None),
                check("b", "b", IbusCheckState::Warn, "", None),
            ],
            activity: None,
        };
        assert!(!report.ready());
        assert_eq!(report.operational_readiness(), IbusCheckState::Unknown);
        let mut broken = report.clone();
        broken.checks.push(check(
            "c",
            "c",
            IbusCheckState::Fail,
            "",
            Some("fix".into()),
        ));
        assert!(!broken.ready());
        assert_eq!(broken.first_failure().unwrap().id, "c");
    }
}
