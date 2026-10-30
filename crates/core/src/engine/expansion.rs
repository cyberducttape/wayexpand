//! Input event processing and expansion computation.
//!
//! Processes input events (text, keys, delimiters, window changes) through the
//! expansion engine, managing buffer state, window context, and pause/focus state.

use super::matching;
use super::{ExpansionEngine, ExpansionResult, InputEvent, PendingExpansionResult};
use super::{MAX_RESULTS_PER_EVENT, MAX_RESULT_BYTES_PER_EVENT};
use crate::MatchMode;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

const GRAPHEME_CHUNK_CHARS: usize = 64;

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
        let mut results = Vec::new();
        self.process_into(event, &mut results);
        results
    }

    /// Process an event into caller-owned storage, clearing previous results
    /// while retaining the vector's allocation for reuse.
    pub fn process_into(&mut self, event: InputEvent, results: &mut Vec<ExpansionResult>) {
        self.process_internal_into(event, results);
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
    }

    /// Process borrowed text as one text event into reusable result storage.
    /// Matching still advances once per Unicode scalar value.
    pub fn process_text_into(&mut self, text: &str, results: &mut Vec<ExpansionResult>) {
        self.process_text_event_into::<ImmediateProcessor>(text, results);
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
    }

    /// Process one already-decoded character without constructing a temporary
    /// `String`, retaining the caller's immediate-result buffer.
    pub fn process_text_char_into(&mut self, character: char, results: &mut Vec<ExpansionResult>) {
        let mut encoded = [0; 4];
        self.process_text_into(character.encode_utf8(&mut encoded), results);
    }

    /// Process input while reserving command-backed matches for caller policy
    /// approval and later dispatch.
    pub fn process_deferred(&mut self, event: InputEvent) -> Vec<PendingExpansionResult> {
        let mut results = Vec::new();
        self.process_deferred_into(event, &mut results);
        results
    }

    /// Process an event into reusable pending-result storage. Results own
    /// their dispatch data and can safely outlive this call or a later reload.
    pub fn process_deferred_into(
        &mut self,
        event: InputEvent,
        results: &mut Vec<PendingExpansionResult>,
    ) {
        self.process_deferred_internal_into(event, results);
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
    }

    /// Process one decoded character through deferred matching without an
    /// intermediate event `String`. Deferred results remain fully owned.
    pub fn process_deferred_char_into(
        &mut self,
        character: char,
        results: &mut Vec<PendingExpansionResult>,
    ) {
        let mut encoded = [0; 4];
        self.process_text_event_into::<DeferredProcessor>(
            character.encode_utf8(&mut encoded),
            results,
        );
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
    }

    /// Convenience wrapper for callers that process a single decoded
    /// character and do not retain result storage between events.
    pub fn process_deferred_char(&mut self, character: char) -> Vec<PendingExpansionResult> {
        let mut results = Vec::new();
        self.process_deferred_char_into(character, &mut results);
        results
    }

    /// Clear matcher history when the preceding document text is already
    /// present but was not observed as user input. Word-boundary matches then
    /// remain conservative until a real boundary is typed.
    pub fn reset_with_unknown_preceding_text(&mut self) {
        self.on_reset();
        self.buffer_truncated = true;
    }
}

/// The policy-specific part of taking a match. The event transitions and
/// character scanner below are shared; only match execution and undo metadata
/// differ between synchronous and deferred callers.
trait MatchProcessor {
    type Output;

    fn before_event(engine: &mut ExpansionEngine);
    fn before_character(engine: &mut ExpansionEngine);
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

    fn before_character(_: &mut ExpansionEngine) {}

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

    fn before_character(engine: &mut ExpansionEngine) {
        engine.restore_stale_command_reservations();
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
        let mut trailing = self.buffer.iter().rev();
        let scalar_count = match (trailing.next(), trailing.next()) {
            (None, _) => 0,
            // Two ASCII scalars form one cluster only as CR LF, so the common
            // case needs no segmentation of the whole buffer.
            (Some(last), previous)
                if last.is_ascii()
                    && previous.is_none_or(|previous| {
                        previous.is_ascii() && !(*previous == '\r' && *last == '\n')
                    }) =>
            {
                1
            }
            _ => self.trailing_grapheme_scalar_count(),
        };
        for _ in 0..scalar_count {
            self.pop_buffered_back();
        }
    }

    /// Find the scalar count of the final extended grapheme cluster from a
    /// bounded suffix. Fall back to the complete retained buffer only when the
    /// grapheme cursor needs earlier context or the cluster crosses the suffix
    /// boundary. This keeps ordinary Unicode backspaces independent of input
    /// history length without guessing at RI, Indic, or emoji boundaries.
    fn trailing_grapheme_scalar_count(&self) -> usize {
        let len = self.buffer.len();
        if len == 0 {
            return 0;
        }

        let chunk_start = len.saturating_sub(GRAPHEME_CHUNK_CHARS);
        let chunk_start_byte = self.buffered_bytes - self.buffer_range_byte_len(chunk_start, len);
        let chunk = self.buffer_range_string(chunk_start, len);
        let mut cursor = GraphemeCursor::new(self.buffered_bytes, self.buffered_bytes, true);
        if let Ok(Some(boundary)) = cursor.prev_boundary(&chunk, chunk_start_byte) {
            if boundary >= chunk_start_byte {
                return self.buffered_scalars_after_byte(boundary);
            }
        }
        // Any request for context or a preceding chunk means the suffix alone
        // is insufficient. This uncommon path favors exact deletion over a
        // guessed scalar or grapheme boundary.
        let buffered: String = self.buffer.iter().collect();
        buffered
            .graphemes(true)
            .next_back()
            .map_or(0, |grapheme| grapheme.chars().count())
    }

    fn buffer_range_string(&self, start: usize, end: usize) -> String {
        let mut text = String::with_capacity(self.buffer_range_byte_len(start, end));
        for index in start..end {
            text.push(self.buffer[index]);
        }
        text
    }

    fn buffer_range_byte_len(&self, start: usize, end: usize) -> usize {
        (start..end)
            .map(|index| self.buffer[index].len_utf8())
            .sum()
    }

    fn buffered_scalars_after_byte(&self, byte_offset: usize) -> usize {
        let mut bytes = self.buffered_bytes;
        let mut scalars = 0;
        for character in self.buffer.iter().rev() {
            if bytes <= byte_offset {
                break;
            }
            bytes -= character.len_utf8();
            scalars += 1;
        }
        scalars
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
        let suffix = self
            .find_match_suffix()
            .filter(|suffix| !self.suffix_is_restored_reservation(suffix.raw_length))?;
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
    /// raw scalar length. NFC input takes the allocation-free trie path; other
    /// buffers need a normalized view of the trailing graphemes and a map
    /// back to their source positions.
    fn find_match_suffix(&self) -> Option<MatchSuffix> {
        // The quick check proves almost all text is NFC without composing
        // it. `No` and `Maybe` take the bounded normalized path below, which
        // is also correct for text that turns out to be NFC already.
        if self.non_ascii_buffer_chars == 0
            || unicode_normalization::is_nfc_quick(self.buffer.iter().copied())
                == unicode_normalization::IsNormalized::Yes
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

        // Normalize whole graphemes from the end, and only as many as the
        // longest trigger can consume; earlier text cannot affect the match.
        // Reverse segmentation still sees the full buffer as context.
        let raw: String = self.buffer.iter().collect();
        let limit = self.matcher.max_trigger_chars();
        let mut normalized = Vec::new();
        let mut source_starts = Vec::new();
        let mut raw_end = self.buffer.len();
        for grapheme in raw.graphemes(true).rev() {
            if normalized.len() >= limit {
                break;
            }
            let raw_start = raw_end - grapheme.chars().count();
            let cluster_start = normalized.len();
            normalized.extend(grapheme.nfc());
            normalized[cluster_start..].reverse();
            source_starts.resize(normalized.len(), raw_start);
            raw_end = raw_start;
        }
        normalized.reverse();
        source_starts.reverse();
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
        if same_focus_target(self.current_window.as_ref(), window.as_ref()) {
            // Trackers republish unchanged focus: a tracker that keeps
            // failing to connect reports "no window" on every retry, and an
            // application retitles its window while the user types. Neither
            // moves focus (the focus witness does not count them either), so
            // keep the typed context and pending work and only refresh the
            // title for `title_contains:` filters.
            self.set_window_context(window);
            return;
        }
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

/// Whether two focus snapshots certainly name the same target: both "no
/// window", or the same application and the same exact window instance.
/// Anything less certain (no instance ID to compare) counts as a change.
fn same_focus_target(
    current: Option<&super::WindowContext>,
    update: Option<&super::WindowContext>,
) -> bool {
    match (current, update) {
        (None, None) => true,
        (Some(current), Some(update)) => {
            current.app_id == update.app_id
                && current
                    .instance_id
                    .as_deref()
                    .is_some_and(|instance| !instance.is_empty())
                && current.instance_id == update.instance_id
        }
        _ => false,
    }
}

impl ExpansionEngine {
    pub(super) fn process_internal_into(
        &mut self,
        event: InputEvent,
        results: &mut Vec<ExpansionResult>,
    ) {
        self.process_event_into::<ImmediateProcessor>(event, results);
    }

    pub(super) fn process_deferred_internal_into(
        &mut self,
        event: InputEvent,
        results: &mut Vec<PendingExpansionResult>,
    ) {
        self.process_event_into::<DeferredProcessor>(event, results);
    }

    fn process_event_into<P: MatchProcessor>(
        &mut self,
        event: InputEvent,
        results: &mut Vec<P::Output>,
    ) {
        results.clear();
        P::before_event(self);
        self.invalidate_undo_unless_undo_chord(&event);
        match event {
            InputEvent::Key(_) => {
                self.note_key_event();
            }
            InputEvent::TextChar(character) => {
                let mut encoded = [0; 4];
                self.process_text_contents::<P>(character.encode_utf8(&mut encoded), results)
            }
            InputEvent::Text(text) => self.process_text_contents::<P>(&text, results),
            InputEvent::Delimiter(character) => {
                // Encode into a stack buffer and pass the borrowed text to
                // the common scanner, avoiding a one-character String and a
                // recursive event dispatch.
                let mut encoded = [0; 4];
                self.process_text_contents::<P>(character.encode_utf8(&mut encoded), results)
            }
            InputEvent::Backspace => {
                self.on_backspace();
            }
            InputEvent::EndOfInput => {
                self.bump_generation();
                let result =
                    self.find_match_suffix()
                        .filter(|suffix| !self.suffix_is_restored_reservation(suffix.raw_length))
                        .and_then(|suffix| {
                            self.matcher_indices.get(suffix.index).copied().and_then(
                                |config_index| P::take(self, config_index, suffix.raw_length, None),
                            )
                        });
                self.clear_buffer();
                if let Some(output) = result {
                    results.push(P::finish_end_of_input(self, output));
                }
            }
            InputEvent::Reset => {
                self.on_reset();
            }
            InputEvent::FocusChanged { sensitive } => {
                self.on_focus_changed(sensitive);
            }
            InputEvent::CompositionChanged { active } => {
                self.on_composition_changed(active);
            }
            InputEvent::PauseChanged(paused) => {
                self.on_pause_changed(paused);
            }
            InputEvent::WindowChanged(window) => {
                self.on_window_changed(window);
            }
        }
    }

    fn process_text_event_into<P: MatchProcessor>(
        &mut self,
        text: &str,
        results: &mut Vec<P::Output>,
    ) {
        results.clear();
        P::before_event(self);
        self.last_expansion = None;
        self.process_text_contents::<P>(text, results);
    }

    fn process_text_contents<P: MatchProcessor>(
        &mut self,
        text: &str,
        results: &mut Vec<P::Output>,
    ) {
        let mut result_bytes = 0usize;
        if !self.is_capture_enabled() {
            return;
        }
        for (index, character) in text.chars().enumerate() {
            if index > 0 {
                P::before_character(self);
            }
            // A later scalar in the same text event invalidates undo for
            // earlier matches, with policy-specific metadata handled here.
            if !results.is_empty() {
                P::invalidate_outputs(self, results);
            }
            self.bump_generation();
            if let Some((config_index, length)) = self.match_completed_by(character) {
                if let Some(output) = P::take(self, config_index, length, Some(character)) {
                    let bytes = P::output_bytes(&output);
                    if results.len() >= MAX_RESULTS_PER_EVENT
                        || result_bytes.saturating_add(bytes) > MAX_RESULT_BYTES_PER_EVENT
                    {
                        self.discard_after_result_limit();
                        break;
                    }
                    result_bytes = result_bytes.saturating_add(bytes);
                    results.push(output);
                }
            }
            if results.len() >= MAX_RESULTS_PER_EVENT {
                self.discard_after_result_limit();
                break;
            }
            self.push_buffered(character);
            match self.settled_match() {
                SettledMatch::Continue => self.maybe_prefetch_clipboard(),
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
                            self.discard_after_result_limit();
                            break;
                        }
                        result_bytes = result_bytes.saturating_add(bytes);
                        results.push(output);
                    }
                    // Do not allow a replacement to combine with following
                    // input and accidentally trigger again.
                    self.consume_matched_text();
                }
            }
        }
    }

    /// Stop matching for the rest of an event that exceeded its result
    /// budget. The skipped text is still in the document but must not
    /// complete a trigger later, so it is dropped like evicted context: a
    /// word-boundary trigger typed next cannot match as if it began the input.
    fn discard_after_result_limit(&mut self) {
        self.clear_buffer();
        self.buffer_truncated = true;
    }
}
