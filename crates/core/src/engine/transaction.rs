//! Undo transaction and deferred command state management.
//!
//! Handles undo history, deferred match reservations, and transaction tracking
//! for expansion results that need to maintain undo semantics across async completion.

use std::borrow::Cow;

use super::{ExpansionEngine, ExpansionResult};
use crate::{InjectorCapabilities, InjectorError, TextInjector};

/// Result of the multi-stage replacement transaction.
///
/// Failures are separated by what can safely be inferred about their effect.
/// Callers may restore/retry only `NotApplied`; an uncertain acknowledgement
/// from an atomic protocol is also non-replayable, even though the protocol
/// itself cannot have applied only part of the replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionOutcome {
    Applied,
    AppliedWithCursorPositionFailure { source: InjectorError },
    NotApplied { source: InjectorError },
    UnknownApplicationOutcome { source: InjectorError },
    UnknownPartialFailure { source: InjectorError },
}

/// Ordered, content-free phases of one replacement attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionPhase {
    Prepared,
    Validated,
    Applying,
    Committed,
    Failed,
    Uncertain,
}

impl TransactionPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Validated => "validated",
            Self::Applying => "applying",
            Self::Committed => "committed",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
        }
    }
}

/// Outcome plus its validated lifecycle journal. The phases are deliberately
/// separate from backend error text so callers can expose diagnostics without
/// recording typed or replacement content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionReport {
    pub outcome: TransactionOutcome,
    pub phases: Vec<TransactionPhase>,
}

struct TransactionJournal {
    phases: Vec<TransactionPhase>,
}

impl TransactionJournal {
    fn prepared() -> Self {
        Self {
            phases: vec![TransactionPhase::Prepared],
        }
    }

    fn advance(&mut self, next: TransactionPhase) {
        let current = *self.phases.last().expect("journal always starts prepared");
        let valid = matches!(
            (current, next),
            (
                TransactionPhase::Prepared,
                TransactionPhase::Validated | TransactionPhase::Failed
            ) | (
                TransactionPhase::Validated,
                TransactionPhase::Applying | TransactionPhase::Failed
            ) | (
                TransactionPhase::Applying,
                TransactionPhase::Committed
                    | TransactionPhase::Failed
                    | TransactionPhase::Uncertain
            )
        );
        assert!(
            valid,
            "invalid output transaction transition: {current:?} -> {next:?}"
        );
        self.phases.push(next);
    }

    fn finish(mut self, outcome: TransactionOutcome) -> TransactionReport {
        let terminal = match outcome {
            TransactionOutcome::Applied
            | TransactionOutcome::AppliedWithCursorPositionFailure { .. } => {
                TransactionPhase::Committed
            }
            TransactionOutcome::NotApplied { .. } => TransactionPhase::Failed,
            TransactionOutcome::UnknownApplicationOutcome { .. }
            | TransactionOutcome::UnknownPartialFailure { .. } => TransactionPhase::Uncertain,
        };
        self.advance(terminal);
        TransactionReport {
            outcome,
            phases: self.phases,
        }
    }
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
            | Self::UnknownApplicationOutcome { source }
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
            Self::UnknownApplicationOutcome { source } => {
                write!(formatter, "replacement outcome is unknown: {source}")
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

/// Why an output route cannot type a replacement exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteLimit {
    /// More characters than the route accepts.
    TooLong { chars: usize, max: usize },
    /// A character outside ASCII on a route that types through the keyboard
    /// layout. `position` counts characters from 1.
    Unicode { character: char, position: usize },
    /// The cursor must move back, and the route cannot move it.
    CursorMove,
}

impl RouteLimit {
    /// The refusal message. It names no replacement content, so it is safe
    /// for logs.
    pub fn message(self, insertion_mode: &str) -> String {
        match self {
            Self::TooLong { chars, max } => {
                // Keystroke typing is limited by the user's
                // `keystroke_max_chars` setting rather than by the protocol,
                // so say how to raise it.
                let hint = if insertion_mode.contains("keysym") {
                    " (raise keystroke_max_chars to allow longer keystroke typing)"
                } else {
                    ""
                };
                format!(
                    "replacement requires {chars} characters but {insertion_mode} supports at most {max}{hint}"
                )
            }
            Self::Unicode { .. } => {
                format!("replacement contains Unicode that {insertion_mode} cannot guarantee")
            }
            Self::CursorMove => format!(
                "replacement requests cursor repositioning but {insertion_mode} does not support it"
            ),
        }
    }
}

/// The first reason `capabilities` cannot type `insert` (plus a reinserted
/// delimiter) exactly, or `None` when it can. This is the check made before
/// anything is typed, shared with `explain`.
pub fn route_limit(
    capabilities: &InjectorCapabilities,
    insert: &str,
    reinsert_after: Option<char>,
    cursor_offset: Option<usize>,
) -> Option<RouteLimit> {
    let trailing_chars = usize::from(reinsert_after.is_some());
    let output_chars = insert.chars().count() + trailing_chars;
    if capabilities.max_text_chars > 0 && output_chars > capabilities.max_text_chars {
        return Some(RouteLimit::TooLong {
            chars: output_chars,
            max: capabilities.max_text_chars,
        });
    }
    if !capabilities.full_unicode {
        if let Some((index, character)) = insert
            .chars()
            .chain(reinsert_after)
            .enumerate()
            .find(|(_, character)| !character.is_ascii())
        {
            return Some(RouteLimit::Unicode {
                character,
                position: index + 1,
            });
        }
    }
    // Mirror apply(): an offset that resolves to zero never moves the
    // cursor, so it must not be rejected on backends without that support.
    let cursor_move = cursor_offset
        .map(|offset| offset.saturating_add(trailing_chars))
        .is_some_and(|offset| offset > 0);
    if cursor_move && !capabilities.cursor_reposition {
        return Some(RouteLimit::CursorMove);
    }
    None
}

impl ExpansionEngine {
    fn preflight_output<I: TextInjector + ?Sized>(
        injector: &mut I,
        result: &ExpansionResult,
    ) -> Result<(), InjectorError> {
        let capabilities = injector.capabilities();
        // Checked first and before anything is typed, so a refusal is a clean
        // "not applied" rather than an uncertain partial write.
        injector.admit(!result.matched_text.is_empty() || result.reinsert_after.is_some())?;
        let preflight_text = result.reinsert_after.map_or_else(
            || Cow::Borrowed(result.insert.as_str()),
            |character| {
                let mut text = String::with_capacity(result.insert.len() + character.len_utf8());
                text.push_str(&result.insert);
                text.push(character);
                Cow::Owned(text)
            },
        );
        injector.preflight_text(&preflight_text)?;
        match route_limit(
            &capabilities,
            &result.insert,
            result.reinsert_after,
            result.cursor_offset,
        ) {
            None => {}
            Some(limit) => {
                return Err(InjectorError {
                    backend: injector.name(),
                    message: limit.message(capabilities.insertion_mode),
                    retryable: false,
                })
            }
        }
        Ok(())
    }

    /// Apply an expansion as one backend transaction, including optional
    /// delimiter reinsertion and cursor repositioning.
    pub fn apply<I: TextInjector + ?Sized>(
        injector: &mut I,
        result: &ExpansionResult,
    ) -> TransactionOutcome {
        Self::apply_with_report(injector, result).outcome
    }

    /// Apply a replacement and retain its state transitions for diagnostics.
    pub fn apply_with_report<I: TextInjector + ?Sized>(
        injector: &mut I,
        result: &ExpansionResult,
    ) -> TransactionReport {
        let mut journal = TransactionJournal::prepared();
        if let Err(source) = Self::preflight_output(injector, result) {
            return journal.finish(TransactionOutcome::NotApplied { source });
        }
        journal.advance(TransactionPhase::Validated);
        journal.advance(TransactionPhase::Applying);
        // Only a match that has to carry a terminating character through needs
        // new strings. The common case borrows the result directly rather than
        // copying the matched text and the whole replacement on every
        // expansion just to hand out references to the copies.
        match result.reinsert_after {
            None => {
                if let Err(source) = injector.replace(&result.matched_text, &result.insert) {
                    let outcome = if injector.capabilities().atomic_replace
                        && !injector.replace_error_may_have_applied()
                    {
                        TransactionOutcome::NotApplied { source }
                    } else if injector.capabilities().atomic_replace {
                        TransactionOutcome::UnknownApplicationOutcome { source }
                    } else {
                        TransactionOutcome::UnknownPartialFailure { source }
                    };
                    return journal.finish(outcome);
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
                    let outcome = if injector.capabilities().atomic_replace
                        && !injector.replace_error_may_have_applied()
                    {
                        TransactionOutcome::NotApplied { source }
                    } else if injector.capabilities().atomic_replace {
                        TransactionOutcome::UnknownApplicationOutcome { source }
                    } else {
                        TransactionOutcome::UnknownPartialFailure { source }
                    };
                    return journal.finish(outcome);
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
                return journal
                    .finish(TransactionOutcome::AppliedWithCursorPositionFailure { source });
            }
        }
        journal.finish(TransactionOutcome::Applied)
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    fn error() -> InjectorError {
        InjectorError {
            backend: "test",
            message: "simulated".into(),
            retryable: true,
        }
    }

    #[test]
    fn terminal_phases_distinguish_commit_failure_and_uncertainty() {
        let mut committed = TransactionJournal::prepared();
        committed.advance(TransactionPhase::Validated);
        committed.advance(TransactionPhase::Applying);
        assert_eq!(
            committed.finish(TransactionOutcome::Applied).phases,
            vec![
                TransactionPhase::Prepared,
                TransactionPhase::Validated,
                TransactionPhase::Applying,
                TransactionPhase::Committed,
            ]
        );

        let mut failed = TransactionJournal::prepared();
        failed.advance(TransactionPhase::Validated);
        failed.advance(TransactionPhase::Applying);
        assert_eq!(
            failed
                .finish(TransactionOutcome::NotApplied { source: error() })
                .phases
                .last(),
            Some(&TransactionPhase::Failed)
        );

        let mut uncertain = TransactionJournal::prepared();
        uncertain.advance(TransactionPhase::Validated);
        uncertain.advance(TransactionPhase::Applying);
        let report =
            uncertain.finish(TransactionOutcome::UnknownPartialFailure { source: error() });
        assert_eq!(report.phases.last(), Some(&TransactionPhase::Uncertain));
        assert!(!report.phases.contains(&TransactionPhase::Committed));
    }

    #[test]
    fn preflight_failure_cannot_enter_the_applying_phase() {
        let report = TransactionJournal::prepared()
            .finish(TransactionOutcome::NotApplied { source: error() });
        assert_eq!(
            report.phases,
            vec![TransactionPhase::Prepared, TransactionPhase::Failed]
        );
    }
}
