//! Input event processing and expansion computation.
//!
//! Processes input events (text, keys, delimiters, window changes) through the
//! expansion engine, managing buffer state, window context, and pause/focus state.

use super::matching;
use super::{ExpansionEngine, ExpansionResult, InputEvent, PendingExpansionResult};
use super::{MAX_RESULTS_PER_EVENT, MAX_RESULT_BYTES_PER_EVENT};
use crate::MatchMode;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

struct MatchSuffix {
    index: usize,
    raw_length: usize,
    normalized: Option<Vec<char>>,
    normalized_length: usize,
}

impl ExpansionEngine {
    /// Process an input event through the expansion state machine.
    ///
    /// The public entry point lives in this module so event processing remains
    /// separated from matching policy, transaction handling, and command
    /// runtime ownership as those implementations continue to be extracted.
    pub fn process(&mut self, event: InputEvent) -> Vec<ExpansionResult> {
        let results = self.process_internal(event);
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
        results
    }

    /// Process input while reserving command-backed matches for caller policy
    /// approval and later dispatch.
    pub fn process_deferred(&mut self, event: InputEvent) -> Vec<PendingExpansionResult> {
        let results = self.process_deferred_internal(event);
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
        results
    }
}

/// The policy-specific part of taking a match. The event transitions and
/// character scanner below are shared; only match execution and undo metadata
/// differ between synchronous and deferred callers.
trait MatchProcessor {
    type Output;

    fn before_event(engine: &mut ExpansionEngine);
    fn take(
        engine: &mut ExpansionEngine,
        config_index: usize,
        length: usize,
        terminator: Option<char>,
    ) -> Option<Self::Output>;
    fn output_bytes(output: &Self::Output) -> usize;
    fn invalidate_outputs(engine: &mut ExpansionEngine, outputs: &mut [Self::Output]);
    fn finish_end_of_input(engine: &ExpansionEngine, output: Self::Output) -> Self::Output;
}

struct ImmediateProcessor;
struct DeferredProcessor;

impl MatchProcessor for ImmediateProcessor {
    type Output = ExpansionResult;

    fn before_event(_: &mut ExpansionEngine) {}

    fn take(
        engine: &mut ExpansionEngine,
        config_index: usize,
        length: usize,
        terminator: Option<char>,
    ) -> Option<Self::Output> {
        engine.take_match(config_index, length, terminator)
    }

    fn output_bytes(output: &Self::Output) -> usize {
        output.trigger.len().saturating_add(output.insert.len())
    }

    fn invalidate_outputs(engine: &mut ExpansionEngine, _: &mut [Self::Output]) {
        engine.last_expansion = None;
    }

    fn finish_end_of_input(_: &ExpansionEngine, output: Self::Output) -> Self::Output {
        output
    }
}

impl MatchProcessor for DeferredProcessor {
    type Output = PendingExpansionResult;

    fn before_event(engine: &mut ExpansionEngine) {
        engine.restore_deferred_matches();
    }

    fn take(
        engine: &mut ExpansionEngine,
        config_index: usize,
        length: usize,
        terminator: Option<char>,
    ) -> Option<Self::Output> {
        engine.take_match_deferred(config_index, length, terminator)
    }

    fn output_bytes(output: &Self::Output) -> usize {
        output
            .trigger
            .len()
            .saturating_add(output.template_text.len())
    }

    fn invalidate_outputs(_: &mut ExpansionEngine, outputs: &mut [Self::Output]) {
        for output in outputs {
            output.undoable = false;
        }
    }

    fn finish_end_of_input(engine: &ExpansionEngine, mut output: Self::Output) -> Self::Output {
        output.generation = engine.input_generation;
        output
    }
}

/// What the buffer's current suffix means once a character has been appended.
enum SettledMatch {
    /// Keep buffering: nothing matched, a longer trigger could still follow,
    /// or a word-boundary trigger is still waiting for its boundary.
    Continue,
    /// The matcher pointed at an expansion that the current configuration no
    /// longer has. Drop the buffered text instead of matching it.
    Stale,
    /// Take this match, then clear the buffer.
    Take { config_index: usize, length: usize },
}

/// Event handling shared by the immediate and deferred processors.
///
/// These two processors differ only in how a match is taken and reported;
/// every event that merely advances engine state behaves identically in both.
/// Keeping those bodies in one place is not tidiness -- a fix applied to one
/// copy and missed in the other is a silent behavioural split between the
/// immediate path and the deferred path the daemon actually runs.
impl ExpansionEngine {
    fn bump_generation(&mut self) {
        self.input_generation = self.input_generation.wrapping_add(1);
        self.shared_input_generation
            .store(self.input_generation, std::sync::atomic::Ordering::Release);
    }

    /// A pending undo is valid only immediately after the expansion it would
    /// revert, with no other event in between. Undo survives only if the key
    /// event is the undo chord itself; any other key (navigation, text,
    /// application shortcuts, ...) invalidates it because the cursor position
    /// may have changed.
    fn invalidate_undo_unless_undo_chord(&mut self, event: &InputEvent) {
        if matches!(event, InputEvent::Key(chord) if self.is_undo_chord(chord)) {
            return;
        }
        self.last_expansion = None;
    }

    fn on_backspace(&mut self) {
        self.bump_generation();
        // All active input paths forward one Backspace per deleted grapheme
        // cluster (the input-method route derives its byte deletion from
        // surrounding text). Keep matcher state aligned with the document,
        // including decomposed accents and emoji sequences.
        let buffered: String = self.buffer.iter().collect();
        let scalar_count = buffered
            .graphemes(true)
            .next_back()
            .map_or(0, |grapheme| grapheme.chars().count());
        for _ in 0..scalar_count {
            self.buffer.pop_back();
        }
    }

    fn on_reset(&mut self) {
        self.bump_generation();
        self.clear_buffer();
    }

    fn on_focus_changed(&mut self, sensitive: bool) {
        self.bump_generation();
        self.sensitive_focus = sensitive;
        self.clear_buffer();
    }

    fn on_composition_changed(&mut self, active: bool) {
        self.bump_generation();
        self.composition_active = active;
        self.clear_buffer();
    }

    fn on_pause_changed(&mut self, paused: bool) {
        self.bump_generation();
        self.user_paused = paused;
        self.clear_buffer();
    }

    /// Whether the suffix already in the buffer must be taken as a match now
    /// that `character` has been typed but not yet appended.
    ///
    /// This is the "a longer trigger could still have followed, but did not"
    /// case: `:a` matches only once the next character rules out `:address`.
    /// Returns the configured expansion and the matched suffix length.
    fn match_completed_by(&self, character: char) -> Option<(usize, usize)> {
        let suffix = self.find_match_suffix()?;
        let config_index = self.matcher_indices.get(suffix.index).copied()?;
        // The configured trigger is the lowercase form. With propagate_case
        // the pending suffix may instead be `:A` or `:AB`, so continuation is
        // checked against the text the user actually typed -- that is the
        // string present in the forward trie, not the generated sibling. The
        // suffix is read straight out of the rolling buffer; collecting it
        // into a `String` first cost one allocation per keystroke.
        let can_continue = if let Some(normalized) = &suffix.normalized {
            self.matcher.can_continue(
                normalized
                    .iter()
                    .skip(normalized.len().saturating_sub(suffix.normalized_length))
                    .copied(),
                character,
            )
        } else {
            self.matcher.can_continue(
                self.buffer
                    .iter()
                    .skip(self.buffer.len().saturating_sub(suffix.raw_length))
                    .copied(),
                character,
            )
        };
        if can_continue {
            return None;
        }
        let match_mode = self.config.expansion[config_index].match_mode;
        if match_mode == MatchMode::WordBoundary
            && matching::continues_word_after(self.buffer.back().copied(), character)
        {
            return None;
        }
        Some((config_index, suffix.raw_length))
    }

    /// Whether the buffer, with `character` already appended, now ends in a
    /// complete trigger that nothing can extend.
    fn settled_match(&self) -> SettledMatch {
        let Some(suffix) = self.find_match_suffix() else {
            return SettledMatch::Continue;
        };
        let Some(config_index) = self.matcher_indices.get(suffix.index).copied() else {
            return SettledMatch::Stale;
        };
        let has_continuation = if let Some(normalized) = &suffix.normalized {
            self.matcher.has_continuation(
                normalized
                    .iter()
                    .skip(normalized.len().saturating_sub(suffix.normalized_length))
                    .copied(),
            )
        } else {
            self.matcher.has_continuation(
                self.buffer
                    .iter()
                    .skip(self.buffer.len().saturating_sub(suffix.raw_length))
                    .copied(),
            )
        };
        if has_continuation {
            return SettledMatch::Continue;
        }
        if self.config.expansion[config_index].match_mode == MatchMode::WordBoundary {
            // A word-boundary trigger waits for its boundary character, which
            // arrives through `match_completed_by` on a later keystroke.
            return SettledMatch::Continue;
        }
        SettledMatch::Take {
            config_index,
            length: suffix.raw_length,
        }
    }

    /// Find a trigger suffix in NFC form while retaining the corresponding
    /// raw scalar length. NFC input takes the allocation-free trie path; only
    /// non-NFC buffers need a normalized view and grapheme-to-source map.
    fn find_match_suffix(&self) -> Option<MatchSuffix> {
        if self
            .buffer
            .iter()
            .copied()
            .nfc()
            .eq(self.buffer.iter().copied())
        {
            let (index, length) = self
                .matcher
                .find_suffix(self.buffer.iter().rev().copied())?;
            return Some(MatchSuffix {
                index,
                raw_length: length,
                normalized: None,
                normalized_length: length,
            });
        }

        let raw: String = self.buffer.iter().collect();
        let mut normalized = Vec::new();
        let mut source_starts = Vec::new();
        let mut raw_start = 0;
        for grapheme in raw.graphemes(true) {
            let cluster: Vec<char> = grapheme.nfc().collect();
            normalized.extend(cluster.iter().copied());
            source_starts.extend(std::iter::repeat_n(raw_start, cluster.len()));
            raw_start += grapheme.chars().count();
        }
        let (index, length) = self.matcher.find_suffix(normalized.iter().rev().copied())?;
        let normalized_start = normalized.len().checked_sub(length)?;
        let raw_start = *source_starts.get(normalized_start)?;
        Some(MatchSuffix {
            index,
            raw_length: self.buffer.len().saturating_sub(raw_start),
            normalized: Some(normalized),
            normalized_length: length,
        })
    }

    fn on_window_changed(&mut self, window: Option<super::WindowContext>) {
        self.bump_generation();
        // The text in the buffer belongs to the previously-focused
        // application. Text expansion state must be scoped to the focused
        // window, not to the desktop session. Even if app_filter would
        // correctly re-evaluate against the new window, the physical
        // characters in the buffer are from the old application and must not
        // be used to compute replacements for the new one. This prevents
        // cross-window trigger matches that can cause unrelated text deletion.
        self.set_window_context(window);
        self.clear_buffer();
    }
}

impl ExpansionEngine {
    /// Process an event stream. A text event may contain multiple Unicode
    /// scalar values; matching is performed after each one.
    pub(super) fn process_internal(&mut self, event: InputEvent) -> Vec<ExpansionResult> {
        self.process_event::<ImmediateProcessor>(event)
    }

    /// Process input and return pending expansion results for deferred execution.
    /// Commands are NOT executed; caller must check policy and complete results
    /// with `dispatch_pending_with_policy()` to preserve engine state.
    pub(super) fn process_deferred_internal(
        &mut self,
        event: InputEvent,
    ) -> Vec<PendingExpansionResult> {
        self.process_event::<DeferredProcessor>(event)
    }

    fn process_event<P: MatchProcessor>(&mut self, event: InputEvent) -> Vec<P::Output> {
        P::before_event(self);
        self.invalidate_undo_unless_undo_chord(&event);
        match event {
            InputEvent::Key(_) => {
                self.note_key_event();
                Vec::new()
            }
            InputEvent::Text(text) => self.process_text::<P>(&text),
            InputEvent::Delimiter(character) => {
                self.process_event::<P>(InputEvent::Text(character.to_string()))
            }
            InputEvent::Backspace => {
                self.on_backspace();
                Vec::new()
            }
            InputEvent::EndOfInput => {
                self.bump_generation();
                let result = self.find_match_suffix().and_then(|suffix| {
                    self.matcher_indices
                        .get(suffix.index)
                        .copied()
                        .and_then(|config_index| {
                            P::take(self, config_index, suffix.raw_length, None)
                        })
                });
                self.clear_buffer();
                result
                    .map(|output| P::finish_end_of_input(self, output))
                    .into_iter()
                    .collect()
            }
            InputEvent::Reset => {
                self.on_reset();
                Vec::new()
            }
            InputEvent::FocusChanged { sensitive } => {
                self.on_focus_changed(sensitive);
                Vec::new()
            }
            InputEvent::CompositionChanged { active } => {
                self.on_composition_changed(active);
                Vec::new()
            }
            InputEvent::PauseChanged(paused) => {
                self.on_pause_changed(paused);
                Vec::new()
            }
            InputEvent::WindowChanged(window) => {
                self.on_window_changed(window);
                Vec::new()
            }
        }
    }

    fn process_text<P: MatchProcessor>(&mut self, text: &str) -> Vec<P::Output> {
        let mut results = Vec::new();
        let mut result_bytes = 0usize;
        if !self.is_capture_enabled() {
            return results;
        }
        for character in text.chars() {
            // A later scalar in the same text event invalidates undo for
            // earlier matches, with policy-specific metadata handled here.
            if !results.is_empty() {
                P::invalidate_outputs(self, &mut results);
            }
            self.bump_generation();
            if let Some((config_index, length)) = self.match_completed_by(character) {
                if let Some(output) = P::take(self, config_index, length, Some(character)) {
                    let bytes = P::output_bytes(&output);
                    if results.len() >= MAX_RESULTS_PER_EVENT
                        || result_bytes.saturating_add(bytes) > MAX_RESULT_BYTES_PER_EVENT
                    {
                        self.clear_buffer();
                        break;
                    }
                    result_bytes = result_bytes.saturating_add(bytes);
                    results.push(output);
                }
            }
            if results.len() >= MAX_RESULTS_PER_EVENT {
                self.clear_buffer();
                break;
            }
            self.push_buffered(character);
            match self.settled_match() {
                SettledMatch::Continue => {}
                SettledMatch::Stale => {
                    self.clear_buffer();
                    continue;
                }
                SettledMatch::Take {
                    config_index,
                    length,
                } => {
                    if let Some(output) = P::take(self, config_index, length, None) {
                        let bytes = P::output_bytes(&output);
                        if result_bytes.saturating_add(bytes) > MAX_RESULT_BYTES_PER_EVENT {
                            self.clear_buffer();
                            break;
                        }
                        result_bytes = result_bytes.saturating_add(bytes);
                        results.push(output);
                    }
                    // Do not allow a replacement to combine with following
                    // input and accidentally trigger again.
                    self.clear_buffer();
                }
            }
        }
        results
    }
}
