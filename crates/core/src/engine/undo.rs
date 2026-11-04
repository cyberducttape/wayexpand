//! Undo planning and commit semantics for applied expansions.

use super::{ExpansionEngine, ExpansionResult};
use crate::KeyChord;
use std::sync::Arc;

impl ExpansionEngine {
    /// Prepare the result that would undo the most recent expansion without
    /// consuming its undo record. Call [`Self::commit_undo`] only after the
    /// result has been injected successfully.
    pub fn prepare_undo(&self, chord: &KeyChord) -> Option<ExpansionResult> {
        if !self.is_capture_enabled() || !self.validated.undo_chord.as_ref()?.matches(chord) {
            return None;
        }
        let (restore_text, erase_text) = self.last_expansion.as_ref()?;
        Some(ExpansionResult {
            snippet_id: Arc::from(""),
            trigger: Arc::from(""),
            matched_text: erase_text.clone().into(),
            insert: restore_text.clone().into(),
            cursor_offset: None,
            reinsert_after: None,
            command_backed: false,
            undoable: false,
            folded_suffix_chars: 0,
        })
    }

    /// Commit a previously prepared undo after successful injection. A
    /// mismatched result cannot consume a newer undo record.
    pub fn commit_undo(&mut self, result: &ExpansionResult) {
        if self
            .last_expansion
            .as_ref()
            .is_some_and(|(restore, erase)| {
                restore == result.insert.as_str() && erase == result.matched_text.as_str()
            })
        {
            self.clear_undo_state();
        }
    }

    /// Forget undo state after an output with an unknown application outcome.
    /// Retaining it could make a later undo erase text the user never saw.
    pub fn discard_undo(&mut self) {
        self.clear_undo_state();
    }

    /// Synchronous convenience method for callers that apply the result
    /// immediately. The daemon should bracket injection with prepare/commit.
    pub fn try_undo(&mut self, chord: &KeyChord) -> Option<ExpansionResult> {
        let result = self.prepare_undo(chord)?;
        self.commit_undo(&result);
        Some(result)
    }
}
