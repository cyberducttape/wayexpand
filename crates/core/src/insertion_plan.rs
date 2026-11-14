//! Capability-based recommendation for delivering replacement text.
//!
//! The plan never grants permission to erase text. A non-atomic route still
//! needs the existing explicit compatibility admission; clipboard/manual
//! paste is the fallback when that contract is unavailable or too slow.

use crate::CompatibilityMode;

/// Do not recommend paced synthetic typing when its estimated duration is
/// longer than a short interactive replacement. The configured keystroke
/// character ceiling cannot authorize paced output beyond this transaction
/// budget; longer output must use copy/paste or file generation.
pub(crate) const MAX_RECOMMENDED_PACED_INSERTION_MS: u64 = 100;

/// Estimate paced synthetic output with conservative ceiling arithmetic.
/// A missing or zero rate is unknown and therefore cannot be approved.
pub fn paced_output_duration_ms(
    events: usize,
    throughput_chars_per_sec: Option<u32>,
) -> Option<u64> {
    let rate = u64::from(throughput_chars_per_sec.filter(|rate| *rate > 0)?);
    let events = u64::try_from(events).unwrap_or(u64::MAX);
    Some(events.saturating_mul(1_000).saturating_add(rate - 1) / rate)
}

/// Count paced output units for a complete replacement transaction. The GUI
/// can supply a conservative cursor offset; runtime preflight supplies the
/// matched text and actual cursor offset from the rendered expansion.
pub fn paced_replacement_event_count(
    trigger_chars: usize,
    replacement_chars: usize,
    reinsert_delimiter: bool,
    cursor_offset: Option<usize>,
) -> usize {
    let delimiter = usize::from(reinsert_delimiter);
    trigger_chars
        .saturating_add(delimiter)
        .saturating_add(replacement_chars)
        .saturating_add(delimiter)
        .saturating_add(cursor_offset.map_or(0, |offset| offset.saturating_add(delimiter)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertionRouteCapabilities {
    /// The active route verifies the target and replaces it atomically.
    pub atomic_verified_replace: bool,
    /// The route can commit UTF-8 text directly instead of synthesizing keys.
    pub native_text_commit: bool,
    /// Approximate speed of a paced synthetic-key route, when applicable.
    pub paced_throughput_chars_per_sec: Option<u32>,
    /// Route-specific maximum output length; zero means no advertised limit.
    pub max_text_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertionStrategy {
    VerifiedReplacement,
    NativeTextCommit,
    ClipboardAssistedPaste,
    PacedSyntheticKeystrokes { predicted_ms: u64 },
}

/// Recommend an output path for standalone inserted text. Replacement callers
/// should use [`plan_replacement`] so trigger erasure and cursor movement are
/// included in the paced-work estimate. Unknown or unsupported routes fail to
/// the manual clipboard path. Paced synthesis is recommended only when it fits
/// both the configured route limit and the 100 ms interactive-output budget.
/// Raising `keystroke_max_chars` does not bypass this safety limit.
pub fn plan_insertion(
    characters: usize,
    route: Option<InsertionRouteCapabilities>,
    compatibility: CompatibilityMode,
) -> InsertionStrategy {
    plan_replacement(characters, characters, route, compatibility)
}

/// Recommend a delivery path for a known replacement workload.
///
/// `output_characters` is the actual number of inserted characters (including
/// a reinserted delimiter, if any). `paced_events` is a conservative count of
/// all paced work: trigger erasure, replacement insertion, delimiter handling,
/// and cursor repositioning. Keeping these separate prevents a long trigger
/// or cursor move from passing the latency budget while still enforcing the
/// route's maximum against replacement text only.
pub fn plan_replacement(
    output_characters: usize,
    paced_events: usize,
    route: Option<InsertionRouteCapabilities>,
    compatibility: CompatibilityMode,
) -> InsertionStrategy {
    let Some(route) = route else {
        return InsertionStrategy::ClipboardAssistedPaste;
    };
    if route.max_text_chars > 0 && output_characters > route.max_text_chars {
        return InsertionStrategy::ClipboardAssistedPaste;
    }
    if route.atomic_verified_replace {
        return InsertionStrategy::VerifiedReplacement;
    }
    if compatibility == CompatibilityMode::Off {
        return InsertionStrategy::ClipboardAssistedPaste;
    }
    if route.native_text_commit {
        return InsertionStrategy::NativeTextCommit;
    }
    let Some(rate) = route
        .paced_throughput_chars_per_sec
        .filter(|rate| *rate > 0)
    else {
        return InsertionStrategy::ClipboardAssistedPaste;
    };
    let predicted_ms = paced_output_duration_ms(paced_events, Some(rate))
        .expect("rate was checked as positive above");
    if predicted_ms > MAX_RECOMMENDED_PACED_INSERTION_MS {
        return InsertionStrategy::ClipboardAssistedPaste;
    }
    InsertionStrategy::PacedSyntheticKeystrokes { predicted_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route() -> InsertionRouteCapabilities {
        InsertionRouteCapabilities {
            atomic_verified_replace: false,
            native_text_commit: false,
            paced_throughput_chars_per_sec: Some(83),
            max_text_chars: 250,
        }
    }

    #[test]
    fn verified_atomic_route_is_preferred_without_compatibility_consent() {
        let strategy = plan_insertion(
            10_000,
            Some(InsertionRouteCapabilities {
                atomic_verified_replace: true,
                max_text_chars: 0,
                ..route()
            }),
            CompatibilityMode::Off,
        );
        assert_eq!(strategy, InsertionStrategy::VerifiedReplacement);
    }

    #[test]
    fn unsafe_automatic_replacement_defaults_to_manual_paste() {
        assert_eq!(
            plan_insertion(4, Some(route()), CompatibilityMode::Off),
            InsertionStrategy::ClipboardAssistedPaste
        );
    }

    #[test]
    fn native_text_commit_is_preferred_after_explicit_compatibility_consent() {
        assert_eq!(
            plan_insertion(
                5_000,
                Some(InsertionRouteCapabilities {
                    native_text_commit: true,
                    max_text_chars: 0,
                    ..route()
                }),
                CompatibilityMode::TrackedFocus,
            ),
            InsertionStrategy::NativeTextCommit
        );
    }

    #[test]
    fn paced_compatibility_output_is_recommended_only_within_the_interactive_budget() {
        assert_eq!(
            plan_insertion(8, Some(route()), CompatibilityMode::TrackedFocus),
            InsertionStrategy::PacedSyntheticKeystrokes { predicted_ms: 97 }
        );
        assert_eq!(
            plan_insertion(9, Some(route()), CompatibilityMode::TrackedFocus),
            InsertionStrategy::ClipboardAssistedPaste
        );
        assert_eq!(
            plan_insertion(
                9,
                Some(InsertionRouteCapabilities {
                    paced_throughput_chars_per_sec: Some(90),
                    ..route()
                }),
                CompatibilityMode::TrackedFocus,
            ),
            InsertionStrategy::PacedSyntheticKeystrokes { predicted_ms: 100 }
        );
        assert_eq!(
            plan_insertion(250, Some(route()), CompatibilityMode::TrackedFocus),
            InsertionStrategy::ClipboardAssistedPaste,
        );
    }

    #[test]
    fn replacement_plan_accounts_for_erasure_delimiters_and_cursor_work() {
        assert_eq!(
            plan_replacement(8, 11, Some(route()), CompatibilityMode::TrackedFocus),
            InsertionStrategy::ClipboardAssistedPaste,
            "the output payload fits, but trigger deletion pushes the transaction over budget"
        );
        assert_eq!(
            plan_replacement(8, 8, Some(route()), CompatibilityMode::TrackedFocus),
            InsertionStrategy::PacedSyntheticKeystrokes { predicted_ms: 97 }
        );
        assert_eq!(
            plan_replacement(8, 300, Some(route()), CompatibilityMode::TrackedFocus),
            InsertionStrategy::ClipboardAssistedPaste,
            "cursor reposition work must count toward paced latency"
        );
    }

    #[test]
    fn planner_uses_the_shared_runtime_duration_boundary() {
        let route_for_rate = |rate| InsertionRouteCapabilities {
            atomic_verified_replace: false,
            native_text_commit: false,
            paced_throughput_chars_per_sec: Some(rate),
            max_text_chars: 0,
        };

        for rate in 1..=500 {
            for events in 0..=600 {
                let predicted = paced_output_duration_ms(events, Some(rate)).unwrap();
                let plan = plan_replacement(
                    0,
                    events,
                    Some(route_for_rate(rate)),
                    CompatibilityMode::TrackedFocus,
                );
                if predicted <= MAX_RECOMMENDED_PACED_INSERTION_MS {
                    assert_eq!(
                        plan,
                        InsertionStrategy::PacedSyntheticKeystrokes {
                            predicted_ms: predicted
                        },
                        "events={events}, rate={rate}"
                    );
                } else {
                    assert_eq!(
                        plan,
                        InsertionStrategy::ClipboardAssistedPaste,
                        "events={events}, rate={rate}"
                    );
                }
            }
        }
    }

    #[test]
    fn paced_duration_refuses_unknown_rates_and_saturates_large_workloads() {
        assert_eq!(paced_output_duration_ms(1, None), None);
        assert_eq!(paced_output_duration_ms(1, Some(0)), None);
        assert_eq!(
            paced_output_duration_ms(usize::MAX, Some(1)),
            Some(u64::MAX)
        );
    }

    #[test]
    fn unavailable_or_over_limit_routes_use_manual_paste() {
        assert_eq!(
            plan_insertion(1, None, CompatibilityMode::TrackedFocus),
            InsertionStrategy::ClipboardAssistedPaste
        );
        assert_eq!(
            plan_insertion(
                9,
                Some(InsertionRouteCapabilities {
                    max_text_chars: 8,
                    ..route()
                }),
                CompatibilityMode::TrackedFocus,
            ),
            InsertionStrategy::ClipboardAssistedPaste
        );
        assert_eq!(
            plan_insertion(
                9,
                Some(InsertionRouteCapabilities {
                    atomic_verified_replace: true,
                    max_text_chars: 8,
                    ..route()
                }),
                CompatibilityMode::Off,
            ),
            InsertionStrategy::ClipboardAssistedPaste
        );
        assert_eq!(
            plan_insertion(
                9,
                Some(InsertionRouteCapabilities {
                    native_text_commit: true,
                    max_text_chars: 8,
                    ..route()
                }),
                CompatibilityMode::TrackedFocus,
            ),
            InsertionStrategy::ClipboardAssistedPaste
        );
    }
}
