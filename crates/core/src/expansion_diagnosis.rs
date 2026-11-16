//! "Why didn't this expand?" from the daemon's content-free status.
//!
//! Reads only fields the daemon already publishes: whether matching is on,
//! the input/match counters since it started, and its last few expansion
//! outcomes. None of them carry typed text, triggers or replacements, so a
//! diagnosis can be shown or pasted into a bug report safely.
//!
//! Matching scope that depends on the focused window (terminal-only routes,
//! capture allowlists) is deliberately left out of the verdict: whoever asks
//! is looking at a terminal or the WayExpand window, not the application
//! where the expansion failed.

use crate::DaemonStatus;

/// Why the daemon is not matching typed text anywhere right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchingBlock {
    /// Keyboard capture is not connected.
    CaptureDisconnected,
    /// The evdev route cannot read the keyboard layout, so it cannot tell
    /// which characters were typed.
    KeyboardLayoutUnavailable,
    /// No output route can replace the trigger.
    OutputUnavailable,
    /// Organization policy does not allow the active route.
    PolicyRoute,
    Paused,
    /// The configuration failed to load or reload.
    ConfigurationUnhealthy,
    /// A keystroke route has no consent to replace typed text
    /// (`compatibility_mode`).
    ReplacementNotAllowed,
    /// A reason this build does not know.
    Other,
}

impl MatchingBlock {
    /// The window-independent reasons from `matching_unavailable_reason`.
    /// Application-scope reasons and `none` give `None`.
    pub fn from_reason(reason: &str) -> Option<Self> {
        Some(match reason {
            "none" | "application_scope_denied" | "application_scope_unknown" => return None,
            "capture_disconnected" => Self::CaptureDisconnected,
            "keyboard_layout_unavailable" => Self::KeyboardLayoutUnavailable,
            "replacement_route_unavailable" => Self::OutputUnavailable,
            "organization_policy_route_incompatible" => Self::PolicyRoute,
            "paused" => Self::Paused,
            "configuration_unhealthy" => Self::ConfigurationUnhealthy,
            "replacement_not_admitted" => Self::ReplacementNotAllowed,
            _ => Self::Other,
        })
    }

    /// A stable identifier for JSON output.
    pub fn code(self) -> &'static str {
        match self {
            Self::CaptureDisconnected => "capture_disconnected",
            Self::KeyboardLayoutUnavailable => "keyboard_layout_unavailable",
            Self::OutputUnavailable => "output_unavailable",
            Self::PolicyRoute => "policy_route",
            Self::Paused => "paused",
            Self::ConfigurationUnhealthy => "configuration_unhealthy",
            Self::ReplacementNotAllowed => "replacement_not_allowed",
            Self::Other => "other",
        }
    }
}

/// What happened to one recent expansion, in user terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// The route accepted the replacement. The application's text was not
    /// checked; only the user can confirm it.
    Expanded,
    /// Inserted, but the cursor could not be moved to `{{cursor}}`.
    ExpandedCursorMisplaced,
    /// The route refused it before changing anything.
    NotApplied,
    /// The route cannot say whether the application received it.
    Unknown,
    /// Part of it was written; the field needs checking.
    Partial,
    /// Organization or command policy stopped it.
    BlockedByPolicy,
    /// More typing arrived before output, so it was dropped rather than
    /// risk landing in the wrong place.
    InterruptedByTyping,
    /// A command, form or helper behind the snippet failed.
    SnippetActionFailed,
    /// A reason this build does not know.
    Other,
}

impl OutcomeKind {
    /// Classify a `completion` label from `recent_expansion_outcomes`.
    pub fn from_completion(completion: &str) -> Self {
        match completion {
            "applied" => Self::Expanded,
            "inserted; cursor placement failed" => Self::ExpandedCursorMisplaced,
            "not applied" => Self::NotApplied,
            "application outcome unknown" => Self::Unknown,
            "partial_write" => Self::Partial,
            "blocked by organization policy"
            | "blocked by command policy"
            | "command blocked by policy" => Self::BlockedByPolicy,
            "input arrived before output"
            | "multiple inputs arrived before output"
            | "key release was not confirmed before output"
            | "input quiet check failed"
            | "input changed before expansion was prepared"
            | "command result became stale" => Self::InterruptedByTyping,
            label
                if ["command", "form", "helper", "action broker"]
                    .iter()
                    .any(|prefix| label.starts_with(prefix)) =>
            {
                Self::SnippetActionFailed
            }
            _ => Self::Other,
        }
    }

    /// Whether this outcome put the replacement in place as far as the
    /// route can tell.
    pub fn expanded(self) -> bool {
        matches!(self, Self::Expanded | Self::ExpandedCursorMisplaced)
    }

    /// A stable identifier for JSON output.
    pub fn code(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::ExpandedCursorMisplaced => "expanded_cursor_misplaced",
            Self::NotApplied => "not_applied",
            Self::Unknown => "unknown",
            Self::Partial => "partial",
            Self::BlockedByPolicy => "blocked_by_policy",
            Self::InterruptedByTyping => "interrupted_by_typing",
            Self::SnippetActionFailed => "snippet_action_failed",
            Self::Other => "other",
        }
    }
}

/// One entry of the daemon's recent expansion history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentOutcome {
    pub at_unix: u64,
    pub kind: OutcomeKind,
    /// The daemon's own label, kept for `--json` and bug reports. It is a
    /// fixed string from the daemon, never user content.
    pub completion: String,
}

/// The most likely answer, most fundamental first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing can expand until this is fixed.
    MatchingOff(MatchingBlock),
    /// No keyboard input has reached the daemon since it started.
    NoTypingSeen,
    /// Typing arrives, but nothing typed has matched a trigger.
    NothingMatched,
    /// The newest expansion attempt did not complete.
    LastStopped(OutcomeKind),
    /// The newest expansion was accepted by the route. If the text still
    /// did not appear, the application ignored or changed it.
    LastExpanded,
}

impl Verdict {
    /// A stable identifier for JSON output.
    pub fn code(self) -> &'static str {
        match self {
            Self::MatchingOff(_) => "matching_off",
            Self::NoTypingSeen => "no_typing_seen",
            Self::NothingMatched => "nothing_matched",
            Self::LastStopped(_) => "last_stopped",
            Self::LastExpanded => "last_expanded",
        }
    }
}

/// Everything the status says about recent expansions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionDiagnosis {
    pub matching_block: Option<MatchingBlock>,
    /// The route serves only the configured terminal applications; other
    /// applications are expected to use another route (usually IBus).
    pub terminal_apps_only: bool,
    pub input_events: Option<u64>,
    pub matches_found: Option<u64>,
    /// Expansions stopped before output since the daemon started.
    pub stopped_total: Option<u64>,
    /// Newest first.
    pub recent: Vec<RecentOutcome>,
}

impl ExpansionDiagnosis {
    pub fn from_status(status: &DaemonStatus) -> Self {
        let mut recent: Vec<RecentOutcome> = status
            .field("recent_expansion_outcomes")
            .and_then(|value| serde_json::from_str::<Vec<serde_json::Value>>(value).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| {
                let completion = record.get("completion")?.as_str()?.to_owned();
                Some(RecentOutcome {
                    at_unix: record.get("at_unix")?.as_u64()?,
                    kind: OutcomeKind::from_completion(&completion),
                    completion,
                })
            })
            .collect();
        // The daemon lists oldest first; keep insertion order among equal
        // timestamps by reversing rather than sorting.
        recent.reverse();
        Self {
            matching_block: status
                .field("matching_unavailable_reason")
                .and_then(MatchingBlock::from_reason),
            terminal_apps_only: status.bool_field("terminal_apps_only") == Some(true),
            input_events: status.u64_field("input_events_total"),
            matches_found: status.u64_field("matches_found_total"),
            stopped_total: status.u64_field("suppressed_expansions_total"),
            recent,
        }
    }

    /// The verdict as of `now_unix`. An outcome older than
    /// [`RECENT_OUTCOME_SECS`] is not the attempt the user is asking about:
    /// had their trigger reached this route, a newer outcome would exist.
    pub fn verdict(&self, now_unix: u64) -> Verdict {
        if let Some(block) = self.matching_block {
            return Verdict::MatchingOff(block);
        }
        if let Some(newest) = self
            .recent
            .first()
            .filter(|newest| now_unix.saturating_sub(newest.at_unix) <= RECENT_OUTCOME_SECS)
        {
            return if newest.kind.expanded() {
                Verdict::LastExpanded
            } else {
                Verdict::LastStopped(newest.kind)
            };
        }
        if self.input_events == Some(0) {
            return Verdict::NoTypingSeen;
        }
        Verdict::NothingMatched
    }
}

/// How old the newest outcome may be and still answer "what happened to
/// the trigger I just typed".
pub const RECENT_OUTCOME_SECS: u64 = 120;

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnose(body: &str) -> ExpansionDiagnosis {
        ExpansionDiagnosis::from_status(&DaemonStatus::parse(&format!("running\n{body}")))
    }

    #[test]
    fn a_global_block_comes_first_and_window_scope_is_ignored() {
        let paused = diagnose(
            "matching_unavailable_reason=paused\ninput_events_total=10\n\
             recent_expansion_outcomes=[{\"at_unix\":5,\"completion\":\"applied\"}]\n",
        );
        assert_eq!(
            paused.verdict(5),
            Verdict::MatchingOff(MatchingBlock::Paused)
        );
        // The asking window is a terminal or WayExpand itself.
        let scoped = diagnose(
            "matching_unavailable_reason=application_scope_denied\nterminal_apps_only=true\n\
             input_events_total=0\n",
        );
        assert_eq!(scoped.matching_block, None);
        assert!(scoped.terminal_apps_only);
        assert_eq!(scoped.verdict(0), Verdict::NoTypingSeen);
    }

    #[test]
    fn the_newest_outcome_decides_and_comes_first() {
        let diagnosis = diagnose(
            "matching_unavailable_reason=none\ninput_events_total=40\n\
             recent_expansion_outcomes=[{\"at_unix\":1,\"completion\":\"applied\"},\
             {\"at_unix\":2,\"completion\":\"input arrived before output\"}]\n",
        );
        assert_eq!(diagnosis.recent[0].at_unix, 2);
        assert_eq!(
            diagnosis.verdict(10),
            Verdict::LastStopped(OutcomeKind::InterruptedByTyping)
        );
        let expanded = diagnose(
            "matching_unavailable_reason=none\n\
             recent_expansion_outcomes=[{\"at_unix\":3,\"completion\":\"applied\"}]\n",
        );
        assert_eq!(
            expanded.verdict(3 + RECENT_OUTCOME_SECS),
            Verdict::LastExpanded
        );
        // Hours later, that expansion is not the one being asked about.
        assert_eq!(expanded.verdict(4 * 3600), Verdict::NothingMatched);
    }

    #[test]
    fn typing_without_a_match_points_at_the_trigger() {
        let diagnosis = diagnose(
            "matching_unavailable_reason=none\ninput_events_total=12\nmatches_found_total=0\n\
             recent_expansion_outcomes=[]\n",
        );
        assert_eq!(diagnosis.verdict(0), Verdict::NothingMatched);
        // An older daemon without the history still gets an answer.
        assert_eq!(diagnose("").verdict(0), Verdict::NothingMatched);
    }

    #[test]
    fn every_daemon_completion_label_is_classified() {
        use OutcomeKind as K;
        let cases = [
            ("applied", K::Expanded),
            (
                "inserted; cursor placement failed",
                K::ExpandedCursorMisplaced,
            ),
            ("not applied", K::NotApplied),
            ("application outcome unknown", K::Unknown),
            ("partial_write", K::Partial),
            ("blocked by organization policy", K::BlockedByPolicy),
            ("blocked by command policy", K::BlockedByPolicy),
            ("input arrived before output", K::InterruptedByTyping),
            (
                "multiple inputs arrived before output",
                K::InterruptedByTyping,
            ),
            (
                "key release was not confirmed before output",
                K::InterruptedByTyping,
            ),
            ("input quiet check failed", K::InterruptedByTyping),
            (
                "input changed before expansion was prepared",
                K::InterruptedByTyping,
            ),
            ("command result became stale", K::InterruptedByTyping),
            ("command timed out before output", K::SnippetActionFailed),
            ("command blocked by policy", K::BlockedByPolicy),
            (
                "command output exceeded policy limit",
                K::SnippetActionFailed,
            ),
            ("form requires verified replacement", K::SnippetActionFailed),
            ("helper timed out", K::SnippetActionFailed),
            ("action broker unavailable", K::SnippetActionFailed),
            ("something new", K::Other),
        ];
        for (label, kind) in cases {
            assert_eq!(OutcomeKind::from_completion(label), kind, "{label}");
        }
    }

    #[test]
    fn unknown_reasons_are_reported_not_dropped() {
        assert_eq!(
            MatchingBlock::from_reason("future_reason"),
            Some(MatchingBlock::Other)
        );
        let malformed = diagnose("recent_expansion_outcomes=not json\n");
        assert!(malformed.recent.is_empty());
    }
}
