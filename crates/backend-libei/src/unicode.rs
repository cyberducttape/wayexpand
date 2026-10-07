use super::{LibeiError, EI_TEXT_MAX_UTF8_BYTES, MAX_TEXT_BYTES};
use unicode_segmentation::UnicodeSegmentation;

pub(super) fn erase_grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

pub(super) fn split_text_chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = start;
        for (offset, character) in text[start..].char_indices() {
            let candidate = start + offset + character.len_utf8();
            if candidate - start > EI_TEXT_MAX_UTF8_BYTES {
                break;
            }
            end = candidate;
        }
        debug_assert!(end > start, "a Unicode scalar must fit in an EI text chunk");
        chunks.push(&text[start..end]);
        start = end;
    }
    chunks
}

pub(super) fn validate_text(text: &str) -> Result<(), LibeiError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(LibeiError::TextTooLarge {
            length: text.len(),
            maximum: MAX_TEXT_BYTES,
        });
    }
    if let Some(character) = text
        .chars()
        .find(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(LibeiError::ControlCharacter(character as u32));
    }
    Ok(())
}
