//! Focused-window tracking: window events into the engine and focus snapshots for the control socket.

use crate::*;

// The shared core limit leaves room in the 1 KiB control frame for the hex
// token, a u64 generation, and the longest valid UTF-8 trigger (128 scalars).
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

impl Daemon {
    /// Route exact application IDs to private local profiles. A transition
    /// pauses matching until the requested library has loaded successfully.
    pub(crate) fn update_profile_for_focus(&mut self) -> Result<()> {
        let app_id = self
            .config
            .engine
            .current_window()
            .and_then(|window| window.app_id.as_deref());
        let profile = self.profile_assignments.profile_for_app(app_id);
        // Runs on every input event: compare without building a path, and
        // allocate the target only when a switch is actually needed.
        if !is_profile_path(self.config.path(), &self.path, profile) {
            let target = profile_path(&self.path, profile);
            if !self.profile_switch_pending {
                process_event(
                    &mut self.config.engine,
                    InputEvent::PauseChanged(true),
                    None,
                    &self.policy,
                    self.active_backend,
                )?;
                self.profile_switch_pending = true;
            }
            self.config.switch_path(target);
        } else if self.profile_switch_pending && self.config.healthy() {
            process_event(
                &mut self.config.engine,
                InputEvent::PauseChanged(self.paused),
                None,
                &self.policy,
                self.active_backend,
            )?;
            self.profile_switch_pending = false;
        }
        Ok(())
    }

    pub(crate) fn reload_profile_assignments(&mut self) {
        let path = profile_root(&self.path).join("applications.toml");
        match wayexpand_core::ProfileAssignments::load(&path) {
            Ok(assignments) => self.profile_assignments = assignments,
            Err(error) => {
                warn!(%error, "application profile assignments rejected; keeping the last valid assignments")
            }
        }
    }
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
    if let Some((window_opt, stamp)) = backend_lifecycle::drain_pending_window_events(receiver) {
        // The engine now reflects this snapshot; keystroke output compares
        // the live focus identity against it before typing.
        crate::assurance::GuardState::global().set_stamp(stamp);
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

/// The configuration a focused application should use: the base library,
/// or `profiles/<name>.toml` beside it.
fn profile_path(base: &std::path::Path, profile: Option<&str>) -> std::path::PathBuf {
    match profile {
        None => base.to_path_buf(),
        Some(name) => profile_root(base).join(format!("{name}.toml")),
    }
}

fn profile_root(base: &std::path::Path) -> std::path::PathBuf {
    base.parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("profiles")
}

/// Whether `active` already equals `profile_path(base, profile)`, decided
/// component by component so the per-event check never allocates.
fn is_profile_path(
    active: &std::path::Path,
    base: &std::path::Path,
    profile: Option<&str>,
) -> bool {
    let Some(name) = profile else {
        return active == base;
    };
    let directory = active.parent();
    active
        .extension()
        .is_some_and(|extension| extension == "toml")
        && active.file_stem().is_some_and(|stem| stem == name)
        && directory
            .and_then(std::path::Path::file_name)
            .is_some_and(|directory| directory == "profiles")
        && directory.and_then(std::path::Path::parent)
            == Some(base.parent().unwrap_or_else(|| std::path::Path::new(".")))
}

#[cfg(test)]
mod tests {
    use super::{focus_token, is_profile_path, profile_path};
    use std::path::Path;
    use wayexpand_core::WindowContext;

    #[test]
    fn profile_path_comparison_matches_the_built_target() {
        let base = Path::new("/home/user/.config/wayexpand/expansions.toml");
        for profile in [None, Some("work"), Some("work.v2"), Some("personal")] {
            let target = profile_path(base, profile);
            assert!(is_profile_path(&target, base, profile), "{profile:?}");
        }
        let work = profile_path(base, Some("work"));
        assert!(!is_profile_path(&work, base, None));
        assert!(!is_profile_path(base, base, Some("work")));
        assert!(!is_profile_path(&work, base, Some("personal")));
        assert!(!is_profile_path(
            Path::new("/home/user/.config/wayexpand/other/work.toml"),
            base,
            Some("work")
        ));
        assert!(!is_profile_path(
            Path::new("/elsewhere/profiles/work.toml"),
            base,
            Some("work")
        ));
        assert!(!is_profile_path(
            Path::new("/home/user/.config/wayexpand/profiles/work.json"),
            base,
            Some("work")
        ));

        let relative = Path::new("expansions.toml");
        let target = profile_path(relative, Some("work"));
        assert_eq!(target, Path::new("profiles/work.toml"));
        assert!(is_profile_path(&target, relative, Some("work")));
    }

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
