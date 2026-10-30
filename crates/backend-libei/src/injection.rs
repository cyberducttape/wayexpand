//! `TextInjector` protocol implementation and output sequencing.

use super::*;

impl LibeiSession {
    pub(super) fn shutdown(mut self) {
        // Close the portal session after the EIS connection is dropped, so
        // the portal never sees a live client on a closed session.
        let portal = self._portal.take();
        drop(self);
        if let Some(portal) = portal {
            portal.close();
        }
    }

    pub(super) fn capabilities(&self) -> InjectorCapabilities {
        let keysym_fallback = matches!(self.mode, TextMode::Keysym(_));
        let direct_commit = self.fcitx5.is_some();
        InjectorCapabilities {
            insertion_mode: if direct_commit {
                "Fcitx5 direct commit (libei fallback available)"
            } else if keysym_fallback {
                "libei keysym fallback"
            } else {
                "ei_text"
            },
            // Direct Fcitx5 commits are bounded by validate_text/MAX_TEXT_BYTES,
            // not by the paced keysym fallback's three-second budget. If the
            // bridge declines because there is no focused Fcitx context, the
            // replace method still applies ensure_representable before any raw
            // erase, so this optimistic capability cannot make fallback unsafe.
            max_text_chars: if keysym_fallback && !direct_commit {
                wayexpand_core::keystroke_max_chars()
            } else {
                0
            },
            expected_throughput_chars_per_sec: (keysym_fallback && !direct_commit).then_some(83),
            // Even a single ei_text flush can fail after the target has
            // processed part of the transaction; libei has no rollback
            // primitive for arbitrary application text.
            atomic_replace: false,
            // Backspaces are sent without seeing the target's text.
            replacement_guarantee: wayexpand_core::ReplacementGuarantee::BestEffort,
            full_unicode: direct_commit || matches!(self.mode, TextMode::Text(_)),
            cursor_reposition: true,
            key_passthrough: true,
        }
    }

    pub(super) fn status_detail(&self) -> &'static str {
        match (&self.mode, self.fcitx5.is_some()) {
            (TextMode::Text(_), true) => "ei_text (UTF-8 insertion) + Fcitx5 direct commit",
            (TextMode::Keysym(_), true) => {
                "ei_keyboard keysym fallback (12ms key pacing) + Fcitx5 direct commit"
            }
            (_, false) => self.mode.status_detail(),
        }
    }

    pub(super) fn set_output_checkpoint(&mut self, checkpoint: wayexpand_core::OutputCheckpoint) {
        self.checkpoint = checkpoint;
    }

    pub(super) fn preflight_text(&mut self, text: &str) -> Result<(), InjectorError> {
        // Refresh modifiers/layout before the transaction is classified. If
        // the active group is unknown, refuse here so the trigger remains
        // untouched and the engine reports a clean not-applied outcome.
        self.sync_server_state()
            .and_then(|()| {
                validate_text(text)?;
                self.ensure_representable(text)
            })
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: false,
            })
    }

    pub(super) fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.sync_server_state()
            .and_then(|()| self.send_backspaces(erase_grapheme_count(trigger)))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    pub(super) fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.sync_server_state()
            .and_then(|()| self.send_text(text))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    pub(super) fn replace(&mut self, trigger: &str, text: &str) -> Result<(), InjectorError> {
        // Synchronize before any replacement attempt. In keysym mode this
        // refreshes the layout used by the fallback; the Fcitx5 path itself
        // uses the focused input context rather than this keymap.
        self.sync_server_state().map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })?;
        validate_text(text).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })?;
        if let Some(client) = &self.fcitx5 {
            self.checkpoint.check().map_err(|message| InjectorError {
                backend: BACKEND_NAME,
                message,
                retryable: false,
            })?;
            match client.replace(trigger, text) {
                fcitx5::ReplaceResult::Committed => return Ok(()),
                fcitx5::ReplaceResult::FallbackAllowed(_) => {}
                fcitx5::ReplaceResult::Blocked(message) => {
                    return Err(InjectorError {
                        backend: BACKEND_NAME,
                        message: format!("Fcitx5 direct replacement blocked: {message}"),
                        retryable: false,
                    })
                }
                fcitx5::ReplaceResult::TransportFailed(message) => {
                    return Err(InjectorError {
                        backend: BACKEND_NAME,
                        message: format!("Fcitx5 direct replacement failed: {message}"),
                        retryable: true,
                    })
                }
            }
        }
        // Only the ordinary libei route needs the active-keymap check. Do it
        // after the optional direct commit attempt so a keysym-only EIS
        // device can still commit Unicode through Fcitx5.
        self.ensure_representable(text)
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })?;
        self.send_backspaces(erase_grapheme_count(trigger))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })?;
        if matches!(self.mode, TextMode::Keysym(_)) {
            // Send the erase on its own and let it land before typing: in
            // keysym mode both halves are key events on the same device, so
            // batching them into one flush lets a late-applied backspace eat
            // a character that was already typed.
            std::thread::sleep(KEY_EVENT_INTERVAL);
            return self.type_keys(text).map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            });
        }
        self.checkpoint.check().map_err(|message| InjectorError {
            backend: BACKEND_NAME,
            message,
            retryable: false,
        })?;
        self.send_text(text).map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: error.to_string(),
            retryable: error.is_retryable(),
        })
    }

    pub(super) fn move_cursor_left(&mut self, count: usize) -> Result<(), InjectorError> {
        // Cursor keys follow the typed text; they must not reach a window
        // that took focus while the text was being typed.
        self.checkpoint
            .check()
            .map_err(LibeiError::Interrupted)
            .and_then(|()| self.sync_server_state())
            .and_then(|()| self.send_left_arrows(count))
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    pub(super) fn inject_key(&mut self, keycode: u32) -> Result<(), InjectorError> {
        self.send_key_with_modifiers(keycode, Modifiers::default())
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    pub(super) fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        self.send_key_with_modifiers(keycode, modifiers)
            .map_err(|error| InjectorError {
                backend: BACKEND_NAME,
                message: error.to_string(),
                retryable: error.is_retryable(),
            })
    }

    pub(super) fn inject_key_event(
        &mut self,
        keycode: u32,
        _modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        let serial = self.connection.serial();
        self.device.device().start_emulating(serial, self.sequence);
        self.sequence = self.sequence.checked_add(1).unwrap_or(1);
        self.keyboard.key(
            keycode,
            match state {
                KeyEventState::Pressed => ei::keyboard::KeyState::Press,
                KeyEventState::Released => ei::keyboard::KeyState::Released,
            },
        );
        self.device
            .device()
            .frame(serial, self.started_at.elapsed().as_micros() as u64);
        self.device.device().stop_emulating(serial);
        self.connection.flush().map_err(|error| InjectorError {
            backend: BACKEND_NAME,
            message: LibeiError::Flush(error.to_string()).to_string(),
            retryable: true,
        })
    }
}
