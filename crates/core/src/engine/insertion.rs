//! Explicit snippet insertion used by the quick picker and control API.

use super::{ExpansionEngine, ExpansionResult};

/// Why a snippet chosen by trigger was not inserted. Each case is a deliberate
/// refusal that mirrors the rules for typed expansion, not a transient failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertError {
    /// No enabled snippet has this trigger in the running configuration.
    NotFound,
    /// Expansion is paused by the user.
    Paused,
    /// The focused field is sensitive (for example a password field).
    SensitiveField,
    /// The snippet's app filter does not match the focused application, or
    /// no window is known (fail closed, as for typed triggers).
    NotForThisApp,
    /// Command-backed snippets run only when their trigger is typed.
    CommandBacked,
    /// The rendered template failed or exceeds the organization limit.
    Unrenderable,
}

impl std::fmt::Display for InsertError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotFound => "no enabled snippet has that trigger",
            Self::Paused => "expansion is paused",
            Self::SensitiveField => "the focused field is sensitive",
            Self::NotForThisApp => "the snippet is not enabled for the focused application",
            Self::CommandBacked => "command-backed snippets run only when their trigger is typed",
            Self::Unrenderable => "the snippet could not be rendered within policy limits",
        })
    }
}

impl std::error::Error for InsertError {}

impl ExpansionEngine {
    /// Prepare an explicit insertion of the snippet whose configured trigger
    /// is `trigger`, at the cursor, with nothing erased. The same safety rules
    /// as typed expansion apply: pause and sensitive fields refuse, app filters
    /// fail closed, and command-backed snippets are never run this way.
    pub fn prepare_insert(&self, trigger: &str) -> Result<ExpansionResult, InsertError> {
        let (config_index, expansion) = self
            .config
            .expansion
            .iter()
            .enumerate()
            .find(|(_, expansion)| expansion.enabled && expansion.answers_to(trigger))
            .ok_or(InsertError::NotFound)?;
        if self.user_paused {
            return Err(InsertError::Paused);
        }
        if self.sensitive_focus {
            return Err(InsertError::SensitiveField);
        }
        if expansion.command.is_some() {
            return Err(InsertError::CommandBacked);
        }
        if !self.app_filter_allows(config_index, expansion) {
            return Err(InsertError::NotForThisApp);
        }
        let (insert, cursor_offset) =
            crate::render_template_with_cursor(&expansion.replacement, &self.template_context())
                .map_err(|_| InsertError::Unrenderable)?;
        let limit = self.config.organization.max_replacement_size;
        if limit > 0 && insert.len() > limit {
            return Err(InsertError::Unrenderable);
        }
        Ok(ExpansionResult {
            snippet_id: expansion.id.clone(),
            trigger: expansion.trigger.clone(),
            matched_text: String::new(),
            insert,
            cursor_offset,
            reinsert_after: None,
            command_backed: false,
            undoable: false,
            folded_suffix_chars: 0,
        })
    }
}
