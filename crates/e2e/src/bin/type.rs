//! `wx-e2e-type text TEXT` / `wx-e2e-type key NAME [COUNT]`
//!
//! Plays the user in the headless KWin end-to-end tests: types TEXT, or
//! presses a named key COUNT times, into the focused window through the same
//! libei injector WayExpand uses for output. It connects through
//! `LIBEI_SOCKET` (the e2e EIS relay), never the RemoteDesktop portal.

use anyhow::{bail, Context, Result};
use wayexpand_backend_libei::{LibeiInjector, LibeiOptions};
use wayexpand_core::TextInjector;

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

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut injector = LibeiInjector::connect(LibeiOptions {
        persist_portal_token: false,
        fcitx5_direct_commit: false,
        portal_token_path: None,
    })
    .context("connecting to EIS through LIBEI_SOCKET")?;
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
    Box::new(injector).shutdown();
    outcome
}
