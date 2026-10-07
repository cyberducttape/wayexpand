use std::{collections::VecDeque, time::Instant};

use crate::Matcher;

/// Triggers whose snippets read the clipboard.
#[derive(Debug, Clone)]
pub(super) struct ClipboardTriggers {
    trie: Matcher,
    /// Every other effective trigger: a prefix that could still become one of
    /// these does not read the clipboard ahead of time.
    others: Matcher,
    /// First characters of those triggers; a cheap filter for start offsets.
    first_chars: Vec<char>,
    /// Longest trigger, in scalars.
    max_chars: usize,
}

impl ClipboardTriggers {
    pub(super) fn new(triggers: Vec<String>, others: Vec<String>) -> Option<Self> {
        if triggers.is_empty() {
            return None;
        }
        let mut first_chars: Vec<char> = triggers.iter().filter_map(|t| t.chars().next()).collect();
        first_chars.sort_unstable();
        first_chars.dedup();
        let max_chars = triggers
            .iter()
            .map(|t| t.chars().count())
            .max()
            .unwrap_or(0);
        Some(Self {
            trie: Matcher::new(triggers),
            others: Matcher::new(others),
            first_chars,
            max_chars,
        })
    }

    /// Whether the end of `buffer` is a proper prefix (at least two
    /// characters, so a lone sigil like `;` never reads the clipboard) that
    /// can only be completed into a clipboard trigger.
    pub(super) fn prefix_pending(&self, buffer: &VecDeque<char>) -> bool {
        let longest = self.max_chars.saturating_sub(1).min(buffer.len());
        (2..=longest).any(|length| {
            let start = buffer.len() - length;
            self.first_chars.binary_search(&buffer[start]).is_ok()
                && self
                    .trie
                    .has_continuation(buffer.iter().skip(start).copied())
                && !self
                    .others
                    .has_continuation(buffer.iter().skip(start).copied())
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct CommandCacheEntry {
    pub(super) expires_at: Instant,
    pub(super) value: String,
}
