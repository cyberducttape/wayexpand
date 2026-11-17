//! One plain-language problem and one next action.
//!
//! Every way expansion can stop working is reduced here to what a user can
//! act on: the service is off, the desktop withheld a permission, GNOME has
//! not selected the input source, the running service is an older build,
//! replacement is not allowed yet, the typing route failed, or the app's
//! field cannot have text replaced. The banner under the header, the status
//! chip and the first-run guide all show the same answer, with one button.
//! Diagnostics stay available, but are never the first thing offered.

use crate::*;
use wayexpand_backend_ibus::chain::IbusCheckState;

/// What is wrong, in the user's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Problem {
    ServiceOff,
    /// Only the control service runs (Quick Picker, hotkeys): nothing
    /// captures typing, because the IBus engine it pairs with is not live.
    NoTypingRoute,
    Paused,
    PermissionNeeded,
    PermissionRevoked,
    RestartNeeded,
    BuildUnknown,
    InputSourceNotSetUp,
    InputSourceNotSelected,
    InputStatusPending,
    IbusEngineUnavailable,
    InputStatusStale,
    ReplacementNotAllowed,
    Connecting,
    RouteFailed,
    Unsupported,
    FieldCannotReplace,
}

/// The one thing to do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    TurnOn,
    Resume,
    RunSetup,
    Restart,
    SelectInputSource,
    ReviewConsent,
    CheckAgain,
    QuickPicker,
}

/// One authoritative interpretation of GNOME IBus evidence. Both the
/// first-run guide and the next-step adviser use this result so they cannot
/// disagree about whether the route is ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GnomeIbusReadiness {
    Ready,
    Paused,
    DiagnosticsPending,
    DiagnosticsStale,
    SourceNotSetUp,
    SourceNotSelected,
    EngineUnavailable,
    ReplacementNotAllowed,
}

/// The facts `next_step` decides from, gathered from the GUI's last status
/// and diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Situation {
    pub daemon_reachable: Option<bool>,
    /// The WayExpand IBus engine is live.
    pub ibus_active: bool,
    /// IBus' global engine selection, separate from a live engine process.
    /// A running WayExpand process can remain alive after another engine is
    /// selected, so process activity alone is not route readiness.
    pub ibus_selection: IbusCheckState,
    /// The daemon only serves the control socket (the IBus route captures).
    pub control_only: bool,
    /// GNOME, judged by its IBus input source.
    pub gnome_ibus: bool,
    pub source_registered: bool,
    pub source_active: bool,
    /// None means no diagnostics have arrived yet; false means the report is
    /// old or belongs to a different IBus engine process.
    pub diagnostics_fresh: Option<bool>,
    pub paused: bool,
    pub route_state: Option<runtime::RouteState>,
    pub build: Option<runtime::DaemonBuildState>,
    /// Replacement is admitted: atomic and verified, or explicitly allowed.
    pub replacement_allowed: bool,
    /// The last normal field refused replacement (no surrounding text).
    pub field_cannot_replace: bool,
}

pub(crate) fn gnome_ibus_readiness(s: &Situation) -> GnomeIbusReadiness {
    if s.paused {
        return GnomeIbusReadiness::Paused;
    }
    match s.diagnostics_fresh {
        None => return GnomeIbusReadiness::DiagnosticsPending,
        Some(false) => return GnomeIbusReadiness::DiagnosticsStale,
        Some(true) => {}
    }
    if !s.source_registered {
        return GnomeIbusReadiness::SourceNotSetUp;
    }
    if !s.source_active {
        return GnomeIbusReadiness::SourceNotSelected;
    }
    if !s.ibus_active {
        return GnomeIbusReadiness::EngineUnavailable;
    }
    if !s.replacement_allowed {
        return GnomeIbusReadiness::ReplacementNotAllowed;
    }
    GnomeIbusReadiness::Ready
}

/// The most important problem and its action, or `None` when expansion
/// should work (or the state is still being checked).
pub(crate) fn next_step(s: &Situation) -> Option<(Problem, Action)> {
    use runtime::RouteState as R;
    let field = || {
        s.field_cannot_replace
            .then_some((Problem::FieldCannotReplace, Action::QuickPicker))
    };
    if s.gnome_ibus {
        return match gnome_ibus_readiness(s) {
            GnomeIbusReadiness::Ready => field(),
            GnomeIbusReadiness::Paused => Some((Problem::Paused, Action::Resume)),
            GnomeIbusReadiness::DiagnosticsPending => {
                Some((Problem::InputStatusPending, Action::CheckAgain))
            }
            GnomeIbusReadiness::DiagnosticsStale => {
                Some((Problem::InputStatusStale, Action::CheckAgain))
            }
            GnomeIbusReadiness::SourceNotSetUp => {
                Some((Problem::InputSourceNotSetUp, Action::RunSetup))
            }
            GnomeIbusReadiness::SourceNotSelected => {
                Some((Problem::InputSourceNotSelected, Action::SelectInputSource))
            }
            GnomeIbusReadiness::EngineUnavailable => {
                Some((Problem::IbusEngineUnavailable, Action::RunSetup))
            }
            GnomeIbusReadiness::ReplacementNotAllowed => {
                Some((Problem::ReplacementNotAllowed, Action::ReviewConsent))
            }
        };
    }
    // The IBus route without a typing daemon (KDE): the engine is the route.
    if s.ibus_active && (s.daemon_reachable != Some(true) || s.control_only) {
        if s.daemon_reachable == Some(true) && s.paused {
            return Some((Problem::Paused, Action::Resume));
        }
        match s.ibus_selection {
            IbusCheckState::Fail | IbusCheckState::Warn => {
                return Some((Problem::InputSourceNotSelected, Action::RunSetup));
            }
            IbusCheckState::Unknown | IbusCheckState::Skip => {
                return Some((Problem::InputStatusPending, Action::CheckAgain));
            }
            IbusCheckState::Pass => {}
        }
        if !s.replacement_allowed {
            return Some((Problem::ReplacementNotAllowed, Action::ReviewConsent));
        }
        return field();
    }
    match s.daemon_reachable {
        None => return None,
        Some(false) => return Some((Problem::ServiceOff, Action::TurnOn)),
        // The control-only service exists to sit beside IBus; reaching here
        // means the IBus engine is not live, so nothing reads typing.
        Some(true) if s.control_only => {
            return Some((Problem::NoTypingRoute, Action::RunSetup));
        }
        Some(true) => {}
    }
    match s.build {
        Some(runtime::DaemonBuildState::RestartRequired) => {
            return Some((Problem::RestartNeeded, Action::Restart))
        }
        Some(runtime::DaemonBuildState::Unverified) => {
            return Some((Problem::BuildUnknown, Action::Restart))
        }
        _ => {}
    }
    if s.paused {
        return Some((Problem::Paused, Action::Resume));
    }
    match s.route_state? {
        R::Connected | R::Degraded => {
            if !s.replacement_allowed {
                Some((Problem::ReplacementNotAllowed, Action::ReviewConsent))
            } else {
                field()
            }
        }
        R::PermissionRequired => Some((Problem::PermissionNeeded, Action::RunSetup)),
        R::PortalRevoked => Some((Problem::PermissionRevoked, Action::RunSetup)),
        R::Starting | R::Reconnecting => Some((Problem::Connecting, Action::CheckAgain)),
        R::Failed | R::Stopped => Some((Problem::RouteFailed, Action::RunSetup)),
        R::Unsupported => Some((Problem::Unsupported, Action::QuickPicker)),
    }
}

impl GuiApp {
    fn ibus_check_state(&self, id: &str) -> IbusCheckState {
        self.ibus_route
            .as_ref()
            .and_then(|report| report.check(id))
            .map_or(IbusCheckState::Unknown, |check| check.state)
    }

    /// Whether current, process-correlated diagnostics show IBus selected as
    /// the typing route. A live engine process alone is insufficient.
    pub(crate) fn ibus_route_active(&self) -> bool {
        self.ibus_active
            && self.ibus_diagnostics_fresh() == Some(true)
            && self.ibus_check_state("selected") == IbusCheckState::Pass
            && (wayexpand_backend_selection::Compositor::detect()
                != wayexpand_backend_selection::Compositor::Gnome
                || self.ibus_check_state("gnome-source-active") == IbusCheckState::Pass)
    }

    /// The service unit to restart for the running route, when known.
    pub(crate) fn active_service_unit(&self) -> Option<&'static str> {
        match runtime::status_field(&self.daemon_status, "source").as_deref() {
            Some("evdev") => Some("wayexpand-evdev.service"),
            Some("input-method" | "input-method-v2") => Some("wayexpand-input-method.service"),
            Some("stdin") => Some("wayexpand-control.service"),
            _ => None,
        }
    }

    pub(crate) fn situation(&self) -> Situation {
        let daemon_running = self.daemon_reachable == Some(true);
        let control_only =
            runtime::status_field(&self.daemon_status, "source").as_deref() == Some("stdin");
        let active_daemon_source = daemon_running
            .then(|| runtime::status_field(&self.daemon_status, "source"))
            .flatten();
        let gnome_ibus = super::first_run::uses_gnome_ibus_route(
            wayexpand_backend_selection::Compositor::detect()
                == wayexpand_backend_selection::Compositor::Gnome,
            self.ibus_check_state("gnome-source-active") == IbusCheckState::Pass,
            self.ibus_check_state("gnome-source-registered") == IbusCheckState::Pass,
            self.recommended_route
                .is_some_and(|route| route.setup_backend() == "ibus"),
            self.daemon_reachable,
            active_daemon_source.as_deref(),
        );
        // Keep process health separate from source selection: GNOME can have
        // the source selected while its engine process has crashed.
        let ibus_active = self.ibus_active;
        let ibus_route = gnome_ibus || ibus_active && (!daemon_running || control_only);
        let replacement_allowed = if ibus_route {
            // IBus replacement is non-atomic. Unknown consent is not consent,
            // and must agree with the first-run gate which requires Pass.
            self.ibus_check_state("consent") == IbusCheckState::Pass
        } else {
            self.daemon_capabilities.as_ref().is_none_or(|caps| {
                caps.inject_atomic_replace == Some(true)
                    && caps.replacement_assurance == Some(runtime::ReplacementAssurance::Verified)
                    || caps
                        .compatibility_mode
                        .is_some_and(|mode| mode != wayexpand_core::CompatibilityMode::Off)
            })
        };
        Situation {
            daemon_reachable: self.daemon_reachable,
            ibus_active,
            ibus_selection: if self.ibus_diagnostics_fresh() == Some(true) {
                self.ibus_check_state("selected")
            } else {
                IbusCheckState::Unknown
            },
            control_only,
            gnome_ibus,
            source_registered: self.ibus_check_state("gnome-source-registered")
                == IbusCheckState::Pass,
            source_active: self.ibus_check_state("gnome-source-active") == IbusCheckState::Pass,
            diagnostics_fresh: self.ibus_diagnostics_fresh(),
            paused: self.paused,
            route_state: self.route_state,
            build: daemon_running.then(|| runtime::daemon_build_state(&self.daemon_status)),
            replacement_allowed,
            field_cannot_replace: ibus_route
                && self.ibus_check_state("surrounding-text") == IbusCheckState::Fail,
        }
    }

    /// Selection evidence is valid only for the current engine process and
    /// for one minute. A previous IBus context can remain alive after a
    /// desktop switches to a different input method.
    fn ibus_diagnostics_fresh(&self) -> Option<bool> {
        let checked_at = self.diagnostics_checked_at?;
        let recent = checked_at.elapsed() < Duration::from_secs(60);
        if !recent {
            return Some(false);
        }
        // A live process identity changes on every engine restart. Do not
        // reuse source-selection evidence collected from the previous run.
        if self.ibus_active {
            let same_instance = self
                .ibus_route
                .as_ref()
                .and_then(|report| report.activity.as_ref())
                .filter(|activity| activity.live)
                .map(|activity| {
                    format!(
                        "ibus:{}:{}",
                        activity.status.pid, activity.status.started_unix
                    )
                })
                .as_deref()
                == self.ibus_instance.as_deref();
            return Some(same_instance);
        }
        Some(true)
    }

    pub(crate) fn current_next_step(&self) -> Option<(Problem, Action)> {
        next_step(&self.situation())
    }

    /// Do `action`. Each runs an existing, confirmed path: guided setup,
    /// the service-restart confirmation, the GNOME source repair, the
    /// copy-only picker.
    pub(crate) fn perform_next_step(&mut self, action: Action) {
        match action {
            Action::TurnOn | Action::RunSetup => self.start_guided_setup(),
            Action::ReviewConsent => self.review_replacement_consent(),
            Action::Resume => self.toggle_pause(),
            Action::Restart => match self.active_service_unit() {
                Some(unit) => {
                    self.service_restart_unit = Some(unit.to_owned());
                    self.service_restart_confirm_open = true;
                }
                None => self.start_guided_setup(),
            },
            Action::SelectInputSource => self.retry_gnome_source_activation(),
            Action::CheckAgain => self.refresh_diagnostics(true),
            Action::QuickPicker => self.open_copy_only_picker(),
        }
    }

    /// Send consent to the control that governs the selected route. IBus
    /// consent belongs in its guided registration flow; keystroke-route
    /// consent belongs in Engine settings. An organization-policy refusal
    /// belongs in diagnostics because a user preference cannot override it.
    pub(crate) fn review_replacement_consent(&mut self) {
        let situation = self.situation();
        let configured_backend = super::setup::current_guided_setup_backend(
            self.daemon_reachable,
            &self.daemon_status,
            self.route_plan.as_ref(),
        );
        let ibus_route = configured_backend.is_none()
            && (situation.gnome_ibus
                || situation.ibus_active
                    && (situation.daemon_reachable != Some(true) || situation.control_only)
                || self
                    .recommended_route
                    .is_some_and(|route| route.setup_backend() == "ibus"));
        let policy_blocked =
            runtime::status_field(&self.daemon_status, "matching_unavailable_reason").as_deref()
                == Some("organization_policy_route_incompatible");
        if policy_blocked {
            self.diagnostics_open = true;
            self.refresh_diagnostics(false);
        } else if ibus_route {
            self.start_guided_setup();
        } else {
            self.open_compatibility_settings();
        }
    }

    /// One line under the header: what is wrong and one button. Hidden
    /// while the first-run guide shows the same thing in its first step.
    pub(crate) fn render_next_step_banner(&mut self, root: &mut egui::Ui, palette: &Palette) {
        if self.onboarding_visible {
            return;
        }
        let Some((problem, action)) = self.current_next_step() else {
            return;
        };
        let busy = self.setup_task.is_some();
        egui::Panel::top("next_step")
            .frame(
                egui::Frame::new()
                    .fill(theme::tint(palette.warning, 26))
                    .inner_margin(egui::Margin::symmetric(22, 8)),
            )
            .show(root, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(self.strings.problem_text(problem))
                            .color(ui.visuals().strong_text_color()),
                    );
                    if ui
                        .add_enabled(!busy, egui::Button::new(self.strings.action_label(action)))
                        .clicked()
                    {
                        self.perform_next_step(action);
                    }
                    if ui.link(self.strings.next_step_details()).clicked() {
                        self.diagnostics_open = true;
                        self.refresh_diagnostics(false);
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        gnome_ibus_readiness, next_step, Action, GnomeIbusReadiness, IbusCheckState, Problem,
        Situation,
    };
    use crate::runtime::{DaemonBuildState, RouteState};

    fn running(route: RouteState) -> Situation {
        Situation {
            daemon_reachable: Some(true),
            route_state: Some(route),
            build: Some(DaemonBuildState::Current),
            replacement_allowed: true,
            ..Situation::default()
        }
    }

    #[test]
    fn each_failure_has_one_action() {
        let cases = [
            (
                Situation {
                    daemon_reachable: Some(false),
                    ..Situation::default()
                },
                Some((Problem::ServiceOff, Action::TurnOn)),
            ),
            (
                running(RouteState::PermissionRequired),
                Some((Problem::PermissionNeeded, Action::RunSetup)),
            ),
            (
                running(RouteState::PortalRevoked),
                Some((Problem::PermissionRevoked, Action::RunSetup)),
            ),
            (
                running(RouteState::Reconnecting),
                Some((Problem::Connecting, Action::CheckAgain)),
            ),
            (
                running(RouteState::Failed),
                Some((Problem::RouteFailed, Action::RunSetup)),
            ),
            (
                running(RouteState::Unsupported),
                Some((Problem::Unsupported, Action::QuickPicker)),
            ),
            // Quick Picker and hotkeys only: a running process, but nothing
            // reads typing, so it must never pass as connected.
            (
                Situation {
                    control_only: true,
                    ..running(RouteState::Connected)
                },
                Some((Problem::NoTypingRoute, Action::RunSetup)),
            ),
            // Beside a live IBus engine the same service is the normal
            // KDE route, and IBus decides.
            (
                Situation {
                    control_only: true,
                    ibus_active: true,
                    ibus_selection: IbusCheckState::Pass,
                    replacement_allowed: true,
                    ..running(RouteState::Connected)
                },
                None,
            ),
            (
                Situation {
                    paused: true,
                    ..running(RouteState::Connected)
                },
                Some((Problem::Paused, Action::Resume)),
            ),
            (
                Situation {
                    build: Some(DaemonBuildState::RestartRequired),
                    ..running(RouteState::Connected)
                },
                Some((Problem::RestartNeeded, Action::Restart)),
            ),
            (
                Situation {
                    replacement_allowed: false,
                    ..running(RouteState::Connected)
                },
                Some((Problem::ReplacementNotAllowed, Action::ReviewConsent)),
            ),
            (running(RouteState::Connected), None),
        ];
        for (situation, expected) in cases {
            assert_eq!(next_step(&situation), expected, "{situation:?}");
        }
    }

    #[test]
    fn unknown_general_state_is_not_claimed_but_gnome_diagnostics_get_a_refresh_action() {
        assert_eq!(next_step(&Situation::default()), None);
        assert_eq!(
            next_step(&Situation {
                gnome_ibus: true,
                ..Situation::default()
            }),
            Some((Problem::InputStatusPending, Action::CheckAgain))
        );
    }

    #[test]
    fn gnome_walks_through_its_input_source_in_order() {
        let gnome = Situation {
            gnome_ibus: true,
            ibus_active: true,
            replacement_allowed: true,
            diagnostics_fresh: Some(true),
            ..Situation::default()
        };
        assert_eq!(
            next_step(&gnome),
            Some((Problem::InputSourceNotSetUp, Action::RunSetup))
        );
        let registered = Situation {
            source_registered: true,
            ..gnome
        };
        assert_eq!(
            next_step(&registered),
            Some((Problem::InputSourceNotSelected, Action::SelectInputSource))
        );
        let selected = Situation {
            source_active: true,
            ..registered
        };
        assert_eq!(next_step(&selected), None);
        assert_eq!(
            next_step(&Situation {
                field_cannot_replace: true,
                ..selected
            }),
            Some((Problem::FieldCannotReplace, Action::QuickPicker))
        );
    }

    #[test]
    fn gnome_engine_crash_is_not_reported_as_ready() {
        let crashed = Situation {
            gnome_ibus: true,
            ibus_active: false,
            source_registered: true,
            source_active: true,
            diagnostics_fresh: Some(true),
            replacement_allowed: true,
            ..Situation::default()
        };
        assert_eq!(
            gnome_ibus_readiness(&crashed),
            GnomeIbusReadiness::EngineUnavailable
        );
        assert_eq!(
            next_step(&crashed),
            Some((Problem::IbusEngineUnavailable, Action::RunSetup))
        );
    }

    #[test]
    fn unknown_ibus_consent_never_looks_ready_or_configured() {
        let unknown_consent = Situation {
            gnome_ibus: true,
            ibus_active: true,
            source_registered: true,
            source_active: true,
            diagnostics_fresh: Some(true),
            replacement_allowed: false,
            ..Situation::default()
        };
        assert_eq!(
            gnome_ibus_readiness(&unknown_consent),
            GnomeIbusReadiness::ReplacementNotAllowed
        );
        assert_eq!(
            next_step(&unknown_consent),
            Some((Problem::ReplacementNotAllowed, Action::ReviewConsent))
        );
    }

    #[test]
    fn gnome_source_deselection_has_a_corrective_action() {
        let deselected = Situation {
            gnome_ibus: true,
            ibus_active: true,
            source_registered: true,
            source_active: false,
            diagnostics_fresh: Some(true),
            replacement_allowed: true,
            ..Situation::default()
        };
        assert_eq!(
            gnome_ibus_readiness(&deselected),
            GnomeIbusReadiness::SourceNotSelected
        );
        assert_eq!(
            next_step(&deselected),
            Some((Problem::InputSourceNotSelected, Action::SelectInputSource))
        );
    }

    #[test]
    fn stale_gnome_diagnostics_request_a_refresh_instead_of_claiming_ready() {
        let stale = Situation {
            gnome_ibus: true,
            ibus_active: true,
            source_registered: true,
            source_active: true,
            diagnostics_fresh: Some(false),
            replacement_allowed: true,
            ..Situation::default()
        };
        assert_eq!(
            gnome_ibus_readiness(&stale),
            GnomeIbusReadiness::DiagnosticsStale
        );
        assert_eq!(
            next_step(&stale),
            Some((Problem::InputStatusStale, Action::CheckAgain))
        );
    }

    #[test]
    fn gnome_engine_restart_requires_fresh_diagnostics_before_ready() {
        let mut state = Situation {
            gnome_ibus: true,
            source_registered: true,
            source_active: true,
            diagnostics_fresh: Some(true),
            replacement_allowed: true,
            ..Situation::default()
        };
        // The process poll detects the crash even if the selected source is
        // still configured.
        assert_eq!(
            next_step(&state),
            Some((Problem::IbusEngineUnavailable, Action::RunSetup))
        );
        // A new process has appeared, but the previous source diagnostics
        // belong to the old process and need to be refreshed first.
        state.ibus_active = true;
        state.diagnostics_fresh = Some(false);
        assert_eq!(
            next_step(&state),
            Some((Problem::InputStatusStale, Action::CheckAgain))
        );
        state.diagnostics_fresh = Some(true);
        assert_eq!(gnome_ibus_readiness(&state), GnomeIbusReadiness::Ready);
        assert_eq!(next_step(&state), None);
    }

    #[test]
    fn an_app_that_cannot_replace_text_points_to_the_quick_picker() {
        let kde_ibus = Situation {
            ibus_active: true,
            ibus_selection: IbusCheckState::Pass,
            daemon_reachable: Some(false),
            replacement_allowed: true,
            field_cannot_replace: true,
            ..Situation::default()
        };
        assert_eq!(
            next_step(&kde_ibus),
            Some((Problem::FieldCannotReplace, Action::QuickPicker))
        );
        // The route itself failing outranks one app's field.
        assert_eq!(
            next_step(&Situation {
                field_cannot_replace: true,
                ..running(RouteState::Failed)
            }),
            Some((Problem::RouteFailed, Action::RunSetup))
        );
    }

    #[test]
    fn kde_live_engine_is_not_ready_when_another_ibus_engine_is_selected() {
        let deselected = Situation {
            ibus_active: true,
            ibus_selection: IbusCheckState::Fail,
            daemon_reachable: Some(false),
            replacement_allowed: true,
            ..Situation::default()
        };
        assert_eq!(
            next_step(&deselected),
            Some((Problem::InputSourceNotSelected, Action::RunSetup))
        );

        let unknown = Situation {
            ibus_selection: IbusCheckState::Unknown,
            ..deselected
        };
        assert_eq!(
            next_step(&unknown),
            Some((Problem::InputStatusPending, Action::CheckAgain))
        );

        let selected = Situation {
            ibus_selection: IbusCheckState::Pass,
            ..deselected
        };
        assert_eq!(next_step(&selected), None);
    }
}
