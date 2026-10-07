//! Pure text and replacement helpers for the protocol-neutral IBus adapter.

use wayexpand_core::ExpansionResult;

use super::IbusAction;

/// The client's text around the cursor as last reported through
/// `SetSurroundingText`, advanced locally by the edits this engine emits.
/// Positions are in Unicode scalar values, as IBus reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SurroundingText {
    text: Vec<char>,
    cursor: usize,
    anchor: usize,
}

impl SurroundingText {
    pub(super) fn new(text: Vec<char>, cursor: usize, anchor: usize) -> Self {
        Self {
            text,
            cursor,
            anchor,
        }
    }

    /// Whether `expected` is exactly the text before a collapsed cursor.
    pub(super) fn ends_with_at_cursor(&self, expected: &str) -> bool {
        if self.cursor != self.anchor || self.cursor > self.text.len() {
            return false;
        }
        let expected: Vec<char> = expected.chars().collect();
        self.text[..self.cursor].ends_with(&expected)
    }

    /// Apply an emitted action. Returns false when the model can no longer
    /// describe the client, so the caller drops it.
    pub(super) fn apply(&mut self, action: &IbusAction) -> bool {
        if self.cursor != self.anchor || self.cursor > self.text.len() {
            return false;
        }
        match action {
            IbusAction::DeleteSurroundingText { nchars } => {
                let count = *nchars as usize;
                let Some(start) = self.cursor.checked_sub(count) else {
                    return false;
                };
                self.text.drain(start..self.cursor);
                self.cursor = start;
            }
            IbusAction::CommitText(text) => {
                let inserted: Vec<char> = text.chars().collect();
                let count = inserted.len();
                self.text.splice(self.cursor..self.cursor, inserted);
                self.cursor += count;
            }
        }
        self.anchor = self.cursor;
        true
    }
}

/// Plan a replacement: the text already in the client document that must be
/// deleted, and the delete/commit actions that replace it.
///
/// `key_delivered` says whether the key that completed the match has reached
/// the client. A synchronous match consumes that key, so an immediate
/// trigger's final character was never delivered and a boundary match's
/// delimiter is carried by the replacement instead. An asynchronous match
/// committed the key when the command was queued, so the whole trigger and
/// any delimiter are in the document.
pub(super) fn replacement_actions(
    result: &ExpansionResult,
    key_delivered: bool,
) -> (String, Vec<IbusAction>) {
    let mut delivered = result.matched_text.clone();
    match (key_delivered, result.reinsert_after) {
        (true, Some(delimiter)) => delivered.push(delimiter),
        (false, None) => {
            delivered.pop();
        }
        _ => {}
    }
    let mut actions = Vec::with_capacity(2);
    let delete_chars = delivered.chars().count();
    if delete_chars > 0 {
        actions.push(IbusAction::DeleteSurroundingText {
            nchars: delete_chars as u32,
        });
    }
    let mut replacement = result.insert.clone();
    if let Some(character) = result.reinsert_after {
        replacement.push(character);
    }
    actions.push(IbusAction::CommitText(replacement));
    (delivered, actions)
}

/// Convert the XKB keysym that IBus supplies to the character it types.
/// Layout, dead-key and Compose resolution has already happened before IBus
/// receives the event. Conversion follows xkbcommon's
/// `xkb_keysym_to_utf32` (via `xkeysym`), so legacy keysyms and keypad digits
/// resolve like explicit Unicode keysyms. Control characters other than tab
/// and newline are not text.
pub(super) fn keysym_to_char(keysym: u32) -> Option<char> {
    match keysym {
        xkeysym::key::Tab | xkeysym::key::KP_Tab => Some('\t'),
        xkeysym::key::Return | xkeysym::key::KP_Enter => Some('\n'),
        _ => xkeysym::Keysym::new(keysym)
            .key_char()
            .filter(|character| !character.is_control()),
    }
}
