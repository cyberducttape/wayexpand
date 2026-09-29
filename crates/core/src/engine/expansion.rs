//! Input event processing and expansion computation.
//!
//! Processes input events (text, keys, delimiters, window changes) through the
//! expansion engine, managing buffer state, window context, and pause/focus state.

use super::matching;
use super::{ExpansionEngine, ExpansionResult, InputEvent, PendingExpansionResult};
use super::{MAX_RESULTS_PER_EVENT, MAX_RESULT_BYTES_PER_EVENT};
use crate::MatchMode;

impl ExpansionEngine {
    /// Process an input event through the expansion state machine.
    ///
    /// The public entry point lives in this module so event processing remains
    /// separated from matching policy, transaction handling, and command
    /// runtime ownership as those implementations continue to be extracted.
    pub fn process(&mut self, event: InputEvent) -> Vec<ExpansionResult> {
        self.process_internal(event)
    }

    /// Process input while reserving command-backed matches for caller policy
    /// approval and later dispatch.
    pub fn process_deferred(&mut self, event: InputEvent) -> Vec<PendingExpansionResult> {
        self.process_deferred_internal(event)
    }
}

impl ExpansionEngine {
    /// Process an event stream. A text event may contain multiple Unicode
    /// scalar values; matching is performed after each one.
    pub(super) fn process_internal(&mut self, event: InputEvent) -> Vec<ExpansionResult> {
        // A pending undo is valid only immediately after the expansion it
        // would revert, with no other event in between. Undo is preserved only
        // if the key event is the undo chord itself; any other key (navigation,
        // text, application shortcuts, etc.) invalidates it because the cursor
        // position may have changed.
        match &event {
            InputEvent::Key(chord) if !self.is_undo_chord(chord) => {
                self.last_expansion = None;
            }
            InputEvent::Key(_) => {
                // Undo chord preserves last_expansion (if any)
            }
            _ => {
                self.last_expansion = None;
            }
        }
        match event {
            InputEvent::Key(_) => {
                self.note_key_event();
                Vec::new()
            }
            InputEvent::Text(text) => {
                let mut results = Vec::new();
                let mut result_bytes = 0usize;
                if !self.is_capture_enabled() {
                    return results;
                }
                for character in text.chars() {
                    // An undoable expansion is valid only when no later
                    // scalar from this same text event follows it.
                    if !results.is_empty() {
                        self.last_expansion = None;
                    }
                    self.input_generation = self.input_generation.wrapping_add(1);
                    let pending = self.matcher.find_suffix(self.buffer.iter().rev().copied());
                    if let Some((index, length)) = pending {
                        if let Some(config_index) = self.matcher_indices.get(index).copied() {
                            let match_mode = self.config.expansion[config_index].match_mode;
                            // `trigger` is the configured lowercase form.
                            // With propagate_case, the pending suffix may
                            // instead be `:A` or `:AB`; continuation must be
                            // checked against the text the user actually
                            // typed, not the generated trie sibling.
                            let typed: String = {
                                let start = self.buffer.len().saturating_sub(length);
                                self.buffer.iter().skip(start).collect()
                            };
                            if !self.matcher.can_continue(&typed, character) {
                                let trailing_word_character = match_mode == MatchMode::WordBoundary
                                    && matching::is_word_character(character);
                                if !trailing_word_character {
                                    if let Some(result) =
                                        self.take_match(config_index, length, Some(character))
                                    {
                                        let bytes = result
                                            .trigger
                                            .len()
                                            .saturating_add(result.insert.len());
                                        if results.len() >= MAX_RESULTS_PER_EVENT
                                            || result_bytes.saturating_add(bytes)
                                                > MAX_RESULT_BYTES_PER_EVENT
                                        {
                                            self.clear_buffer();
                                            break;
                                        }
                                        result_bytes = result_bytes.saturating_add(bytes);
                                        results.push(result);
                                    }
                                }
                            }
                        }
                    }
                    if results.len() >= MAX_RESULTS_PER_EVENT {
                        self.clear_buffer();
                        break;
                    }
                    self.buffer.push_back(character);
                    while self.buffer.len() > self.max_buffer_chars {
                        self.buffer.pop_front();
                        self.buffer_truncated = true;
                    }
                    if let Some((index, length)) =
                        self.matcher.find_suffix(self.buffer.iter().rev().copied())
                    {
                        let config_index = self.matcher_indices.get(index).copied();
                        let Some(config_index) = config_index else {
                            self.clear_buffer();
                            continue;
                        };
                        let (trigger, match_mode) = {
                            let expansion = &self.config.expansion[config_index];
                            (expansion.trigger.clone(), expansion.match_mode)
                        };
                        // The actually-typed suffix, which may be an
                        // uppercase or capitalized variant of `trigger` for
                        // a `propagate_case` expansion (see
                        // `ExpansionEngine::new`): `has_continuation` must
                        // be checked against what was typed, since that is
                        // the string actually present in the forward trie,
                        // not necessarily `trigger` itself.
                        let typed: String = {
                            let start = self.buffer.len().saturating_sub(length);
                            self.buffer.iter().skip(start).collect()
                        };
                        if self.matcher.has_continuation(&typed)
                            || match_mode == MatchMode::WordBoundary
                        {
                            continue;
                        }
                        if let Some(result) = self.take_match(config_index, length, None) {
                            let expansion_bytes = trigger.len().saturating_add(result.insert.len());
                            if result_bytes.saturating_add(expansion_bytes)
                                > MAX_RESULT_BYTES_PER_EVENT
                            {
                                self.clear_buffer();
                                break;
                            }
                            result_bytes = result_bytes.saturating_add(expansion_bytes);
                            results.push(result);
                        }
                        // Do not allow a replacement to combine with the
                        // next typed text and accidentally trigger again.
                        self.clear_buffer();
                    }
                }
                results
            }
            InputEvent::Delimiter(character) => {
                self.process_internal(InputEvent::Text(character.to_string()))
            }
            InputEvent::Backspace => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.buffer.pop_back();
                Vec::new()
            }
            InputEvent::EndOfInput => {
                self.input_generation = self.input_generation.wrapping_add(1);
                let result = self
                    .matcher
                    .find_suffix(self.buffer.iter().rev().copied())
                    .and_then(|(index, length)| {
                        self.matcher_indices
                            .get(index)
                            .copied()
                            .and_then(|config_index| self.take_match(config_index, length, None))
                    });
                self.clear_buffer();
                result.into_iter().collect()
            }
            InputEvent::Reset => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.clear_buffer();
                Vec::new()
            }
            InputEvent::FocusChanged { sensitive } => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.sensitive_focus = sensitive;
                self.clear_buffer();
                Vec::new()
            }
            InputEvent::PauseChanged(paused) => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.user_paused = paused;
                self.clear_buffer();
                Vec::new()
            }
            InputEvent::WindowChanged(window) => {
                self.input_generation = self.input_generation.wrapping_add(1);
                // The text in the buffer belongs to the previously-focused
                // application. Text expansion state must be scoped to the
                // focused window, not to the desktop session. Even if
                // app_filter would correctly re-evaluate against the new
                // window, the physical characters in the buffer are from the
                // old application and must not be used to compute replacements
                // for the new one. This prevents cross-window trigger matches
                // that can cause unrelated text deletion.
                self.set_window_context(window);
                self.clear_buffer();
                Vec::new()
            }
        }
    }

    /// Process input and return pending expansion results for deferred execution.
    /// Commands are NOT executed; caller must check policy and complete results
    /// with `dispatch_pending_with_policy()` to preserve engine state.
    pub(super) fn process_deferred_internal(
        &mut self,
        event: InputEvent,
    ) -> Vec<PendingExpansionResult> {
        self.restore_deferred_matches();
        // Same undo validity rules as process(): only the undo chord preserves undo.
        match &event {
            InputEvent::Key(chord) if !self.is_undo_chord(chord) => {
                self.last_expansion = None;
            }
            InputEvent::Key(_) => {
                // Undo chord preserves last_expansion (if any)
            }
            _ => {
                self.last_expansion = None;
            }
        }
        match event {
            InputEvent::Key(_) => {
                self.note_key_event();
                Vec::new()
            }
            InputEvent::Text(text) => {
                let mut results = Vec::new();
                let mut result_bytes = 0usize;
                if !self.is_capture_enabled() {
                    return results;
                }
                for character in text.chars() {
                    // Match the immediate processor's undo semantics: once
                    // another scalar follows a match in the same event, that
                    // earlier expansion is no longer immediately undoable.
                    if !results.is_empty() {
                        for result in &mut results {
                            result.undoable = false;
                        }
                    }
                    self.input_generation = self.input_generation.wrapping_add(1);
                    let pending = self.matcher.find_suffix(self.buffer.iter().rev().copied());
                    if let Some((index, length)) = pending {
                        if let Some(config_index) = self.matcher_indices.get(index).copied() {
                            let match_mode = self.config.expansion[config_index].match_mode;
                            let typed: String = {
                                let start = self.buffer.len().saturating_sub(length);
                                self.buffer.iter().skip(start).collect()
                            };
                            if !self.matcher.can_continue(&typed, character) {
                                let trailing_word_character = match_mode == MatchMode::WordBoundary
                                    && matching::is_word_character(character);
                                if !trailing_word_character {
                                    if let Some(result) = self.take_match_deferred(
                                        config_index,
                                        length,
                                        Some(character),
                                    ) {
                                        let bytes = result
                                            .trigger
                                            .len()
                                            .saturating_add(result.template_text.len());
                                        if results.len() >= MAX_RESULTS_PER_EVENT
                                            || result_bytes.saturating_add(bytes)
                                                > MAX_RESULT_BYTES_PER_EVENT
                                        {
                                            self.clear_buffer();
                                            break;
                                        }
                                        result_bytes = result_bytes.saturating_add(bytes);
                                        results.push(result);
                                    }
                                }
                            }
                        }
                    }
                    if results.len() >= MAX_RESULTS_PER_EVENT {
                        self.clear_buffer();
                        break;
                    }
                    self.buffer.push_back(character);
                    while self.buffer.len() > self.max_buffer_chars {
                        self.buffer.pop_front();
                        self.buffer_truncated = true;
                    }
                    if let Some((index, length)) =
                        self.matcher.find_suffix(self.buffer.iter().rev().copied())
                    {
                        let config_index = self.matcher_indices.get(index).copied();
                        let Some(config_index) = config_index else {
                            self.clear_buffer();
                            continue;
                        };
                        let (trigger, match_mode) = {
                            let expansion = &self.config.expansion[config_index];
                            (expansion.trigger.clone(), expansion.match_mode)
                        };
                        let typed: String = {
                            let start = self.buffer.len().saturating_sub(length);
                            self.buffer.iter().skip(start).collect()
                        };
                        if self.matcher.has_continuation(&typed)
                            || match_mode == MatchMode::WordBoundary
                        {
                            continue;
                        }
                        if let Some(result) = self.take_match_deferred(config_index, length, None) {
                            let expansion_bytes =
                                trigger.len().saturating_add(result.template_text.len());
                            if result_bytes.saturating_add(expansion_bytes)
                                > MAX_RESULT_BYTES_PER_EVENT
                            {
                                self.clear_buffer();
                                break;
                            }
                            result_bytes = result_bytes.saturating_add(expansion_bytes);
                            results.push(result);
                        }
                        self.clear_buffer();
                    }
                }
                results
            }
            InputEvent::Delimiter(character) => {
                self.process_deferred_internal(InputEvent::Text(character.to_string()))
            }
            InputEvent::Backspace => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.buffer.pop_back();
                Vec::new()
            }
            InputEvent::EndOfInput => {
                self.input_generation = self.input_generation.wrapping_add(1);
                let result = self
                    .matcher
                    .find_suffix(self.buffer.iter().rev().copied())
                    .and_then(|(index, length)| {
                        self.matcher_indices
                            .get(index)
                            .copied()
                            .and_then(|config_index| {
                                self.take_match_deferred(config_index, length, None)
                            })
                    });
                self.clear_buffer();
                result
                    .map(|mut pending| {
                        pending.generation = self.input_generation;
                        pending
                    })
                    .into_iter()
                    .collect()
            }
            InputEvent::Reset => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.clear_buffer();
                Vec::new()
            }
            InputEvent::FocusChanged { sensitive } => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.sensitive_focus = sensitive;
                self.clear_buffer();
                Vec::new()
            }
            InputEvent::PauseChanged(paused) => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.user_paused = paused;
                self.clear_buffer();
                Vec::new()
            }
            InputEvent::WindowChanged(window) => {
                self.input_generation = self.input_generation.wrapping_add(1);
                self.set_window_context(window);
                self.clear_buffer();
                Vec::new()
            }
        }
    }
}
