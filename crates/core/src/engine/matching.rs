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
    /// Apply preflight policy to a match plan without causing side effects.
    pub(super) fn apply_preflight_policy(&self, plan: MatchPlan) -> Option<MatchPlan> {
        if plan.is_command_backed() && self.config.organization.disable_commands {
            return None;
        }
        if self.user_paused || self.sensitive_focus {
            return None;
        }
        Some(plan)
    }

    /// Validate command output after execution and before injection.
    pub(super) fn apply_postflight_policy(&self, plan: &MatchPlan, output: &str) -> Option<String> {
        let max_size = self.config.organization.max_replacement_size;
        if max_size > 0 && output.len() > max_size {
            return None;
        }
        if plan.generation != self.input_generation {
            return None;
        }
        if plan.sensitive_focus != self.sensitive_focus || plan.user_paused != self.user_paused {
            return None;
        }
        Some(output.to_string())
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
            sensitive_focus: self.sensitive_focus,
            user_paused: self.user_paused,
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
