//! Focused-window tracking: window events into the engine and focus snapshots for the control socket.

use crate::*;

// Leaves room in the 1 KiB control frame for the hex token, a u64 generation,
// and the longest valid UTF-8 trigger (128 scalars).
const MAX_WINDOW_INSTANCE_ID_BYTES: usize = 192;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

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

/// Encode a compositor-issued window identity without hashing it: picker
/// targeting must not collapse distinct IDs through a finite-width hash.
/// Missing, empty, and unreasonably large backend identities are unavailable
/// for exact targeting and therefore make picker insertion clipboard-only.
pub(crate) fn focus_token(window: &WindowContext) -> Option<String> {
    let identity = window.instance_id.as_deref()?.as_bytes();
    if identity.is_empty() || identity.len() > MAX_WINDOW_INSTANCE_ID_BYTES {
        return None;
    }

    let mut token = String::with_capacity(identity.len() * 2);
    for byte in identity {
        token.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        token.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    Some(token)
}

pub(crate) fn publish_focus_snapshot(control: &ControlServer, state: &FocusState) {
    let token = state.previous.as_ref().and_then(focus_token);
    control.set_focus_snapshot(FocusSnapshot {
        generation: state.generation,
        exact_window_identity: token.is_some(),
        token,
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

#[cfg(test)]
mod tests {
    use super::focus_token;
    use wayexpand_core::WindowContext;

    #[test]
    fn exact_window_tokens_differ_for_same_app_and_title() {
        let first = WindowContext {
            app_id: Some("org.kde.konsole".into()),
            title: Some("bash".into()),
            instance_id: Some("window-a".into()),
        };
        let second = WindowContext {
            instance_id: Some("window-b".into()),
            ..first.clone()
        };
        let first_token = focus_token(&first).unwrap();
        let second_token = focus_token(&second).unwrap();
        assert_ne!(first_token, second_token);
        assert_eq!(first_token, "77696e646f772d61");
    }

    #[test]
    fn empty_missing_and_oversized_window_ids_are_not_exact_identity() {
        let missing = WindowContext {
            app_id: Some("org.example.Editor".into()),
            title: Some("Document".into()),
            instance_id: None,
        };
        let empty = WindowContext {
            instance_id: Some(String::new()),
            ..missing.clone()
        };
        let oversized = WindowContext {
            instance_id: Some("x".repeat(super::MAX_WINDOW_INSTANCE_ID_BYTES + 1)),
            ..missing
        };

        assert_eq!(focus_token(&empty), None);
        assert_eq!(focus_token(&oversized), None);
        assert_eq!(
            focus_token(&WindowContext {
                instance_id: None,
                ..empty
            }),
            None
        );
    }

    #[test]
    fn exact_window_encoding_is_injective_for_distinct_ids() {
        let first = WindowContext {
            instance_id: Some("window-α".into()),
            app_id: None,
            title: None,
        };
        let second = WindowContext {
            instance_id: Some("window-β".into()),
            ..first.clone()
        };
        assert_ne!(focus_token(&first), focus_token(&second));
    }
}
