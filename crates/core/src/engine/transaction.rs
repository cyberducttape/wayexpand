//! Undo transaction and deferred command state management.
//!
//! Handles undo history, deferred match reservations, and transaction tracking
//! for expansion results that need to maintain undo semantics across async completion.

use super::{ExpansionEngine, ExpansionError, ExpansionResult};
use crate::TextInjector;

/// Return the exact `(restore, erase)` strings for an expansion's undo
/// transaction.
///
/// A non-exclusive word-boundary match has already delivered its terminating
/// character to the application. Since `apply` replaces that character along
/// with the trigger, undo must include it as well.
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
        // Only a match that has to carry a terminating character through needs
        // new strings. The common case borrows the result directly rather than
        // copying the matched text and the whole replacement on every
        // expansion just to hand out references to the copies.
        match result.reinsert_after {
            None => injector.replace(&result.matched_text, &result.insert)?,
            Some(character) => {
                let mut erase =
                    String::with_capacity(result.matched_text.len() + character.len_utf8());
                erase.push_str(&result.matched_text);
                erase.push(character);
                let mut insert = String::with_capacity(result.insert.len() + character.len_utf8());
                insert.push_str(&result.insert);
                insert.push(character);
                injector.replace(&erase, &insert)?;
            }
        }
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
