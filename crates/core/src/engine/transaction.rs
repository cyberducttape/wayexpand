//! Undo transaction and deferred command state management.
//!
//! Handles undo history, deferred match reservations, and transaction tracking
//! for expansion results that need to maintain undo semantics across async completion.

use super::{ExpansionEngine, ExpansionError, ExpansionResult};
use crate::TextInjector;

pub(super) fn transaction_texts(
    restore: &str,
    replacement: &str,
    reinsert_after: Option<char>,
) -> (String, String) {
    let mut restore_text = restore.to_owned();
    let mut replacement_text = replacement.to_owned();
    if let Some(character) = reinsert_after {
        restore_text.push(character);
        replacement_text.push(character);
    }
    (restore_text, replacement_text)
}

impl ExpansionEngine {
    /// Apply an expansion as one backend transaction, including optional
    /// delimiter reinsertion and cursor repositioning.
    pub fn apply<I: TextInjector + ?Sized>(
        injector: &mut I,
        result: &ExpansionResult,
    ) -> Result<(), ExpansionError> {
        let mut erase = result.matched_text.clone();
        let mut insert = result.insert.clone();
        if let Some(character) = result.reinsert_after {
            erase.push(character);
            insert.push(character);
        }
        injector.replace(&erase, &insert)?;
        let trailing_offset = usize::from(result.reinsert_after.is_some());
        if let Some(offset) = result
            .cursor_offset
            .map(|offset| offset.saturating_add(trailing_offset))
            .filter(|offset| *offset > 0)
        {
            let _ = injector.move_cursor_left(offset);
        }
        Ok(())
    }
}
