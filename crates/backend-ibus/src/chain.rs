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
    os::unix::{ffi::OsStrExt, fs::MetadataExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_ENVIRON_BYTES: u64 = 256 * 1024;
/// The KWin Virtual Keyboard entry that launches IBus with Wayland support.
pub const KWIN_IBUS_INPUT_METHOD: &str =
    "/usr/share/applications/org.freedesktop.IBus.Panel.Wayland.Gtk3.desktop";

/// Outcome of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IbusCheckState {
    Pass,
    /// Works, but needs attention (for example not selected yet).
    Warn,
    /// This link is broken; IBus expansion cannot work until it is fixed.
    Fail,
    /// Not applicable here, or not checked because an earlier link failed.
    Skip,
}

impl IbusCheckState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Skip => "skip",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
            Self::Skip => "·",
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
    /// A field sent keys and reported its text, so triggers can be verified.
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
            component_installed: passed("component") && passed("manifest"),
            registered: passed("registered"),
            ibus_running: passed("daemon"),
            selected: passed("selected"),
            engine_running: live.is_some(),
            text_received: live.is_some_and(|reading| {
                reading.status.recent_fields.iter().any(|field| {
                    field.keys > 0
                        && field.content_type == crate::telemetry::IbusContentTypeState::Normal
                        && field.surrounding_nonempty
                })
            }),
            expanded: live.is_some_and(|reading| reading.status.totals.expansions > 0),
        }
    }

    /// True when no link is broken.
    pub fn ready(&self) -> bool {
        self.checks
            .iter()
            .all(|check| check.state != IbusCheckState::Fail)
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

/// Read KWin's configured input method (`[Wayland] InputMethod`).
pub fn kwin_input_method() -> Option<String> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let text = fs::read_to_string(config_home.join("kwinrc")).ok()?;
    let mut in_wayland = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_wayland = line == "[Wayland]";
        } else if in_wayland {
            if let Some(value) = line.strip_prefix("InputMethod=") {
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
    checks.push(match &daemon {
        Some(process) => check(
            "daemon",
            "IBus is running",
            IbusCheckState::Pass,
            format!("ibus-daemon pid {}", process.pid),
            None,
        ),
        None => check(
            "daemon",
            "IBus is running",
            IbusCheckState::Fail,
            "no ibus-daemon is running for this user",
            Some(start_ibus_fix(desktop)),
        ),
    });

    // 3. Desktop routing: who sends keys to IBus.
    checks.push(match desktop {
        IbusDesktop::KdeWayland => {
            let configured = kwin_input_method();
            let ibus_configured = configured
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains("ibus"));
            if !ibus_configured {
                check(
                    "desktop",
                    "KWin sends input to IBus",
                    IbusCheckState::Fail,
                    match configured.as_deref() {
                        Some("") | None => "KWin has no Virtual Keyboard (input method) configured, so Wayland apps never reach IBus".to_owned(),
                        Some(other) => format!("KWin's Virtual Keyboard is {other}, not IBus"),
                    },
                    Some(start_ibus_fix(desktop)),
                )
            } else {
                match kwin_input_method_available() {
                    Some(true) => check("desktop", "KWin sends input to IBus", IbusCheckState::Pass, "KWin's Virtual Keyboard is IBus Wayland and running", None),
                    Some(false) => check("desktop", "KWin sends input to IBus", IbusCheckState::Fail, "KWin is configured for IBus but has not started it", Some("reselect \"IBus Wayland\" in System Settings → Keyboard → Virtual Keyboard, or run `wayexpand setup --backend ibus`".into())),
                    None => check("desktop", "KWin sends input to IBus", IbusCheckState::Warn, "KWin is configured for IBus; its state could not be queried", None),
                }
            }
        }
        IbusDesktop::Gnome => check("desktop", "The desktop sends input to IBus", IbusCheckState::Pass, "GNOME Shell routes input methods to IBus", None),
        IbusDesktop::OtherWayland => check("desktop", "The desktop sends input to IBus", IbusCheckState::Warn, "this compositor must route input methods to IBus; WayExpand cannot verify it", Some("use your compositor's input-method support for IBus (for example `ibus-daemon --panel disable` with input-method-v2), then run `wayexpand ibus watch`".into())),
        IbusDesktop::X11 => check("desktop", "Applications use IBus", IbusCheckState::Warn, "X11 applications reach IBus through GTK_IM_MODULE/QT_IM_MODULE/XMODIFIERS", Some("set GTK_IM_MODULE=ibus QT_IM_MODULE=ibus XMODIFIERS=@im=ibus for your session".into())),
        IbusDesktop::Unknown => check("desktop", "The desktop sends input to IBus", IbusCheckState::Skip, "no graphical session detected", None),
    });

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

    // 5. The manifest describes this engine.
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

    IbusChainReport {
        desktop,
        checks,
        activity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn report_readiness_follows_failures_only() {
        let report = IbusChainReport {
            desktop: IbusDesktop::Unknown,
            checks: vec![
                check("a", "a", IbusCheckState::Pass, "", None),
                check("b", "b", IbusCheckState::Warn, "", None),
            ],
            activity: None,
        };
        assert!(report.ready());
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
