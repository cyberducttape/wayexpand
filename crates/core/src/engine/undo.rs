//! Undo planning and commit semantics for applied expansions.

use super::{ExpansionEngine, ExpansionResult};
use crate::KeyChord;

impl ExpansionEngine {
    /// Prepare the result that would undo the most recent expansion without
    /// consuming its undo record. Call [`Self::commit_undo`] only after the
    /// result has been injected successfully.
    pub fn prepare_undo(&self, chord: &KeyChord) -> Option<ExpansionResult> {
        if !self.is_capture_enabled() || !self.undo_chord.as_ref()?.matches(chord) {
            return None;
        }
        let (restore_text, erase_text) = self.last_expansion.as_ref()?;
        Some(ExpansionResult {
            snippet_id: String::new(),
            trigger: String::new(),
            matched_text: erase_text.clone(),
            insert: restore_text.clone(),
            cursor_offset: None,
            reinsert_after: None,
            command_backed: false,
            undoable: false,
        })
    }

    /// Commit a previously prepared undo after successful injection. A
    /// mismatched result cannot consume a newer undo record.
    pub fn commit_undo(&mut self, result: &ExpansionResult) {
        if self
            .last_expansion
            .as_ref()
            .is_some_and(|(restore, erase)| {
                restore == &result.insert && erase == &result.matched_text
            })
        {
            self.last_expansion = None;
        }
    }

    /// Synchronous convenience method for callers that apply the result
    /// immediately. The daemon should bracket injection with prepare/commit.
    pub fn try_undo(&mut self, chord: &KeyChord) -> Option<ExpansionResult> {
        let result = self.prepare_undo(chord)?;
        self.commit_undo(&result);
        Some(result)
    }
}
