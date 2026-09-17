use std::process::Command;
use thiserror::Error;
use wayexpand_core::TextInjector;

const BACKEND_NAME: &str = "clipboard";

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
        let which = |program: &str| {
            Command::new("which")
                .arg(program)
                .output()
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
        let result = Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .ok()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait().ok()
            });

        if result.is_some_and(|status| status.success()) {
            return Ok(());
        }

        // Fall back to xsel
        let result = Command::new("xsel")
            .arg("--clipboard")
            .arg("--input")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .ok()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait().ok()
            });

        if result.is_some_and(|status| status.success()) {
            return Ok(());
        }

        Err(ClipboardError::SetClipboard)
    }

    fn trigger_paste(&self) -> Result<(), ClipboardError> {
        // Use xdotool to simulate Ctrl+V paste
        let result = Command::new("xdotool")
            .arg("key")
            .arg("ctrl+v")
            .output();

        match result {
            Ok(output) if output.status.success() => Ok(()),
            _ => Err(ClipboardError::Paste),
        }
    }
}

impl Default for ClipboardInjector {
    fn default() -> Self {
        Self::new().expect("clipboard backend requires xclip/xsel and xdotool")
    }
}

impl ClipboardInjector {
    fn get_clipboard(&self) -> Option<String> {
        let output = Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .arg("-o")
            .output()
            .ok()?;

        if output.status.success() {
            String::from_utf8(output.stdout).ok()
        } else {
            // Try xsel fallback
            let output = Command::new("xsel")
                .arg("--clipboard")
                .arg("--output")
                .output()
                .ok()?;

            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
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
        Command::new("xdotool")
            .arg("key")
            .arg("--repeat")
            .arg(backspace_count.to_string())
            .arg("BackSpace")
            .output()
            .map_err(|e| wayexpand_core::InjectorError {
                backend: BACKEND_NAME,
                message: format!("xdotool backspace failed: {}", e),
                retryable: false,
            })?;
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

        // Restore original clipboard if we saved it
        if let Some(original) = original_clipboard {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let _ = self.copy_to_clipboard(&original);
        }

        Ok(())
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), wayexpand_core::InjectorError> {
        self.erase(trigger)?;
        self.insert(text)?;
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
}
