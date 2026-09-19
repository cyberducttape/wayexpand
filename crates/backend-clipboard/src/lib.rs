use std::{
    io::{Read, Write},
    process::{Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use thiserror::Error;
use wayexpand_core::TextInjector;

const BACKEND_NAME: &str = "clipboard";
/// Upper bound on any single external process (`xclip`/`xsel`/`xdotool`/
/// `which`) this backend spawns. Every one of these calls is reachable
/// synchronously from the daemon's main loop (`text_contains_newlines` in
/// `crates/daemon/src/main.rs` routes any expansion with a newline through
/// here), so an unbounded wait -- e.g. `xclip` hanging against a wedged X
/// server or a clipboard manager holding the selection open -- would stall
/// the whole daemon exactly like the D-Bus hang found and fixed elsewhere
/// this session, including delaying `stop_requested` past systemd's
/// shutdown timeout. A local X11 round trip normally completes in single
/// digit milliseconds; this exists specifically for when it does not.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

/// Runs `command` to completion and returns its output, or a timeout error
/// if it doesn't finish within `timeout`. `stdin_data`, if given, is
/// written to the child's stdin before waiting. On timeout the child is
/// killed. Output is drained on a separate thread while polling for exit
/// (mirroring `crates/core/src/engine.rs::run_command`'s pattern) so a
/// child that writes more than the pipe buffer holds can't deadlock this
/// against a `try_wait` loop that never reads its stdout. Callers within
/// this crate use `COMMAND_TIMEOUT`; the parameter exists so tests can
/// exercise the timeout path without a multi-second real wait.
fn run_bounded(
    mut command: Command,
    stdin_data: Option<&str>,
    timeout: Duration,
) -> std::io::Result<Output> {
    command
        .stdin(if stdin_data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(data) = stdin_data {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data.as_bytes());
            // Dropped here (end of scope): closes the write end so the
            // child sees EOF on its stdin instead of waiting for more.
        }
    }
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let (stdout_tx, stdout_rx) = mpsc::sync_channel(1);
    let (stderr_tx, stderr_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buf);
        }
        let _ = stdout_tx.send(buf);
    });
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buf);
        }
        let _ = stderr_tx.send(buf);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "command exceeded the bounded timeout",
                ));
            }
        }
    };
    let stdout = stdout_rx
        .recv_timeout(Duration::from_millis(500))
        .unwrap_or_default();
    let stderr = stderr_rx
        .recv_timeout(Duration::from_millis(500))
        .unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("clipboard: failed to set clipboard content")]
    SetClipboard,
    #[error("clipboard: failed to paste (xdotool or xclip not available)")]
    Paste,
    #[error("clipboard: {0}")]
    Other(String),
}

pub struct ClipboardInjector;

impl ClipboardInjector {
    pub fn new() -> Result<Self, ClipboardError> {
        // `insert`/`erase` synthesize key events via `xdotool`, which speaks
        // X11/XTest and has no notion of a Wayland-native surface: it always
        // targets whichever window currently holds *X11* input focus. On a
        // pure Wayland session with no XWayland running, that target simply
        // does not exist ($DISPLAY is unset) and every paste/erase silently
        // fails; with XWayland present but the focused window Wayland-native,
        // xdotool would instead misdirect keystrokes at some unrelated X11
        // window. Neither is acceptable for a backend the daemon can select
        // automatically (see `text_contains_newlines` in the daemon), so
        // refuse to construct rather than let either happen at runtime.
        if std::env::var_os("DISPLAY").is_none() {
            return Err(ClipboardError::Other(
                "no X11 DISPLAY available (xdotool cannot target a Wayland-native window); \
                 this backend requires XWayland"
                    .into(),
            ));
        }

        let which = |program: &str| {
            let mut command = Command::new("which");
            command.arg(program);
            run_bounded(command, None, COMMAND_TIMEOUT)
                .ok()
                .is_some_and(|o| o.status.success())
        };

        if !which("xclip") && !which("xsel") {
            return Err(ClipboardError::Other(
                "neither xclip nor xsel found in PATH".into(),
            ));
        }
        // `insert`/`erase` shell out to xdotool unconditionally (there is no
        // fallback for it the way there is between xclip/xsel); checking
        // only the clipboard tool here let construction succeed and then
        // fail confusingly on the first real paste/erase instead.
        if !which("xdotool") {
            return Err(ClipboardError::Other("xdotool not found in PATH".into()));
        }

        Ok(ClipboardInjector)
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<(), ClipboardError> {
        // Try xclip first, fall back to xsel
        let mut command = Command::new("xclip");
        command.arg("-selection").arg("clipboard").arg("-i");
        if run_bounded(command, Some(text), COMMAND_TIMEOUT)
            .is_ok_and(|output| output.status.success())
        {
            return Ok(());
        }

        let mut command = Command::new("xsel");
        command.arg("--clipboard").arg("--input");
        if run_bounded(command, Some(text), COMMAND_TIMEOUT)
            .is_ok_and(|output| output.status.success())
        {
            return Ok(());
        }

        Err(ClipboardError::SetClipboard)
    }

    fn trigger_paste(&self) -> Result<(), ClipboardError> {
        // Use xdotool to simulate Ctrl+V paste
        let mut command = Command::new("xdotool");
        command.arg("key").arg("ctrl+v");
        match run_bounded(command, None, COMMAND_TIMEOUT) {
            Ok(output) if output.status.success() => Ok(()),
            _ => Err(ClipboardError::Paste),
        }
    }
}

impl ClipboardInjector {
    fn get_clipboard(&self) -> Option<String> {
        // xclip and xsel are independent optional dependencies (`new` only
        // requires one of them to be present, matching `copy_to_clipboard`'s
        // fallback below): a spawn failure here (binary missing entirely,
        // not just a non-zero exit) must still fall through to xsel rather
        // than short-circuit via `?`, or an xsel-only install silently never
        // saves the original clipboard for `insert` to restore.
        let mut command = Command::new("xclip");
        command.arg("-selection").arg("clipboard").arg("-o");
        if let Ok(output) = run_bounded(command, None, COMMAND_TIMEOUT) {
            if output.status.success() {
                return String::from_utf8(output.stdout).ok();
            }
        }

        let mut command = Command::new("xsel");
        command.arg("--clipboard").arg("--output");
        let output = run_bounded(command, None, COMMAND_TIMEOUT).ok()?;

        if output.status.success() {
            String::from_utf8(output.stdout).ok()
        } else {
            None
        }
    }
}

impl TextInjector for ClipboardInjector {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn erase(&mut self, trigger: &str) -> Result<(), wayexpand_core::InjectorError> {
        let backspace_count = trigger.chars().count();
        if backspace_count == 0 {
            return Ok(());
        }
        // One `xdotool` invocation for the whole trigger via `--repeat`,
        // rather than spawning a separate process per character: each
        // spawn is a real, user-visible delay (fork/exec plus an X11 round
        // trip), so erasing even a modest 20-character trigger one
        // character at a time could take the better part of a second.
        let mut command = Command::new("xdotool");
        command
            .arg("key")
            .arg("--repeat")
            .arg(backspace_count.to_string())
            .arg("BackSpace");
        let output = run_bounded(command, None, COMMAND_TIMEOUT).map_err(|e| {
            wayexpand_core::InjectorError {
                backend: BACKEND_NAME,
                message: format!("xdotool backspace failed: {}", e),
                retryable: false,
            }
        })?;
        // A non-zero exit means the trigger was NOT actually erased. This
        // must be a hard error, not a warning: `replace()` calls `insert()`
        // right after this, and if that proceeds anyway the replacement
        // text gets typed after the still-present trigger instead of in
        // place of it.
        if !output.status.success() {
            return Err(wayexpand_core::InjectorError {
                backend: BACKEND_NAME,
                message: format!(
                    "xdotool backspace exited with {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                retryable: false,
            });
        }
        Ok(())
    }

    fn insert(&mut self, text: &str) -> Result<(), wayexpand_core::InjectorError> {
        // Save original clipboard
        let original_clipboard = self.get_clipboard();

        self.copy_to_clipboard(text)
            .map_err(|e| wayexpand_core::InjectorError {
                backend: BACKEND_NAME,
                message: e.to_string(),
                retryable: false,
            })?;

        std::thread::sleep(std::time::Duration::from_millis(100));

        self.trigger_paste()
            .map_err(|e| wayexpand_core::InjectorError {
                backend: BACKEND_NAME,
                message: e.to_string(),
                retryable: false,
            })?;

        // Restore original clipboard if we saved it. `trigger_paste`
        // returning only means the XTest Ctrl+V event was dispatched, not
        // that the target application finished consuming the clipboard --
        // there is no portable way to detect that here. Wait at least as
        // long as we waited before pasting (previously this was shorter,
        // which made the restore race ahead of slower apps on exactly the
        // large snippets this backend exists for) and scale a little with
        // text size as a best-effort margin, not a real synchronization.
        if let Some(original) = original_clipboard {
            let restore_delay =
                std::time::Duration::from_millis((150 + text.len() as u64 / 100).min(500));
            std::thread::sleep(restore_delay);
            let _ = self.copy_to_clipboard(&original);
        }

        Ok(())
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), wayexpand_core::InjectorError> {
        self.erase(trigger)?;
        self.insert(text)?;
        Ok(())
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), wayexpand_core::InjectorError> {
        if count == 0 {
            return Ok(());
        }
        let mut command = Command::new("xdotool");
        command
            .arg("key")
            .arg("--repeat")
            .arg(count.to_string())
            .arg("Left");
        let output = run_bounded(command, None, COMMAND_TIMEOUT).map_err(|e| {
            wayexpand_core::InjectorError {
                backend: BACKEND_NAME,
                message: format!("xdotool cursor-left failed: {}", e),
                retryable: false,
            }
        })?;
        if !output.status.success() {
            return Err(wayexpand_core::InjectorError {
                backend: BACKEND_NAME,
                message: format!(
                    "xdotool cursor-left exited with {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                retryable: false,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_injector_can_be_created() {
        let _injector = ClipboardInjector::new();
        // May fail in CI if xclip/xsel not available, that's OK
    }

    #[test]
    fn run_bounded_kills_and_returns_timed_out_for_a_hanging_command() {
        let mut command = Command::new("sleep");
        command.arg("30");
        let result = run_bounded(command, None, Duration::from_millis(200));
        let error = result.expect_err("a 30s sleep must not complete within a 200ms timeout");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn run_bounded_returns_promptly_for_a_command_that_finishes_in_time() {
        let command = Command::new("true");
        let output = run_bounded(command, None, Duration::from_secs(2))
            .expect("`true` should exit successfully well within 2s");
        assert!(output.status.success());
    }

    #[test]
    fn run_bounded_writes_stdin_and_captures_stdout() {
        let command = Command::new("cat");
        let output = run_bounded(command, Some("hello"), Duration::from_secs(2))
            .expect("`cat` should echo stdin to stdout");
        assert_eq!(output.stdout, b"hello");
    }
}
