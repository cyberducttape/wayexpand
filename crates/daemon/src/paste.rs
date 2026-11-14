//! Terminal paste mode: deliver replacements in `settings.terminal_apps`
//! through the clipboard and the terminal's paste key instead of typing them
//! key by key.
//!
//! Typing a multi-line replacement into a shell runs each line as its Enter
//! arrives, and long replacements are slow and limited by
//! `keystroke_max_chars`. A paste arrives at once, and a shell with bracketed
//! paste holds it until the user presses Enter.
//!
//! The decision is made per transaction from a process-wide mode the reactor
//! publishes for the focused window, like `keystroke_max_chars`, because the
//! output backend runs on its own worker thread. The output assurance gate
//! still wraps this injector, so consent and focus checks apply unchanged.

use std::sync::atomic::{AtomicU8, Ordering};

use wayexpand_core::{
    InjectorCapabilities, InjectorError, KeyEventState, Modifiers, OutputCheckpoint,
    OutputProgress, TerminalPaste, TextInjector,
};

use crate::clipboard::{paste_text, PasteOutcome};

/// Linux evdev code of the V key; terminals paste on Ctrl+Shift+V.
const KEY_V: u32 = 47;
const PASTE_MODIFIERS: Modifiers = Modifiers {
    ctrl: true,
    alt: false,
    shift: true,
    super_key: false,
};

/// Paste mode in effect for the focused window: the configured mode while a
/// listed terminal has focus, otherwise off.
static ACTIVE_MODE: AtomicU8 = AtomicU8::new(0);

fn encode(mode: TerminalPaste) -> u8 {
    match mode {
        TerminalPaste::Off => 0,
        TerminalPaste::Multiline => 1,
        TerminalPaste::Always => 2,
    }
}

fn decode(code: u8) -> TerminalPaste {
    match code {
        1 => TerminalPaste::Multiline,
        2 => TerminalPaste::Always,
        _ => TerminalPaste::Off,
    }
}

/// Publish the paste mode for the focused window. Called by the reactor
/// whenever focus or configuration may have changed.
pub(crate) fn set_active_mode(configured: TerminalPaste, focused_terminal: bool) {
    let mode = if focused_terminal {
        configured
    } else {
        TerminalPaste::Off
    };
    ACTIVE_MODE.store(encode(mode), Ordering::Release);
}

fn active_mode() -> TerminalPaste {
    decode(ACTIVE_MODE.load(Ordering::Acquire))
}

/// Route capabilities while paste mode is active for the focused window:
/// pasted text is neither limited by `keystroke_max_chars` nor by the
/// keyboard layout. Text this mode would type instead is still checked by
/// the typing backend's own preflight before anything is erased.
pub(crate) fn adjust_capabilities(mut capabilities: InjectorCapabilities) -> InjectorCapabilities {
    if active_mode() != TerminalPaste::Off {
        capabilities.max_text_chars = 0;
        capabilities.full_unicode = true;
    }
    capabilities
}

/// Wraps a keystroke injector and pastes replacements when paste mode is
/// active for the focused terminal.
pub(crate) struct PasteInjector {
    inner: Box<dyn TextInjector>,
    checkpoint: OutputCheckpoint,
}

impl PasteInjector {
    pub(crate) fn wrap(inner: Box<dyn TextInjector>) -> Box<dyn TextInjector> {
        Box::new(Self {
            inner,
            checkpoint: OutputCheckpoint::default(),
        })
    }

    /// Whether `text` is delivered by pasting. Text the typing backend could
    /// not type (a character missing from the keyboard layout, or over the
    /// typing limit) is pasted whenever paste mode is on at all.
    fn pastes(&mut self, text: &str) -> bool {
        let mode = active_mode();
        mode != TerminalPaste::Off
            && (mode.pastes(text, wayexpand_core::keystroke_max_chars())
                || self.inner.preflight_text(text).is_err())
    }

    /// Paste `text`. `before_press` runs once the text is on the clipboard
    /// and before the paste key, so a trigger is erased only after the
    /// clipboard is known to be usable. Returns `Ok(false)` when paste mode
    /// was unavailable: then nothing was erased or pressed, and the caller
    /// may type the text instead.
    fn paste(
        &mut self,
        text: &str,
        before_press: impl FnOnce(&mut dyn TextInjector) -> Result<(), InjectorError>,
    ) -> Result<bool, InjectorError> {
        let name = self.inner.name();
        let checkpoint = self.checkpoint.clone();
        let inner = &mut self.inner;
        let outcome = paste_text(text, || {
            before_press(inner.as_mut())?;
            // The clipboard can take a moment; do not press the paste key
            // into a window that took focus meanwhile.
            checkpoint.check().map_err(|message| InjectorError {
                backend: name,
                message,
                retryable: false,
            })?;
            inner.inject_key_with_modifiers(KEY_V, PASTE_MODIFIERS)
        })?;
        match outcome {
            PasteOutcome::Pasted => Ok(true),
            PasteOutcome::Unavailable(reason) => {
                tracing::warn!(reason, "terminal paste unavailable; typing the replacement instead");
                Ok(false)
            }
            PasteOutcome::NotRequested => Err(InjectorError {
                backend: name,
                message: "the terminal did not read the clipboard after Ctrl+Shift+V; check its paste shortcut".into(),
                retryable: false,
            }),
        }
    }
}

impl TextInjector for PasteInjector {
    fn shutdown(self: Box<Self>) {
        self.inner.shutdown();
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn capabilities(&self) -> InjectorCapabilities {
        adjust_capabilities(self.inner.capabilities())
    }

    fn uses_paced_synthetic_output(&self) -> bool {
        self.inner.uses_paced_synthetic_output()
    }

    fn replace_error_may_have_applied(&self) -> bool {
        self.inner.replace_error_may_have_applied()
    }

    fn status_detail(&self) -> &'static str {
        self.inner.status_detail()
    }

    fn admit(&self, erases: bool) -> Result<(), InjectorError> {
        self.inner.admit(erases)
    }

    fn preflight_text(&mut self, text: &str) -> Result<(), InjectorError> {
        if self.pastes(text) {
            Ok(())
        } else {
            self.inner.preflight_text(text)
        }
    }

    fn set_output_checkpoint(&mut self, checkpoint: OutputCheckpoint) {
        self.checkpoint = checkpoint.clone();
        self.inner.set_output_checkpoint(checkpoint);
    }

    fn reset_output_progress(&mut self) {
        self.inner.reset_output_progress();
    }

    fn output_progress(&self) -> OutputProgress {
        self.inner.output_progress()
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.inner.erase(trigger)
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        if self.pastes(text) && self.paste(text, |_| Ok(()))? {
            return Ok(());
        }
        // Not pasted, and nothing was sent: type it, after the same checks
        // the typing backend applies to any text.
        self.inner.preflight_text(text)?;
        self.inner.insert(text)
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        if self.pastes(text) && self.paste(text, |inner| inner.erase(trigger))? {
            return Ok(());
        }
        self.inner.preflight_text(text)?;
        self.inner.replace(trigger, text)
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        self.inner.move_cursor_left(count)
    }

    fn inject_key(&mut self, keycode: u32) -> Result<(), InjectorError> {
        self.inner.inject_key(keycode)
    }

    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        self.inner.inject_key_with_modifiers(keycode, modifiers)
    }

    fn inject_key_event(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        self.inner.inject_key_event(keycode, modifiers, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_and_unfocused_terminals_never_paste() {
        for mode in [
            TerminalPaste::Off,
            TerminalPaste::Multiline,
            TerminalPaste::Always,
        ] {
            assert_eq!(decode(encode(mode)), mode);
        }
        set_active_mode(TerminalPaste::Always, false);
        assert_eq!(active_mode(), TerminalPaste::Off);
        set_active_mode(TerminalPaste::Multiline, true);
        assert_eq!(active_mode(), TerminalPaste::Multiline);
        let adjusted = adjust_capabilities(InjectorCapabilities {
            max_text_chars: 8,
            full_unicode: false,
            ..InjectorCapabilities::default()
        });
        assert_eq!(adjusted.max_text_chars, 0);
        assert!(adjusted.full_unicode);
        set_active_mode(TerminalPaste::Off, true);
        let unchanged = adjust_capabilities(InjectorCapabilities {
            max_text_chars: 8,
            ..InjectorCapabilities::default()
        });
        assert_eq!(unchanged.max_text_chars, 8);
    }
}
