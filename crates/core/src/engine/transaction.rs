//! Undo transaction and deferred command state management.
//!
//! Handles undo history, deferred match reservations, and transaction tracking
//! for expansion results that need to maintain undo semantics across async completion.

use super::{ExpansionEngine, ExpansionResult};
use crate::{InjectorError, TextInjector};

/// Result of the multi-stage replacement transaction.
///
/// The distinction between `NotApplied` and `UnknownPartialFailure` is
/// intentional: callers may safely restore/retry only the former. Once a
/// non-atomic backend may have erased or inserted part of the replacement,
/// the original text must not be replayed automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionOutcome {
    Applied,
    AppliedWithCursorPositionFailure { source: InjectorError },
    NotApplied { source: InjectorError },
    UnknownPartialFailure { source: InjectorError },
}

impl TransactionOutcome {
    pub fn is_applied(&self) -> bool {
        matches!(
            self,
            Self::Applied | Self::AppliedWithCursorPositionFailure { .. }
        )
    }

    pub fn source(&self) -> Option<&InjectorError> {
        match self {
            Self::Applied => None,
            Self::AppliedWithCursorPositionFailure { source }
            | Self::NotApplied { source }
            | Self::UnknownPartialFailure { source } => Some(source),
        }
    }
}

impl std::fmt::Display for TransactionOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Applied => formatter.write_str("transaction applied"),
            Self::AppliedWithCursorPositionFailure { source } => {
                write!(
                    formatter,
                    "replacement applied but cursor repositioning failed: {source}"
                )
            }
            Self::NotApplied { source } => write!(formatter, "transaction not applied: {source}"),
            Self::UnknownPartialFailure { source } => {
                write!(formatter, "transaction may be partially applied: {source}")
            }
        }
    }
}

impl std::error::Error for TransactionOutcome {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source().map(|source| source as &dyn std::error::Error)
    }
}

/// Return the exact `(restore, erase)` strings for an expansion's undo
/// transaction.
///
/// A non-exclusive word-boundary match has already delivered its terminating
/// character to the application. Since `apply` replaces that character along
/// with the trigger, undo must include it as well.
pub(super) fn transaction_texts(
    restore: &str,
    replacement: &str,
    reinsert_after: Option<char>,
) -> (String, String) {
    let mut restore_text = restore.to_owned();
    let mut replacement_text = replacement.to_owned();
    if let Some(character) = reinsert_after {
        restore_text.push(character);
        replacement_text.push(character);
    }
    (restore_text, replacement_text)
}

impl ExpansionEngine {
    fn preflight_output<I: TextInjector + ?Sized>(
        injector: &I,
        result: &ExpansionResult,
    ) -> Result<(), InjectorError> {
        let capabilities = injector.capabilities();
        let output_chars =
            result.insert.chars().count() + result.reinsert_after.map_or(0, char::len_utf8);
        if capabilities.max_text_chars > 0 && output_chars > capabilities.max_text_chars {
            return Err(InjectorError {
                backend: injector.name(),
                message: format!(
                    "replacement requires {output_chars} characters but {} supports at most {}",
                    capabilities.insertion_mode, capabilities.max_text_chars
                ),
                retryable: false,
            });
        }
        if !capabilities.full_unicode
            && result
                .insert
                .chars()
                .chain(result.reinsert_after)
                .any(|character| !character.is_ascii())
        {
            return Err(InjectorError {
                backend: injector.name(),
                message: format!(
                    "replacement contains Unicode that {} cannot guarantee",
                    capabilities.insertion_mode
                ),
                retryable: false,
            });
        }
        if result.cursor_offset.is_some() && !capabilities.cursor_reposition {
            return Err(InjectorError {
                backend: injector.name(),
                message: format!(
                    "replacement requests cursor repositioning but {} does not support it",
                    capabilities.insertion_mode
                ),
                retryable: false,
            });
        }
        Ok(())
    }

    /// Apply an expansion as one backend transaction, including optional
    /// delimiter reinsertion and cursor repositioning.
    pub fn apply<I: TextInjector + ?Sized>(
        injector: &mut I,
        result: &ExpansionResult,
    ) -> TransactionOutcome {
        if let Err(source) = Self::preflight_output(injector, result) {
            return TransactionOutcome::NotApplied { source };
        }
        // Only a match that has to carry a terminating character through needs
        // new strings. The common case borrows the result directly rather than
        // copying the matched text and the whole replacement on every
        // expansion just to hand out references to the copies.
        match result.reinsert_after {
            None => {
                if let Err(source) = injector.replace(&result.matched_text, &result.insert) {
                    return if injector.capabilities().atomic_replace {
                        TransactionOutcome::NotApplied { source }
                    } else {
                        TransactionOutcome::UnknownPartialFailure { source }
                    };
                }
            }
            Some(character) => {
                let mut erase =
                    String::with_capacity(result.matched_text.len() + character.len_utf8());
                erase.push_str(&result.matched_text);
                erase.push(character);
                let mut insert = String::with_capacity(result.insert.len() + character.len_utf8());
                insert.push_str(&result.insert);
                insert.push(character);
                if let Err(source) = injector.replace(&erase, &insert) {
                    return if injector.capabilities().atomic_replace {
                        TransactionOutcome::NotApplied { source }
                    } else {
                        TransactionOutcome::UnknownPartialFailure { source }
                    };
                }
            }
        }
        let trailing_offset = usize::from(result.reinsert_after.is_some());
        if let Some(offset) = result
            .cursor_offset
            .map(|offset| offset.saturating_add(trailing_offset))
            .filter(|offset| *offset > 0)
        {
            if let Err(source) = injector.move_cursor_left(offset) {
                return TransactionOutcome::AppliedWithCursorPositionFailure { source };
            }
        }
        TransactionOutcome::Applied
    }
}
