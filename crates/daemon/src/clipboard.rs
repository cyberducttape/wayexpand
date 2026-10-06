//! `{{clipboard}}` support for the daemon.
//!
//! Reads the clipboard with `wl-paste` (wl-clipboard) through the same
//! bounded, shell-free command runner used for command-backed snippets: fixed
//! arguments, a minimal environment that passes only the Wayland session
//! variables, a short timeout, and the usual output cap. The reader runs only
//! while rendering a snippet that uses the variable, and only when the user
//! enabled it (`settings.allow_clipboard`) and policy allows it. Clipboard
//! contents are never logged or cached.

use std::sync::Arc;

use wayexpand_core::{run_command, ClipboardReader, CommandConfig, CommandEnvironment};

/// Upper bound on one `{{clipboard}}` read. A healthy `wl-paste` returns in a
/// few milliseconds; this only limits how long a stuck clipboard owner can
/// delay an expansion. Reading on demand (never caching) is deliberate, see
/// the module docs.
const CLIPBOARD_TIMEOUT_MS: u64 = 150;

fn wl_paste_command() -> CommandConfig {
    CommandConfig {
        action: None,
        program: "wl-paste".into(),
        args: vec![
            "--no-newline".into(),
            "--type".into(),
            "text/plain;charset=utf-8".into(),
        ],
        timeout_ms: CLIPBOARD_TIMEOUT_MS,
        cache_ms: 0,
        environment: CommandEnvironment::Minimal,
        pass_env: vec!["WAYLAND_DISPLAY".into(), "XDG_RUNTIME_DIR".into()],
    }
}

pub fn wl_paste_reader() -> ClipboardReader {
    let command = wl_paste_command();
    ClipboardReader(Arc::new(move || run_command(&command).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_command_is_bounded_and_passes_only_wayland_variables() {
        let command = wl_paste_command();
        wayexpand_core::validate_command_config(&command).unwrap();
        assert_eq!(command.environment, CommandEnvironment::Minimal);
        assert_eq!(command.pass_env, ["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"]);
        assert!(command.timeout_ms <= 150);
        assert_eq!(command.cache_ms, 0);
    }
}
