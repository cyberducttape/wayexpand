//! Trigger matching and expansion planning.
//!
//! Handles pattern matching against the input buffer, match plan generation,
//! and policy application for deciding whether and how to execute expansions.

use std::sync::Arc;

use super::{ExpansionEngine, MatchPlan};
use crate::config::capitalize_first_letter;
use crate::{render_template_with_cursor, AppFilter, ExpansionConfig, MatchMode};

#[derive(Debug, Clone)]
pub(super) struct GlobPattern {
    tokens: Vec<GlobToken>,
}

#[derive(Debug, Clone, Copy)]
enum GlobToken {
    Literal(char),
    Any,
    Star,
}

impl GlobPattern {
    pub(super) fn compile(pattern: &str) -> Self {
        Self {
            tokens: pattern
                .chars()
                .map(|character| match character {
                    '?' => GlobToken::Any,
                    '*' => GlobToken::Star,
                    character => GlobToken::Literal(character),
                })
                .collect(),
        }
    }

    fn matches(&self, value: &str) -> bool {
        // Match Unicode scalar values rather than UTF-8 bytes. In particular,
        // `?` consumes one Rust `char` (Unicode scalar value), not one byte or
        // one grapheme cluster. `Chars` is clonable, so wildcard backtracking
        // can retain a position without allocating a character vector.
        let mut value = value.chars();
        let mut pattern_index = 0;
        let mut star = None;
        let mut star_value = None;

        while let Some(value_character) = value.clone().next() {
            match self.tokens.get(pattern_index) {
                Some(GlobToken::Star) => {
                    // Record the backtrack point without consuming input, so
                    // `*` can match the empty string (`*foo` matches `foo`).
                    star = Some(pattern_index);
                    pattern_index += 1;
                    star_value = Some(value.clone());
                }
                Some(GlobToken::Literal(character)) if *character == value_character => {
                    pattern_index += 1;
                    value.next();
                }
                Some(GlobToken::Any) => {
                    pattern_index += 1;
                    value.next();
                }
                _ => {
                    let (Some(star_index), Some(star_position)) = (star, star_value.as_mut())
                    else {
                        return false;
                    };
                    pattern_index = star_index + 1;
                    star_position.next();
                    value = star_position.clone();
                }
            }
        }

        while pattern_index < self.tokens.len()
            && matches!(self.tokens[pattern_index], GlobToken::Star)
        {
            pattern_index += 1;
        }
        pattern_index == self.tokens.len()
    }
}

#[cfg(test)]
fn glob_matches(pattern: &str, value: &str) -> bool {
    GlobPattern::compile(pattern).matches(value)
}

pub(super) fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Recases a rendered replacement to match the casing pattern of the typed
/// trigger when `propagate_case` is enabled.
pub(super) fn apply_case_style(typed: &str, text: &str) -> String {
    let letters: Vec<char> = typed
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect();
    match letters.as_slice() {
        [] => text.to_owned(),
        [single] if single.is_uppercase() => capitalize_first_letter(text),
        letters if letters.iter().all(|character| character.is_uppercase()) => text.to_uppercase(),
        [first, ..] if first.is_uppercase() => capitalize_first_letter(text),
        _ => text.to_owned(),
    }
}

impl ExpansionEngine {
    /// Whether a match may proceed, judged before any side effect. The plan is
    /// borrowed and the answer is a plain yes/no: an earlier signature took the
    /// plan by value and handed it back, which forced every caller to deep-copy
    /// a plan (three `String`s and the command `Arc`) on each match only to
    /// throw the copy away.
    pub(super) fn preflight_allows(&self, plan: &MatchPlan) -> bool {
        if plan
            .command
            .as_deref()
            .is_some_and(|command| self.command_execution_disabled(command))
        {
            return false;
        }
        !self.user_paused && !self.sensitive_focus
    }

    /// Whether command output produced after the match may still be injected.
    ///
    /// `generation` is the input generation the command was queued against.
    /// The session must not have entered a paused or sensitive state while the
    /// command was running, and the output must fit the configured limit.
    pub(super) fn postflight_allows(&self, generation: u64, output: &str) -> bool {
        let max_size = self.config.organization.max_replacement_size;
        if max_size > 0 && output.len() > max_size {
            return false;
        }
        if generation != self.input_generation {
            return false;
        }
        !self.sensitive_focus && !self.user_paused
    }

    /// Build the side-effect-free plan shared by immediate and deferred paths.
    pub(super) fn take_match_plan(
        &self,
        config_index: usize,
        length: usize,
        terminating_char: Option<char>,
    ) -> Option<MatchPlan> {
        if !self.match_allowed(config_index, length) {
            return None;
        }
        let expansion = &self.config.expansion[config_index];
        let (replacement_text, cursor_offset) = if expansion.command.is_some() {
            (expansion.replacement.clone(), None)
        } else {
            let (rendered, cursor_offset) = render_template_with_cursor(
                &expansion.replacement,
                &crate::TemplateContext::system(),
            )
            .ok()?;
            (rendered, cursor_offset)
        };
        let start = self.buffer.len().saturating_sub(length);
        let matched_text: String = self.buffer.iter().skip(start).collect();
        Some(MatchPlan {
            matched_text,
            terminating_char,
            cursor_offset,
            generation: self.input_generation,
            trigger_config: expansion.trigger.clone(),
            replacement_text,
            command: expansion
                .command
                .as_ref()
                .map(|command| Arc::new(command.clone())),
            propagate_case: expansion.propagate_case,
        })
    }

    fn match_allowed(&self, config_index: usize, length: usize) -> bool {
        let expansion = &self.config.expansion[config_index];
        if !self.app_filter_allows(config_index, expansion) {
            return false;
        }
        if expansion.match_mode != MatchMode::WordBoundary {
            return true;
        }
        match self.buffer.iter().rev().nth(length) {
            Some(character) => !is_word_character(*character),
            None => !self.buffer_truncated,
        }
    }

    /// An empty app filter matches everywhere. Each configured filter names
    /// its source explicitly; app IDs never fall back to editable titles.
    pub(super) fn app_filter_allows(
        &self,
        config_index: usize,
        expansion: &ExpansionConfig,
    ) -> bool {
        if expansion.app_filter.is_empty() {
            return true;
        }
        let Some(window) = &self.normalized_window else {
            return false;
        };
        self.app_filters[config_index]
            .iter()
            .enumerate()
            .any(|(filter_index, filter)| match filter {
                AppFilter::AppIdExact(app_id_filter) => window
                    .app_id
                    .as_deref()
                    .is_some_and(|app_id| app_id == app_id_filter),
                AppFilter::AppIdGlob(_) => window.app_id.as_deref().is_some_and(|app_id| {
                    self.app_filter_globs[config_index][filter_index]
                        .as_ref()
                        .is_some_and(|pattern| pattern.matches(app_id))
                }),
                AppFilter::TitleContains(title_filter) => {
                    if self.config.organization.disable_title_matching {
                        false
                    } else {
                        window
                            .title
                            .as_deref()
                            .is_some_and(|title| title.contains(title_filter))
                    }
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::glob_matches;

    #[test]
    fn question_mark_matches_one_unicode_scalar() {
        for character in ["é", "ö", "你", "Ж"] {
            assert!(glob_matches("?", character));
            assert!(glob_matches("x?x", &format!("x{character}x")));
        }
    }

    #[test]
    fn question_mark_does_not_match_multiple_unicode_scalars() {
        assert!(!glob_matches("?", "éé"));
        assert!(!glob_matches("?", "你Ж"));
        assert!(!glob_matches("x?x", "xééx"));
    }

    #[test]
    fn star_keeps_matching_across_unicode_scalars() {
        assert!(glob_matches("*你*", "prefix你suffix"));
        assert!(glob_matches("你*Ж", "你éöЖ"));
        assert!(!glob_matches("你*Ж", "你éöж"));
    }

    #[test]
    fn star_matches_the_empty_string() {
        assert!(glob_matches("*foo", "foo"));
        assert!(glob_matches("a*b", "ab"));
        assert!(glob_matches("org.*", "org."));
        assert!(glob_matches("firefox*", "firefox"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("a**b", "ab"));
        assert!(glob_matches("org.*.app", "org.kde.app"));
        assert!(!glob_matches("a*b", "a"));
        assert!(!glob_matches("*foo", "fo"));
    }
}
