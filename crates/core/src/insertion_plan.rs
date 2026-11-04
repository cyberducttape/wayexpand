//! Capability-based recommendation for delivering replacement text.
//!
//! The plan never grants permission to erase text. A non-atomic route still
//! needs the existing explicit compatibility admission; clipboard/manual
//! paste is the fallback when that contract is unavailable or too slow.

use crate::CompatibilityMode;

/// Product latency objective for ordinary paced-key output. It is a routing
/// threshold, not a certification claim or an observed p95 guarantee.
pub const INTERACTIVE_INSERTION_TARGET_MS: u64 = 100;

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

/// Recommend an output path from live route capabilities and explicit
/// compatibility consent. Unknown or unsupported routes fail to the manual
/// clipboard path. Paced synthesis is recommended only when the estimated
/// duration is within the ordinary-interaction target.
pub fn plan_insertion(
    characters: usize,
    route: Option<InsertionRouteCapabilities>,
    compatibility: CompatibilityMode,
) -> InsertionStrategy {
    let Some(route) = route else {
        return InsertionStrategy::ClipboardAssistedPaste;
    };
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
    if route.max_text_chars > 0 && characters > route.max_text_chars {
        return InsertionStrategy::ClipboardAssistedPaste;
    }
    let characters = u64::try_from(characters).unwrap_or(u64::MAX);
    let rate = u64::from(rate);
    let predicted_ms = characters.saturating_mul(1_000).saturating_add(rate - 1) / rate;
    if predicted_ms > INTERACTIVE_INSERTION_TARGET_MS {
        InsertionStrategy::ClipboardAssistedPaste
    } else {
        InsertionStrategy::PacedSyntheticKeystrokes { predicted_ms }
    }
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
                    ..route()
                }),
                CompatibilityMode::TrackedFocus,
            ),
            InsertionStrategy::NativeTextCommit
        );
    }

    #[test]
    fn paced_synthesis_is_recommended_only_under_the_latency_target() {
        assert_eq!(
            plan_insertion(8, Some(route()), CompatibilityMode::TrackedFocus),
            InsertionStrategy::PacedSyntheticKeystrokes { predicted_ms: 97 }
        );
        assert_eq!(
            plan_insertion(9, Some(route()), CompatibilityMode::TrackedFocus),
            InsertionStrategy::ClipboardAssistedPaste
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
    }
}
