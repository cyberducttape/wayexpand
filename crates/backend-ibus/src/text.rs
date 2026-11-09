//! Pure text and replacement helpers for the protocol-neutral IBus adapter.

use wayexpand_core::limits::MAX_TRIGGER_CHARS;
use wayexpand_core::{ExpansionResult, SensitiveText};

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

// The longest possible delivered text is a 128-scalar trigger plus one
// boundary character. Keeping this tail is sufficient for every replacement
// precondition and local edit while bounding memory per IBus context.
const SURROUNDING_WINDOW_CHARS: usize = MAX_TRIGGER_CHARS + 1;

impl SurroundingText {
    pub(super) fn new(text: &str, cursor: usize, anchor: usize) -> Option<Self> {
        if anchor != cursor {
            // A selection is never a valid replacement target. Do not retain
            // document text just to remember that it must be refused.
            return Some(Self {
                text: Vec::new(),
                cursor: 0,
                anchor: 1,
            });
        }
        // IBus positions count Unicode scalar values, while Rust strings use
        // UTF-8 byte offsets. Small contexts use one simple pass; large ones
        // locate the cursor byte and collect only the bounded Unicode tail.
        let mut preceding: Vec<char>;
        if cursor <= SURROUNDING_WINDOW_CHARS {
            preceding = text.chars().take(cursor).collect();
            if preceding.len() != cursor {
                return None;
            }
        } else {
            let mut scalar_cursor = 0;
            let mut byte_cursor = text.len();
            for (byte, _) in text.char_indices() {
                if scalar_cursor == cursor {
                    byte_cursor = byte;
                    break;
                }
                scalar_cursor += 1;
            }
            if scalar_cursor != cursor {
                return None;
            }
            preceding = text[..byte_cursor]
                .chars()
                .rev()
                .take(SURROUNDING_WINDOW_CHARS)
                .collect();
            preceding.reverse();
        }
        let cursor = preceding.len();
        Some(Self {
            text: preceding,
            cursor,
            anchor: cursor,
        })
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
                let mut inserted: Vec<char> =
                    text.chars().rev().take(SURROUNDING_WINDOW_CHARS).collect();
                inserted.reverse();
                let keep = SURROUNDING_WINDOW_CHARS.saturating_sub(inserted.len());
                let discard = self.cursor.saturating_sub(keep);
                self.text.truncate(self.cursor);
                self.text.drain(..discard);
                self.text.extend(inserted);
                self.cursor = self.text.len();
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
) -> (SensitiveText, Vec<IbusAction>) {
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

#[cfg(test)]
mod surrounding_text_tests {
    use super::{SurroundingText, SURROUNDING_WINDOW_CHARS};
    use crate::IbusAction;
    use wayexpand_core::SensitiveText;

    #[test]
    fn cursor_offsets_remain_unicode_scalar_based() {
        let text = SurroundingText::new("🌍a漢b", 3, 3).unwrap();
        assert!(text.ends_with_at_cursor("a漢"));
        assert!(!text.ends_with_at_cursor("漢b"));
    }

    #[test]
    fn selection_and_out_of_range_cursor_fail_closed() {
        let selected = SurroundingText::new("sample", 4, 2).unwrap();
        assert!(!selected.ends_with_at_cursor("samp"));
        assert!(SurroundingText::new("sample", 7, 7).is_none());
    }

    #[test]
    fn large_documents_retain_only_a_unicode_safe_trigger_window() {
        let document = "漢🙂é".repeat(20_000);
        let cursor = document.chars().count();
        let mut surrounding = SurroundingText::new(&document, cursor, cursor).unwrap();
        assert_eq!(surrounding.text.len(), SURROUNDING_WINDOW_CHARS);
        assert!(surrounding.ends_with_at_cursor("漢🙂é"));
        assert!(!surrounding.ends_with_at_cursor(&"x".repeat(SURROUNDING_WINDOW_CHARS + 1)));

        assert!(surrounding.apply(&IbusAction::DeleteSurroundingText { nchars: 3 }));
        assert!(surrounding.apply(&IbusAction::CommitText(SensitiveText::from("🐈"))));
        assert!(surrounding.ends_with_at_cursor("🐈"));
        assert!(surrounding.text.len() <= SURROUNDING_WINDOW_CHARS);
    }
}
