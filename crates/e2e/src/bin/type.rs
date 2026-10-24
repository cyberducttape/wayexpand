//! `wx-e2e-type text TEXT` / `wx-e2e-type key NAME [COUNT]`
//!
//! Plays the user in the headless end-to-end tests: types TEXT, or presses a
//! named key COUNT times, into the focused window through one of the
//! injectors WayExpand itself uses for output. `WX_E2E_TYPIST` picks it:
//! - `libei` (the default, for KWin) connects through `LIBEI_SOCKET` (the
//!   e2e EIS relay), never the RemoteDesktop portal;
//! - `wlroots` (for Sway) creates its own virtual keyboard on
//!   `WAYLAND_DISPLAY`, separate from the daemon's. Only `text` is
//!   supported there, because that injector has no named-key output.

use anyhow::{bail, Context, Result};
use wayexpand_backend_libei::{LibeiInjector, LibeiOptions};
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

fn connect() -> Result<Box<dyn TextInjector>> {
    match std::env::var("WX_E2E_TYPIST").as_deref() {
        Ok("libei") | Err(std::env::VarError::NotPresent) => Ok(Box::new(
            LibeiInjector::connect(LibeiOptions {
                persist_portal_token: false,
                fcitx5_direct_commit: false,
                portal_token_path: None,
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
    let result = match arguments.as_slice() {
        [mode, text] if mode == "text" => injector.insert(text),
        [mode, name, rest @ ..] if mode == "key" && rest.len() <= 1 => {
            let code = key_code(name).with_context(|| format!("unknown key {name:?}"))?;
            let count: usize = match rest.first() {
                Some(count) => count.parse().context("COUNT must be a number")?,
                None => 1,
            };
            (0..count).try_for_each(|_| injector.inject_key(code))
        }
        _ => bail!("usage: wx-e2e-type text TEXT | wx-e2e-type key NAME [COUNT]"),
    };
    let outcome = result.map_err(|error| anyhow::anyhow!("typing failed: {}", error.message));
    if injector.name() == "wlroots-virtual-keyboard" {
        // Sway 1.11 delivers none of a virtual keyboard's keys when its
        // client disconnects right after sending them. The daemon's keyboard
        // lives as long as the daemon; this one-shot typist must linger.
        std::thread::sleep(TYPIST_LINGER);
    }
    injector.shutdown();
    outcome
}
