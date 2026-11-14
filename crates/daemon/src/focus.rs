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
    let diagnostic_app_id = state
        .previous
        .as_ref()
        .and_then(|window| window.app_id.as_deref())
        .and_then(sanitize_diagnostic_app_id);
    control.set_focus_snapshot(FocusSnapshot {
        generation: state.generation,
        exact_window_identity: token.is_some(),
        token,
        diagnostic_app_id,
    });
}

/// App IDs are exposed only through an explicit, local diagnostic command.
/// Keep the response single-line and bounded, and reject non-identifier data.
fn sanitize_diagnostic_app_id(app_id: &str) -> Option<String> {
    if let Some(path) = app_id.strip_prefix(wayexpand_core::PROCESS_EXECUTABLE_APP_ID_PREFIX) {
        let executable = std::path::Path::new(path);
        if path.is_empty()
            || path.chars().count() > wayexpand_core::limits::MAX_PROCESS_EXECUTABLE_PATH_CHARS
            || !executable.is_absolute()
            || path.chars().any(char::is_control)
            || executable.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
            || path.split('/').skip(1).any(str::is_empty)
        {
            return None;
        }
        return Some(app_id.to_owned());
    }
    if app_id.is_empty()
        || app_id.len() > 128
        || !app_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some(app_id.to_ascii_lowercase())
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
        crate::status::record_window_tracker_update(window_opt.as_ref());
        // Process the new identity before moving the output guard's baseline.
        // While this event is being applied, any in-flight output still
        // compares its immutable transaction stamp with the tracker's live
        // identity and therefore stops on a focus change.
        apply_focus_change_before_advancing_guard(
            &crate::assurance::GuardState::global(),
            stamp,
            || {
                process_event(
                    engine,
                    InputEvent::WindowChanged(window_opt),
                    None,
                    policy,
                    active_backend,
                )
            },
        )?;
    }
    // Runs on every loop iteration: compare by reference and publish only on
    // an actual focus change.
    if focus_state.previous.as_ref() != engine.current_window() {
        focus_state.previous = engine.current_window().cloned();
        focus_state.generation = focus_state.generation.wrapping_add(1);
        publish_focus_snapshot(control, focus_state);
    }
    let focus_known = window_tracker
        .as_ref()
        .is_some_and(backend_lifecycle::WindowTrackerHandle::is_connected)
        && engine
            .current_window()
            .and_then(|window| window.app_id.as_deref())
            .is_some();
    crate::status::set_capture_app_scope_state(
        engine.terminal_apps_only(),
        focus_known,
        !engine.terminal_apps_only() || engine.focused_app_is_terminal(),
    );
    Ok(())
}

/// Keep output checkpoints invalidated until the input engine has applied the
/// same focus snapshot. Advancing the shared guard first can make an in-flight
/// transaction compare against its new target and continue into that window.
fn apply_focus_change_before_advancing_guard(
    guard: &crate::assurance::GuardState,
    stamp: wayexpand_core::FocusStamp,
    apply: impl FnOnce() -> std::result::Result<(), Box<EventError>>,
) -> std::result::Result<(), Box<EventError>> {
    let result = apply();
    // Advance even when applying failed: the engine has already seen the new
    // window, and a guard left on the old stamp would refuse every later
    // output as "focused window changed" until the next focus change.
    guard.set_stamp(stamp);
    result
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
    use super::{
        apply_focus_change_before_advancing_guard, focus_token, is_profile_path, profile_path,
        sanitize_diagnostic_app_id,
    };
    use std::path::Path;
    use wayexpand_core::{window_update_channel, FocusCheck, WindowContext};

    #[test]
    fn focus_guard_remains_invalidated_until_engine_applies_the_new_window() {
        let guard = crate::assurance::GuardState::default();
        let (sender, receiver) = window_update_channel();
        guard.set_witness(Some(receiver.focus_witness()));
        sender.send(window("window-a")).unwrap();
        let (_, original_stamp) = receiver.try_recv_stamped().unwrap();
        guard.set_stamp(original_stamp);

        sender.send(window("window-b")).unwrap();
        let (_, new_stamp) = receiver.try_recv_stamped().unwrap();
        assert_eq!(guard.focus(), FocusCheck::Changed);

        apply_focus_change_before_advancing_guard(&guard, new_stamp, || {
            assert_eq!(
                guard.focus(),
                FocusCheck::Changed,
                "an output in flight must remain tied to the prior target during engine update"
            );
            Ok(())
        })
        .unwrap();

        assert_eq!(guard.focus(), FocusCheck::Unchanged);
    }

    fn window(instance: &str) -> Option<WindowContext> {
        Some(WindowContext {
            app_id: Some("org.example.Editor".into()),
            title: Some("document".into()),
            instance_id: Some(instance.into()),
        })
    }

    #[test]
    fn diagnostic_app_id_is_bounded_and_single_line() {
        assert_eq!(
            sanitize_diagnostic_app_id("Org.KDE.Konsole").as_deref(),
            Some("org.kde.konsole")
        );
        assert_eq!(sanitize_diagnostic_app_id("bad\nid"), None);
        assert_eq!(sanitize_diagnostic_app_id(&"x".repeat(129)), None);
    }

    #[test]
    fn explicit_focus_diagnostic_accepts_only_sanitized_process_executable_identity() {
        let prefix = wayexpand_core::PROCESS_EXECUTABLE_APP_ID_PREFIX;
        assert_eq!(
            sanitize_diagnostic_app_id(&format!("{prefix}/usr/bin/konsole")).as_deref(),
            Some("process-executable:/usr/bin/konsole")
        );
        assert_eq!(
            sanitize_diagnostic_app_id(&format!("{prefix}relative/konsole")),
            None
        );
        assert_eq!(
            sanitize_diagnostic_app_id(&format!("{prefix}//usr/bin/konsole")),
            None
        );
        assert_eq!(
            sanitize_diagnostic_app_id(&format!("{prefix}/usr/bin/../bin/konsole")),
            None
        );
        assert_eq!(
            sanitize_diagnostic_app_id(&format!("{prefix}/usr/bin/konsole\ninjected")),
            None
        );
    }

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
