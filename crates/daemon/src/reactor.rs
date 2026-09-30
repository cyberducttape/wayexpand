//! Explicit control-plane transitions for the daemon reactor.
//!
//! Control flags are sampled at the start of each iteration, before reload
//! and input work. Keeping that ordering in one small type makes it visible
//! and unit-testable without entangling it with compositor I/O.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactorTransition {
    Continue {
        pause_changed: Option<bool>,
        reload: bool,
    },
    Stop {
        pause_changed: Option<bool>,
        reload: bool,
    },
}

impl ReactorTransition {
    pub fn sample(
        stop: &AtomicBool,
        pause: &AtomicBool,
        reload: &AtomicBool,
        paused: bool,
    ) -> Self {
        let requested_pause = pause.load(Ordering::Acquire);
        let pause_changed = (requested_pause != paused).then_some(requested_pause);
        // Preserve ordering: consume reload before observing stop.
        let reload = reload.swap(false, Ordering::AcqRel);
        if stop.load(Ordering::Acquire) {
            Self::Stop {
                pause_changed,
                reload,
            }
        } else {
            Self::Continue {
                pause_changed,
                reload,
            }
        }
    }

    pub fn pause_changed(self) -> Option<bool> {
        match self {
            Self::Continue { pause_changed, .. } | Self::Stop { pause_changed, .. } => {
                pause_changed
            }
        }
    }

    pub fn reload_requested(self) -> bool {
        match self {
            Self::Continue { reload, .. } | Self::Stop { reload, .. } => reload,
        }
    }

    pub fn should_stop(self) -> bool {
        matches!(self, Self::Stop { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_pause_reload_and_stop_once_in_reactor_order() {
        let stop = AtomicBool::new(false);
        let pause = AtomicBool::new(true);
        let reload = AtomicBool::new(true);
        let transition = ReactorTransition::sample(&stop, &pause, &reload, false);
        assert_eq!(
            transition,
            ReactorTransition::Continue {
                pause_changed: Some(true),
                reload: true
            }
        );
        assert!(!reload.load(Ordering::Acquire));
        assert!(!transition.should_stop());
    }

    #[test]
    fn stop_transition_still_reports_pending_pause_and_reload() {
        let stop = AtomicBool::new(true);
        let pause = AtomicBool::new(false);
        let reload = AtomicBool::new(true);
        let transition = ReactorTransition::sample(&stop, &pause, &reload, true);
        assert_eq!(transition.pause_changed(), Some(false));
        assert!(transition.reload_requested());
        assert!(transition.should_stop());
        assert!(!reload.load(Ordering::Acquire));
    }
}
