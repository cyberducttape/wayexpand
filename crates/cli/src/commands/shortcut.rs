//! Give the Quick Picker (`wayexpand-gui --picker`) a global keyboard
//! shortcut, on the desktops that let an application ask for one.
//!
//! - KDE Plasma: the `picker` action of `wayexpand.desktop`, through KDE's
//!   global shortcut service (`org.kde.kglobalaccel`), the same place System
//!   Settings › Shortcuts writes. KDE runs the desktop file's action itself.
//! - GNOME: a custom keybinding under
//!   `org.gnome.settings-daemon.plugins.media-keys`, the list Settings ›
//!   Keyboard › Custom Shortcuts edits.
//!
//! A key combination another action already uses is refused, never taken
//! over. Other desktops get the command to bind by hand.

use super::Args;
use crate::*;

/// Free by default on KDE Plasma and GNOME. Meta+. (the natural choice) is
/// Plasma's emoji selector.
pub(crate) const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+Space";
const PICKER_COMMAND: &str = "wayexpand-gui --picker";
const USAGE: &str = "usage: wayexpand shortcut [status] [--json] | set [KEYS] | remove";

pub(crate) fn shortcut_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let json = take_json_flag(&mut rest);
    let desktop = Desktop::detect();
    match rest.first().map(String::as_str) {
        None | Some("status") if rest.len() <= 1 => {
            let current = desktop.current()?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "desktop": desktop.name(),
                        "supported": desktop.supported(),
                        "shortcut": current.as_ref().map(Keys::display),
                        "command": PICKER_COMMAND,
                    })
                );
            } else {
                match (&current, desktop.supported()) {
                    (Some(keys), _) => println!(
                        "Quick Picker shortcut: {} ({})",
                        keys.display(),
                        desktop.name()
                    ),
                    (None, true) => println!(
                        "No Quick Picker shortcut is set. `wayexpand shortcut set` assigns {DEFAULT_SHORTCUT}."
                    ),
                    (None, false) => println!("{}", desktop.manual_instructions()),
                }
            }
            Ok(())
        }
        Some("set") if !json && rest.len() <= 2 => {
            let spec = rest.get(1).map_or(DEFAULT_SHORTCUT, String::as_str);
            let keys = Keys::parse(spec).map_err(|reason| usage_error(&reason))?;
            if !desktop.supported() {
                bail!("{}", desktop.manual_instructions());
            }
            desktop.set(&keys)?;
            println!(
                "Quick Picker shortcut set to {} on {}. Press it in any text field to search and insert a snippet.",
                keys.display(),
                desktop.name()
            );
            Ok(())
        }
        Some("remove") if !json && rest.len() == 1 => {
            if !desktop.supported() {
                bail!("{}", desktop.manual_instructions());
            }
            desktop.remove()?;
            println!("Quick Picker shortcut removed.");
            Ok(())
        }
        _ => usage_bail!("{USAGE}"),
    }
}

/// A key combination: modifiers plus one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Keys {
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
    key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    /// How the key is written for people: "Space", "Period", "K", "F5".
    label: &'static str,
    /// GNOME accelerator (X keysym) name.
    keysym: &'static str,
    /// `Qt::Key` value, as KDE stores shortcuts.
    qt: i32,
}

const QT_SHIFT: i32 = 0x0200_0000;
const QT_CONTROL: i32 = 0x0400_0000;
const QT_ALT: i32 = 0x0800_0000;
const QT_META: i32 = 0x1000_0000;

const NAMED_KEYS: &[(&[&str], Key)] = &[
    (
        &["space"],
        Key {
            label: "Space",
            keysym: "space",
            qt: 0x20,
        },
    ),
    (
        &["period", "."],
        Key {
            label: "Period",
            keysym: "period",
            qt: 0x2e,
        },
    ),
    (
        &["comma", ","],
        Key {
            label: "Comma",
            keysym: "comma",
            qt: 0x2c,
        },
    ),
    (
        &["slash", "/"],
        Key {
            label: "Slash",
            keysym: "slash",
            qt: 0x2f,
        },
    ),
    (
        &["semicolon", ";"],
        Key {
            label: "Semicolon",
            keysym: "semicolon",
            qt: 0x3b,
        },
    ),
    (
        &["minus", "-"],
        Key {
            label: "Minus",
            keysym: "minus",
            qt: 0x2d,
        },
    ),
    (
        &["equal", "="],
        Key {
            label: "Equal",
            keysym: "equal",
            qt: 0x3d,
        },
    ),
    (
        &["insert", "ins"],
        Key {
            label: "Insert",
            keysym: "Insert",
            qt: 0x0100_0006,
        },
    ),
];

const LETTERS: [&str; 26] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];
const LOWER_LETTERS: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];
const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
const FUNCTION_KEYS: [&str; 12] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];

impl Key {
    fn parse(token: &str) -> Option<Self> {
        let lower = token.to_ascii_lowercase();
        if let Some((_, key)) = NAMED_KEYS
            .iter()
            .find(|(names, _)| names.contains(&lower.as_str()))
        {
            return Some(*key);
        }
        if let Some(index) = LOWER_LETTERS.iter().position(|letter| *letter == lower) {
            return Some(Key {
                label: LETTERS[index],
                keysym: LOWER_LETTERS[index],
                qt: 0x41 + index as i32,
            });
        }
        if let Some(index) = DIGITS.iter().position(|digit| *digit == lower) {
            return Some(Key {
                label: DIGITS[index],
                keysym: DIGITS[index],
                qt: 0x30 + index as i32,
            });
        }
        FUNCTION_KEYS
            .iter()
            .position(|name| name.eq_ignore_ascii_case(token))
            .map(|index| Key {
                label: FUNCTION_KEYS[index],
                keysym: FUNCTION_KEYS[index],
                qt: 0x0100_0030 + index as i32,
            })
    }

    fn from_qt(code: i32) -> Option<Self> {
        let all = NAMED_KEYS
            .iter()
            .map(|(_, key)| *key)
            .chain((0..26).map(|index| Key {
                label: LETTERS[index],
                keysym: LOWER_LETTERS[index],
                qt: 0x41 + index as i32,
            }))
            .chain((0..10).map(|index| Key {
                label: DIGITS[index],
                keysym: DIGITS[index],
                qt: 0x30 + index as i32,
            }))
            .chain((0..12).map(|index| Key {
                label: FUNCTION_KEYS[index],
                keysym: FUNCTION_KEYS[index],
                qt: 0x0100_0030 + index as i32,
            }));
        let mut all = all;
        all.find(|key| key.qt == code)
    }

    fn from_keysym(name: &str) -> Option<Self> {
        Self::parse(name).filter(|key| key.keysym.eq_ignore_ascii_case(name))
    }
}

impl Keys {
    /// Parse "Ctrl+Alt+Space", "Super+period", "meta+.", "Ctrl+Shift+K".
    /// A shortcut needs Ctrl, Alt or Super, so it can never replace a key
    /// the user types.
    pub(crate) fn parse(spec: &str) -> std::result::Result<Self, String> {
        let invalid = || {
            format!(
                "`{spec}` is not a key combination; write it like {DEFAULT_SHORTCUT} or Super+Period"
            )
        };
        let mut keys = Keys {
            ctrl: false,
            alt: false,
            shift: false,
            super_key: false,
            key: Key {
                label: "",
                keysym: "",
                qt: 0,
            },
        };
        let tokens: Vec<&str> = spec.split('+').map(str::trim).collect();
        // "Ctrl++" or "Ctrl+." both name a key after the last separator.
        let (last, modifiers) = match tokens.as_slice() {
            [modifiers @ .., "", ""] => ("+", modifiers),
            [modifiers @ .., last] => (*last, modifiers),
            [] => return Err(invalid()),
        };
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => keys.ctrl = true,
                "alt" => keys.alt = true,
                "shift" => keys.shift = true,
                "super" | "meta" | "win" | "logo" => keys.super_key = true,
                _ => return Err(invalid()),
            }
        }
        keys.key = Key::parse(last).ok_or_else(invalid)?;
        if !(keys.ctrl || keys.alt || keys.super_key) {
            return Err(format!(
                "`{spec}` needs Ctrl, Alt or Super, or it would replace a key you type"
            ));
        }
        Ok(keys)
    }

    pub(crate) fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.super_key {
            parts.push("Super");
        }
        parts.push(self.key.label);
        parts.join("+")
    }

    fn qt_code(&self) -> i32 {
        let mut code = self.key.qt;
        if self.ctrl {
            code |= QT_CONTROL;
        }
        if self.alt {
            code |= QT_ALT;
        }
        if self.shift {
            code |= QT_SHIFT;
        }
        if self.super_key {
            code |= QT_META;
        }
        code
    }

    fn from_qt_code(code: i32) -> Option<Self> {
        let modifiers = QT_CONTROL | QT_ALT | QT_SHIFT | QT_META;
        Some(Keys {
            ctrl: code & QT_CONTROL != 0,
            alt: code & QT_ALT != 0,
            shift: code & QT_SHIFT != 0,
            super_key: code & QT_META != 0,
            key: Key::from_qt(code & !modifiers)?,
        })
    }

    fn gnome_accelerator(&self) -> String {
        let mut accelerator = String::new();
        if self.ctrl {
            accelerator.push_str("<Control>");
        }
        if self.alt {
            accelerator.push_str("<Alt>");
        }
        if self.shift {
            accelerator.push_str("<Shift>");
        }
        if self.super_key {
            accelerator.push_str("<Super>");
        }
        accelerator.push_str(self.key.keysym);
        accelerator
    }

    /// Parse a GNOME accelerator such as `<Control><Alt>space` or
    /// `<Primary><Super>period`.
    fn from_gnome_accelerator(accelerator: &str) -> Option<Self> {
        let mut keys = Keys {
            ctrl: false,
            alt: false,
            shift: false,
            super_key: false,
            key: Key {
                label: "",
                keysym: "",
                qt: 0,
            },
        };
        let mut rest = accelerator.trim();
        while let Some(stripped) = rest.strip_prefix('<') {
            let (modifier, after) = stripped.split_once('>')?;
            match modifier.to_ascii_lowercase().as_str() {
                "control" | "ctrl" | "primary" => keys.ctrl = true,
                "alt" | "mod1" => keys.alt = true,
                "shift" => keys.shift = true,
                "super" | "meta" | "mod4" => keys.super_key = true,
                _ => return None,
            }
            rest = after;
        }
        keys.key = Key::from_keysym(rest)?;
        Some(keys)
    }
}

enum Desktop {
    Kde,
    Gnome,
    Other(String),
}

impl Desktop {
    fn detect() -> Self {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let names: Vec<String> = desktop
            .split(':')
            .map(|name| name.to_ascii_lowercase())
            .collect();
        if names.iter().any(|name| name == "kde") {
            Desktop::Kde
        } else if names.iter().any(|name| name == "gnome") {
            Desktop::Gnome
        } else {
            Desktop::Other(desktop)
        }
    }

    fn name(&self) -> &str {
        match self {
            Desktop::Kde => "KDE Plasma",
            Desktop::Gnome => "GNOME",
            Desktop::Other(name) if !name.is_empty() => name,
            Desktop::Other(_) => "this desktop",
        }
    }

    fn supported(&self) -> bool {
        !matches!(self, Desktop::Other(_))
    }

    fn manual_instructions(&self) -> String {
        format!(
            "WayExpand cannot set shortcuts on {}. Add one in your desktop's keyboard settings that runs `{PICKER_COMMAND}`.",
            self.name()
        )
    }

    fn current(&self) -> Result<Option<Keys>> {
        match self {
            Desktop::Kde => kde::current(),
            Desktop::Gnome => gnome::current(),
            Desktop::Other(_) => Ok(None),
        }
    }

    fn set(&self, keys: &Keys) -> Result<()> {
        match self {
            Desktop::Kde => kde::set(keys),
            Desktop::Gnome => gnome::set(keys),
            Desktop::Other(_) => unreachable!("checked by supported()"),
        }
    }

    fn remove(&self) -> Result<()> {
        match self {
            Desktop::Kde => kde::remove(),
            Desktop::Gnome => gnome::remove(),
            Desktop::Other(_) => unreachable!("checked by supported()"),
        }
    }
}

mod kde {
    use super::Keys;
    use crate::*;

    const SERVICE: &str = "org.kde.kglobalaccel";
    const PATH: &str = "/kglobalaccel";
    const INTERFACE: &str = "org.kde.KGlobalAccel";
    /// Component and action as KDE names a desktop-file action.
    const ACTION_ID: [&str; 4] = [
        "wayexpand.desktop",
        "picker",
        "WayExpand",
        "Quick Insert Snippet",
    ];

    type KeySequences = Vec<(Vec<i32>,)>;
    /// `KGlobalShortcutInfo`: action and component names, their friendly
    /// names, context names, keys and default keys.
    type ShortcutInfo = (
        String,
        String,
        String,
        String,
        String,
        String,
        Vec<i32>,
        Vec<i32>,
    );

    /// Call a KDE method without D-Bus auto-start. On Plasma 6 the service
    /// lives inside KWin; when it is absent, starting a standalone copy
    /// beside the compositor would do more harm than reporting the absence.
    fn call<B, R>(proxy: &zbus::blocking::Proxy<'_>, method: &str, body: &B) -> zbus::Result<R>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        proxy
            .call_with_flags(method, zbus::proxy::MethodFlags::NoAutoStart.into(), body)?
            .ok_or_else(|| zbus::Error::Failure(format!("{method} sent no reply")))
    }

    fn proxy() -> Result<zbus::blocking::Proxy<'static>> {
        let connection =
            zbus::blocking::Connection::session().context("cannot reach the session bus")?;
        zbus::blocking::Proxy::new(&connection, SERVICE, PATH, INTERFACE)
            .context("cannot reach KDE's global shortcut service")
    }

    pub(super) fn current() -> Result<Option<Keys>> {
        let keys: KeySequences = call(&proxy()?, "shortcutKeys", &(ACTION_ID.to_vec(),))
            .context("cannot read KDE's shortcut for the Quick Picker")?;
        Ok(keys
            .first()
            .and_then(|(sequence,)| sequence.first())
            .and_then(|code| Keys::from_qt_code(*code)))
    }

    pub(super) fn set(keys: &Keys) -> Result<()> {
        let proxy = proxy()?;
        let code = keys.qt_code();
        let owners: Vec<ShortcutInfo> = call(&proxy, "getGlobalShortcutsByKey", &(code,))
            .context("cannot check whether KDE already uses this shortcut")?;
        if let Some(owner) = owners
            .iter()
            .find(|owner| !(owner.2 == ACTION_ID[0] && owner.0 == ACTION_ID[1]))
        {
            bail!(
                "{} is already used by “{}” ({}). Choose another, for example `wayexpand shortcut set Ctrl+Alt+Period`.",
                keys.display(),
                owner.1,
                owner.3
            );
        }
        let action_id = ACTION_ID.to_vec();
        call::<_, ()>(&proxy, "doRegister", &(action_id.clone(),))
            .context("KDE refused to register the Quick Picker action")?;
        // Qt sends a QKeySequence as all four of its key combinations, unused
        // ones as 0; a shorter array makes KDE's demarshalling abort.
        let sequences: KeySequences = vec![(vec![code, 0, 0, 0],)];
        call::<_, ()>(&proxy, "setForeignShortcutKeys", &(action_id, sequences))
            .context("KDE refused the shortcut")?;
        match current()? {
            Some(set) if set == *keys => Ok(()),
            _ => bail!("KDE did not keep {} for the Quick Picker", keys.display()),
        }
    }

    pub(super) fn remove() -> Result<()> {
        let proxy = proxy()?;
        let empty: KeySequences = Vec::new();
        call::<_, ()>(
            &proxy,
            "setForeignShortcutKeys",
            &(ACTION_ID.to_vec(), empty),
        )
        .context("KDE refused to remove the shortcut")
    }
}

mod gnome {
    use super::{Keys, PICKER_COMMAND};
    use crate::*;
    use std::process::Command;

    const LIST_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
    const LIST_KEY: &str = "custom-keybindings";
    const ENTRY_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
    const PATH: &str =
        "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/wayexpand-picker/";

    fn gsettings(args: &[&str]) -> Result<String> {
        let output = Command::new("gsettings")
            .args(args)
            .output()
            .context("cannot run gsettings")?;
        if !output.status.success() {
            bail!(
                "gsettings {} failed: {}",
                args.first().copied().unwrap_or_default(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn entry(path: &str) -> String {
        format!("{ENTRY_SCHEMA}:{path}")
    }

    /// Parse a GVariant string array as gsettings prints it:
    /// `['a', 'b']` or `@as []`.
    pub(super) fn parse_string_array(text: &str) -> Vec<String> {
        let text = text.trim().trim_start_matches("@as").trim();
        let Some(inner) = text
            .strip_prefix('[')
            .and_then(|text| text.strip_suffix(']'))
        else {
            return Vec::new();
        };
        inner
            .split(',')
            .map(|item| {
                item.trim()
                    .trim_matches(|quote| quote == '\'' || quote == '"')
            })
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect()
    }

    pub(super) fn format_string_array(items: &[String]) -> String {
        let quoted: Vec<String> = items.iter().map(|item| format!("'{item}'")).collect();
        format!("[{}]", quoted.join(", "))
    }

    fn unquote(value: &str) -> &str {
        value.trim().trim_matches('\'')
    }

    fn custom_paths() -> Result<Vec<String>> {
        Ok(parse_string_array(&gsettings(&[
            "get",
            LIST_SCHEMA,
            LIST_KEY,
        ])?))
    }

    pub(super) fn current() -> Result<Option<Keys>> {
        if !custom_paths()?.iter().any(|path| path == PATH) {
            return Ok(None);
        }
        let binding = gsettings(&["get", &entry(PATH), "binding"])?;
        Ok(Keys::from_gnome_accelerator(unquote(&binding)))
    }

    /// Who already uses `keys`: a built-in GNOME keybinding or another
    /// custom shortcut.
    fn owner(keys: &Keys) -> Result<Option<String>> {
        for path in custom_paths()? {
            if path == PATH {
                continue;
            }
            let binding = gsettings(&["get", &entry(&path), "binding"])?;
            if Keys::from_gnome_accelerator(unquote(&binding)).as_ref() == Some(keys) {
                let name = gsettings(&["get", &entry(&path), "name"])?;
                return Ok(Some(format!("custom shortcut “{}”", unquote(&name))));
            }
        }
        for schema in [
            "org.gnome.desktop.wm.keybindings",
            "org.gnome.shell.keybindings",
            "org.gnome.mutter.keybindings",
            "org.gnome.mutter.wayland.keybindings",
            LIST_SCHEMA,
        ] {
            let Ok(listing) = gsettings(&["list-recursively", schema]) else {
                continue;
            };
            for line in listing.lines() {
                let mut fields = line.splitn(3, ' ');
                let (Some(_), Some(key), Some(value)) =
                    (fields.next(), fields.next(), fields.next())
                else {
                    continue;
                };
                let accelerators =
                    if value.trim_start().starts_with('[') || value.starts_with("@as") {
                        parse_string_array(value)
                    } else {
                        vec![unquote(value).to_owned()]
                    };
                if accelerators.iter().any(|accelerator| {
                    Keys::from_gnome_accelerator(accelerator).as_ref() == Some(keys)
                }) {
                    return Ok(Some(format!("GNOME's “{key}” shortcut")));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn set(keys: &Keys) -> Result<()> {
        if let Some(owner) = owner(keys)? {
            bail!(
                "{} is already used by {owner}. Choose another, for example `wayexpand shortcut set Ctrl+Alt+Period`.",
                keys.display()
            );
        }
        let entry = entry(PATH);
        gsettings(&["set", &entry, "name", "'WayExpand Quick Picker'"])?;
        gsettings(&["set", &entry, "command", &format!("'{}'", picker_command())])?;
        gsettings(&[
            "set",
            &entry,
            "binding",
            &format!("'{}'", keys.gnome_accelerator()),
        ])?;
        let mut paths = custom_paths()?;
        if !paths.iter().any(|path| path == PATH) {
            paths.push(PATH.to_owned());
            gsettings(&["set", LIST_SCHEMA, LIST_KEY, &format_string_array(&paths)])?;
        }
        match current()? {
            Some(set) if set == *keys => Ok(()),
            _ => bail!("GNOME did not keep {} for the Quick Picker", keys.display()),
        }
    }

    pub(super) fn remove() -> Result<()> {
        let mut paths = custom_paths()?;
        let before = paths.len();
        paths.retain(|path| path != PATH);
        if paths.len() != before {
            gsettings(&["set", LIST_SCHEMA, LIST_KEY, &format_string_array(&paths)])?;
        }
        let entry = entry(PATH);
        for key in ["name", "command", "binding"] {
            gsettings(&["reset", &entry, key])?;
        }
        Ok(())
    }

    /// `wayexpand-gui --picker` when the GUI is on PATH; otherwise the GUI
    /// installed beside this CLI. GNOME runs the command without a shell.
    fn picker_command() -> String {
        let on_path = std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("wayexpand-gui").is_file())
        });
        if on_path {
            return PICKER_COMMAND.to_owned();
        }
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("wayexpand-gui")))
            .filter(|gui| gui.is_file())
            .and_then(|gui| gui.to_str().map(|gui| format!("{gui} --picker")))
            .filter(|command| !command.contains('\''))
            .unwrap_or_else(|| PICKER_COMMAND.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{gnome, Keys, DEFAULT_SHORTCUT};

    #[test]
    fn key_combinations_parse_and_round_trip_through_both_desktops() {
        for (spec, display, accelerator, qt) in [
            (
                "Ctrl+Alt+Space",
                "Ctrl+Alt+Space",
                "<Control><Alt>space",
                0x0c00_0020,
            ),
            ("super+.", "Super+Period", "<Super>period", 0x1000_002e),
            (
                "meta+shift+k",
                "Shift+Super+K",
                "<Shift><Super>k",
                0x1200_004b,
            ),
            (
                "Control+Alt+F5",
                "Ctrl+Alt+F5",
                "<Control><Alt>F5",
                0x0d00_0034,
            ),
            ("Alt+7", "Alt+7", "<Alt>7", 0x0800_0037),
        ] {
            let keys = Keys::parse(spec).unwrap();
            assert_eq!(keys.display(), display, "{spec}");
            assert_eq!(keys.gnome_accelerator(), accelerator, "{spec}");
            assert_eq!(keys.qt_code(), qt, "{spec}");
            assert_eq!(Keys::from_qt_code(qt), Some(keys.clone()), "{spec}");
            assert_eq!(
                Keys::from_gnome_accelerator(accelerator),
                Some(keys),
                "{spec}"
            );
        }
        assert_eq!(
            Keys::from_gnome_accelerator("<Primary><Alt>space"),
            Keys::parse(DEFAULT_SHORTCUT).ok()
        );
    }

    #[test]
    fn shortcuts_must_not_replace_a_typed_key() {
        for spec in [
            "Space",
            "Shift+K",
            "",
            "Ctrl+",
            "Ctrl+Hyper+K",
            "Ctrl+Alt+Nope",
        ] {
            assert!(Keys::parse(spec).is_err(), "{spec:?}");
        }
    }

    #[test]
    fn gsettings_string_arrays_round_trip() {
        assert!(gnome::parse_string_array("@as []").is_empty());
        let paths = gnome::parse_string_array(
            "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/', '/x/']",
        );
        assert_eq!(paths.len(), 2);
        assert_eq!(
            gnome::format_string_array(&paths),
            "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/', '/x/']"
        );
    }
}
