//! Self-healing for an output route that stays connected but goes stale.
//!
//! Transport failures already retire the output actor and reconnect. Some
//! routes instead keep a live connection while their view of the keyboard
//! goes wrong, and then refuse every expansion until the daemon restarts:
//! for example a libei route whose active-layout report stopped updating.
//! Those refusals carry a route-state category, not a snippet-content one.
//!
//! After [`STATE_REFUSALS_BEFORE_REFRESH`] such refusals in a row, with no
//! acknowledged expansion between them, the daemon reconnects the output
//! route once so it renegotiates its keyboard state. At most one refresh is
//! made per [`MIN_REFRESH_INTERVAL`], so a genuine, persistent mismatch keeps
//! being refused instead of reconnecting in a loop. The refused expansions
//! are never replayed.

use std::time::{Duration, Instant};

pub(crate) const STATE_REFUSALS_BEFORE_REFRESH: u32 = 3;
pub(crate) const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Refusal categories (see `backend_refusal_category`) that depend on the
/// route's keyboard state, which a new connection resynchronizes. Content
/// refusals such as `unicode-keymap` or a route limit are not included: a
/// reconnect cannot make the route type text it cannot represent.
pub(crate) fn is_route_state_refusal(category: &str) -> bool {
    matches!(
        category,
        "layout-mismatch" | "active-layout-unavailable" | "keyboard-state-busy"
    )
}

#[derive(Debug, Default)]
pub(crate) struct Detector {
    consecutive: u32,
    last_refresh: Option<Instant>,
}

impl Detector {
    pub(crate) const fn new() -> Self {
        Self {
            consecutive: 0,
            last_refresh: None,
        }
    }

    /// An expansion was acknowledged, or the route was (re)connected.
    pub(crate) fn reset(&mut self) {
        self.consecutive = 0;
    }

    /// Count one route-state refusal. Returns true when the route should be
    /// reconnected now.
    pub(crate) fn note_state_refusal(&mut self, now: Instant) -> bool {
        self.consecutive = self.consecutive.saturating_add(1);
        if self.consecutive < STATE_REFUSALS_BEFORE_REFRESH {
            return false;
        }
        if self
            .last_refresh
            .is_some_and(|last| now.saturating_duration_since(last) < MIN_REFRESH_INTERVAL)
        {
            return false;
        }
        self.consecutive = 0;
        self.last_refresh = Some(now);
        true
    }
}

// Outcomes are resolved and refusals handled on the reactor thread, so the
// count lives there. A thread-local also keeps parallel tests independent.
thread_local! {
    static DETECTOR: std::cell::RefCell<Detector> = const { std::cell::RefCell::new(Detector::new()) };
}

pub(crate) fn note_healthy() {
    DETECTOR.with(|detector| detector.borrow_mut().reset());
}

pub(crate) fn note_state_refusal() -> bool {
    DETECTOR.with(|detector| detector.borrow_mut().note_state_refusal(Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_route_state_categories_trigger_a_refresh() {
        assert!(is_route_state_refusal("layout-mismatch"));
        assert!(is_route_state_refusal("active-layout-unavailable"));
        assert!(is_route_state_refusal("keyboard-state-busy"));
        assert!(!is_route_state_refusal("unicode-keymap"));
        assert!(!is_route_state_refusal("fallback-too-slow"));
    }

    #[test]
    fn refreshes_after_consecutive_refusals_once_per_interval() {
        let mut detector = Detector::new();
        let start = Instant::now();
        assert!(!detector.note_state_refusal(start));
        assert!(!detector.note_state_refusal(start));
        assert!(detector.note_state_refusal(start));

        // A persistent mismatch is not reconnected again within the interval.
        for _ in 0..10 {
            assert!(!detector.note_state_refusal(start + Duration::from_secs(1)));
        }
        let later = start + MIN_REFRESH_INTERVAL;
        assert!(detector.note_state_refusal(later));
    }

    #[test]
    fn an_acknowledged_expansion_restarts_the_count() {
        let mut detector = Detector::new();
        let now = Instant::now();
        assert!(!detector.note_state_refusal(now));
        assert!(!detector.note_state_refusal(now));
        detector.reset();
        assert!(!detector.note_state_refusal(now));
        assert!(!detector.note_state_refusal(now));
        assert!(detector.note_state_refusal(now));
    }
}
