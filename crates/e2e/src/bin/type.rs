//! `wx-e2e-type text TEXT` / `wx-e2e-type key NAME [COUNT]`
//!
//! Plays the user in the headless end-to-end tests: types TEXT, or presses a
//! named key COUNT times, into the focused window through one of the
//! injectors WayExpand itself uses for output. `WX_E2E_TYPIST` picks it:
//! - `libei` (the default, for KWin) connects through `LIBEI_SOCKET` (the
//!   e2e EIS relay), never the RemoteDesktop portal;
//! - `wlroots` (for Sway) creates its own virtual keyboard on
//!   `WAYLAND_DISPLAY`, separate from the daemon's. It types `text` and
//!   `keys` the same way (each character as its own keysym, which an input
//!   method's keyboard grab decodes too) and supports only `key Left`, the
//!   one named key that injector can send.
//!
//! With several layouts configured, the libei typist refuses (as the
//! product does) unless `WX_E2E_ACTIVE_LAYOUT=GROUP:LAYOUTS:VARIANTS`
//! names the active group, for example `1:us,de:,`. The harness reads it
//! from the compositor; this helper never guesses.

use anyhow::{bail, Context, Result};
use wayexpand_backend_libei::{ActiveKeyboardLayout, LibeiInjector, LibeiOptions};
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::TextInjector;

const TYPIST_LINGER: std::time::Duration = std::time::Duration::from_millis(300);

/// Linux evdev key codes for the keys the scenarios need.
fn key_code(name: &str) -> Option<u32> {
    Some(match name {
        "BackSpace" => 14,
        "Tab" => 15,
        "Return" => 28,
        "space" => 57,
        "Home" => 102,
        "Left" => 105,
        "Right" => 106,
        "End" => 107,
        "Delete" => 111,
        _ => return None,
    })
}

fn ascii_key_code(character: char) -> Option<u32> {
    Some(match character {
        'a' => 30,
        'b' => 48,
        'c' => 46,
        'd' => 32,
        'e' => 18,
        'f' => 33,
        'g' => 34,
        'h' => 35,
        'i' => 23,
        'j' => 36,
        'k' => 37,
        'l' => 38,
        'm' => 50,
        'n' => 49,
        'o' => 24,
        'p' => 25,
        'q' => 16,
        'r' => 19,
        's' => 31,
        't' => 20,
        'u' => 22,
        'v' => 47,
        'w' => 17,
        'x' => 45,
        'y' => 21,
        'z' => 44,
        '0' => 11,
        '1' => 2,
        '2' => 3,
        '3' => 4,
        '4' => 5,
        '5' => 6,
        '6' => 7,
        '7' => 8,
        '8' => 9,
        '9' => 10,
        ' ' => 57,
        _ => return None,
    })
}

/// Parse `GROUP:LAYOUTS:VARIANTS` from `WX_E2E_ACTIVE_LAYOUT`.
fn active_layout_from_env() -> Result<Option<ActiveKeyboardLayout>> {
    let Ok(value) = std::env::var("WX_E2E_ACTIVE_LAYOUT") else {
        return Ok(None);
    };
    let mut parts = value.splitn(3, ':');
    let (Some(group), Some(layout), Some(variant)) = (parts.next(), parts.next(), parts.next())
    else {
        bail!("WX_E2E_ACTIVE_LAYOUT must be GROUP:LAYOUTS:VARIANTS");
    };
    Ok(Some(ActiveKeyboardLayout {
        generation: 1,
        group: group.parse().context("WX_E2E_ACTIVE_LAYOUT group")?,
        rules: None,
        // Only the ordered layout names are compared with the EIS keymap.
        model: Some("pc105".to_owned()),
        layout: Some(layout.to_owned()),
        variant: Some(variant.to_owned()),
        options: None,
    }))
}

fn connect() -> Result<Box<dyn TextInjector>> {
    match std::env::var("WX_E2E_TYPIST").as_deref() {
        Ok("libei") | Err(std::env::VarError::NotPresent) => Ok(Box::new(
            LibeiInjector::connect(LibeiOptions {
                persist_portal_token: false,
                fcitx5_direct_commit: false,
                portal_token_path: None,
                active_layout_provider: active_layout_from_env()?.map(|layout| {
                    Box::new(move || Some(layout.clone()))
                        as wayexpand_backend_libei::ActiveLayoutProvider
                }),
            })
            .context("connecting to EIS through LIBEI_SOCKET")?,
        )),
        Ok("wlroots") => Ok(Box::new(
            WlrootsInjector::connect().context("creating a wlroots virtual keyboard")?,
        )),
        _ => bail!("WX_E2E_TYPIST must be libei or wlroots"),
    }
}

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut injector = connect()?;
    let result: Result<(), String> = match arguments.as_slice() {
        [mode, text] if mode == "text" => injector.insert(text).map_err(|error| error.message),
        // Its per-output keymap already sends real key events, which an
        // input method's grab receives like a physical keyboard's.
        [mode, text] if mode == "keys" && injector.name() == "wlroots-virtual-keyboard" => {
            injector.insert(text).map_err(|error| error.message)
        }
        [mode, name, rest @ ..]
            if mode == "key"
                && name == "Left"
                && rest.len() <= 1
                && injector.name() == "wlroots-virtual-keyboard" =>
        {
            let count: usize = match rest.first() {
                Some(count) => count
                    .parse()
                    .map_err(|_| anyhow::anyhow!("COUNT must be a number"))?,
                None => 1,
            };
            injector
                .move_cursor_left(count)
                .map_err(|error| error.message)
        }
        [mode, text] if mode == "keys" => text.chars().try_for_each(|character| {
            let code = ascii_key_code(character)
                .ok_or_else(|| format!("unsupported test key {character:?}"))?;
            injector.inject_key(code).map_err(|error| error.message)
        }),
        [mode, name, rest @ ..] if mode == "key" && rest.len() <= 1 => {
            let Some(code) = key_code(name) else {
                bail!("unknown key {name:?}");
            };
            let count: usize = match rest.first() {
                Some(count) => count
                    .parse()
                    .map_err(|_| anyhow::anyhow!("COUNT must be a number"))?,
                None => 1,
            };
            (0..count).try_for_each(|_| injector.inject_key(code).map_err(|error| error.message))
        }
        _ => bail!("usage: wx-e2e-type text TEXT | keys ASCII | key NAME [COUNT]"),
    };
    let outcome = result.map_err(|message| anyhow::anyhow!("typing failed: {message}"));
    if injector.name() == "wlroots-virtual-keyboard" {
        // Sway 1.11 delivers none of a virtual keyboard's keys when its
        // client disconnects right after sending them. The daemon's keyboard
        // lives as long as the daemon; this one-shot typist must linger.
        std::thread::sleep(TYPIST_LINGER);
    }
    injector.shutdown();
    outcome
}
