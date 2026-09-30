//! Trigger matching and expansion planning.
//!
//! Handles pattern matching against the input buffer, match plan generation,
//! and policy application for deciding whether and how to execute expansions.

use std::sync::Arc;

use super::{ExpansionEngine, MatchPlan};
use crate::config::capitalize_first_letter;
use crate::{render_template_with_cursor, ExpansionConfig, MatchMode};

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
        if plan.is_command_backed() && self.config.organization.disable_commands {
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

    /// An empty app filter matches everywhere; a configured filter requires a
    /// known app id, or an allowed title fallback when no app id is available.
    fn app_filter_allows(&self, config_index: usize, expansion: &ExpansionConfig) -> bool {
        if expansion.app_filter.is_empty() {
            return true;
        }
        let Some(window) = &self.normalized_window else {
            return false;
        };
        self.app_filters_lower[config_index].iter().any(|filter| {
            if let Some(app_id) = window.app_id.as_deref() {
                app_id.contains(filter.as_str())
            } else if self.config.organization.disable_title_matching {
                false
            } else {
                window
                    .title
                    .as_deref()
                    .is_some_and(|title| title.contains(filter.as_str()))
            }
        })
    }
}
