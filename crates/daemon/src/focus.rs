//! Focused-window tracking: window events into the engine and focus snapshots for the control socket.

use crate::*;

/// The last published focused window and a counter bumped on every change,
/// so the quick-insert picker can tell focus left and came back.
pub(crate) struct FocusState {
    previous: Option<WindowContext>,
    generation: u64,
}

impl FocusState {
    /// Start tracking from the window the engine already knows about.
    pub(crate) fn new(previous: Option<WindowContext>) -> Self {
        Self {
            previous,
            generation: 0,
        }
    }
}

pub(crate) fn focus_token(window: &WindowContext) -> String {
    let mut hasher = DefaultHasher::new();
    window.app_id.hash(&mut hasher);
    window.title.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn publish_focus_snapshot(control: &ControlServer, state: &FocusState) {
    control.set_focus_snapshot(FocusSnapshot {
        generation: state.generation,
        token: state.previous.as_ref().map(focus_token),
    });
}

/// Drain any pending window-change events from the tracker's receiver
/// and apply them to the engine. This prevents app-filter races where a
/// focus change arrives between input-event wait and processing.
pub(crate) fn drain_pending_window_events(
    window_tracker: &Option<backend_lifecycle::WindowTrackerHandle>,
    engine: &mut ExpansionEngine,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
    control: &ControlServer,
    focus_state: &mut FocusState,
) -> Result<()> {
    let receiver = window_tracker.as_ref().map(|tracker| &tracker.receiver);
    if let Some(window_opt) = backend_lifecycle::drain_pending_window_events(receiver) {
        process_event(
            engine,
            InputEvent::WindowChanged(window_opt),
            None,
            policy,
            active_backend,
        )?;
    }
    // Runs on every loop iteration: compare by reference and publish only on
    // an actual focus change.
    if focus_state.previous.as_ref() != engine.current_window() {
        focus_state.previous = engine.current_window().cloned();
        focus_state.generation = focus_state.generation.wrapping_add(1);
        publish_focus_snapshot(control, focus_state);
    }
    Ok(())
}
