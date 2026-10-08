//! Input and expansion state contracts owned by the engine.

use crate::{CommandConfig, KeyChord};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Text(String),
    Key(KeyChord),
    Backspace,
    EndOfInput,
    Delimiter(char),
    Reset,
    FocusChanged { sensitive: bool },
    CompositionChanged { active: bool },
    PauseChanged(bool),
    WindowChanged(Option<WindowContext>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowContext {
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub instance_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct NormalizedWindowContext {
    pub(super) app_id: Option<String>,
    pub(super) title: Option<String>,
    pub(super) instance_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionResult {
    pub snippet_id: String,
    pub trigger: String,
    pub matched_text: String,
    pub insert: String,
    pub cursor_offset: Option<usize>,
    pub reinsert_after: Option<char>,
    pub command_backed: bool,
    pub undoable: bool,
    /// Characters a host appended to `matched_text` and `insert` after the
    /// engine produced this result (see [`ExpansionResult::fold_typed_suffix`]).
    /// Lets the engine find this result's deferred reservation without
    /// depending on the edited display text.
    pub folded_suffix_chars: usize,
}

impl ExpansionResult {
    /// Fold a character the user already typed into the target after the
    /// trigger into this transaction: it is erased with the trigger and typed
    /// again after the replacement. A requested cursor position stays at the
    /// same place inside the replacement.
    pub fn fold_typed_suffix(&mut self, character: char) {
        self.matched_text.push(character);
        self.insert.push(character);
        self.folded_suffix_chars += 1;
        if let Some(offset) = self.cursor_offset.as_mut() {
            *offset += 1;
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingExpansionResult {
    pub(super) snippet_id: String,
    pub trigger: String,
    pub matched_text: String,
    pub template_text: String,
    pub cursor_offset: Option<usize>,
    pub reinsert_after: Option<char>,
    pub(super) max_replacement_size: usize,
    pub(super) config_index: usize,
    pub(super) generation: u64,
    pub(super) propagate_case: bool,
    pub(super) cache_ms: u64,
    pub(super) cached_output: Option<String>,
    pub(super) undoable: bool,
    pub command: Option<CommandConfig>,
    pub(super) form: Option<Arc<Vec<crate::FormField>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingExpansionDispatch {
    Ready(ExpansionResult),
    Queued,
}

#[derive(Debug, Clone)]
pub struct MatchPlan {
    pub snippet_id: String,
    pub matched_text: String,
    pub terminating_char: Option<char>,
    pub cursor_offset: Option<usize>,
    pub generation: u64,
    pub trigger_config: String,
    pub replacement_text: String,
    pub command: Option<Arc<CommandConfig>>,
    pub propagate_case: bool,
    pub form: Option<Arc<Vec<crate::FormField>>>,
}

impl MatchPlan {
    pub fn is_command_backed(&self) -> bool {
        self.command.is_some()
    }
}
