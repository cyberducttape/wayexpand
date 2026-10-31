//! Input and expansion state contracts owned by the engine.

use crate::{CommandConfig, KeyChord};
use std::{fmt, ops::Deref, sync::Arc};
use zeroize::{Zeroize, Zeroizing};

/// Owned transient text that is overwritten when its last owner is dropped.
/// Clones receive their own protected allocation, and Debug never exposes
/// plaintext payloads.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SensitiveText(Zeroizing<String>);

impl SensitiveText {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Move the protected allocation into an ordinary string for a public API
    /// that explicitly returns rendered text to its caller.
    pub fn into_string(mut self) -> String {
        std::mem::take(&mut *self.0)
    }

    pub fn into_zeroizing(self) -> Zeroizing<String> {
        self.0
    }

    pub fn push(&mut self, character: char) {
        self.0.push(character);
    }

    pub fn push_str(&mut self, value: &str) {
        self.0.push_str(value);
    }

    pub fn pop(&mut self) -> Option<char> {
        self.0.pop()
    }

    pub fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SensitiveText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Deref for SensitiveText {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for SensitiveText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for SensitiveText {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for SensitiveText {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl PartialEq<str> for SensitiveText {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for SensitiveText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for SensitiveText {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

#[cfg(test)]
mod sensitive_text_tests {
    use super::SensitiveText;
    use zeroize::Zeroize;

    #[test]
    fn clones_are_independently_zeroizing_and_debug_is_redacted() {
        let value = SensitiveText::from("transient replacement secret");
        let clone = value.clone();
        assert_eq!(value.as_str(), clone.as_str());
        assert_ne!(value.as_ptr(), clone.as_ptr());
        assert_eq!(format!("{value:?}"), "[REDACTED]");
        assert!(!format!("{value:?}").contains("transient replacement secret"));
    }

    #[test]
    fn explicit_zeroize_overwrites_retained_bytes() {
        let mut value = SensitiveText::from("transaction plaintext");
        let pointer = value.as_ptr();
        let length = value.len();
        value.0.zeroize();

        assert!(value.is_empty());
        let residual = unsafe { std::slice::from_raw_parts(pointer, length) };
        assert!(residual.iter().all(|byte| *byte == 0));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// One committed Unicode scalar. Physical key routes use this to avoid
    /// allocating an owned string for ordinary printable keys.
    TextChar(char),
    Text(String),
    Key(KeyChord),
    Backspace,
    EndOfInput,
    Delimiter(char),
    Reset,
    FocusChanged {
        sensitive: bool,
    },
    CompositionChanged {
        active: bool,
    },
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
    pub snippet_id: Arc<str>,
    pub trigger: Arc<str>,
    pub matched_text: SensitiveText,
    pub insert: SensitiveText,
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
    pub(super) snippet_id: Arc<str>,
    pub trigger: Arc<str>,
    pub matched_text: SensitiveText,
    pub template_text: SensitiveText,
    pub cursor_offset: Option<usize>,
    pub reinsert_after: Option<char>,
    pub(super) max_replacement_size: usize,
    pub(super) config_index: usize,
    pub(super) generation: u64,
    /// Focus context at the moment the trigger was consumed. Deferred
    /// command results must not be rendered after this context changes.
    pub(super) captured_window: Option<crate::WindowContext>,
    pub(super) propagate_case: bool,
    pub(super) cache_ms: u64,
    pub(super) cached_output: Option<SensitiveText>,
    pub(super) undoable: bool,
    pub command: Option<Arc<CommandConfig>>,
    pub(super) form_template: Option<Arc<str>>,
    pub(super) form: Option<Arc<Vec<crate::FormField>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingExpansionDispatch {
    Ready(ExpansionResult),
    Queued,
}

#[derive(Debug, Clone)]
pub struct MatchPlan {
    pub snippet_id: Arc<str>,
    pub matched_text: SensitiveText,
    pub terminating_char: Option<char>,
    pub cursor_offset: Option<usize>,
    pub generation: u64,
    pub trigger_config: Arc<str>,
    pub replacement_text: SensitiveText,
    pub command: Option<Arc<CommandConfig>>,
    pub propagate_case: bool,
    pub form_template: Option<Arc<str>>,
    pub form: Option<Arc<Vec<crate::FormField>>>,
}

impl MatchPlan {
    pub fn is_command_backed(&self) -> bool {
        self.command.is_some()
    }
}
