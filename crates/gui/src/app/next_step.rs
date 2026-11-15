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
    Paused,
    PermissionNeeded,
    PermissionRevoked,
    RestartNeeded,
    BuildUnknown,
    InputSourceNotSetUp,
    InputSourceNotSelected,
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

/// The facts `next_step` decides from, gathered from the GUI's last status
/// and diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Situation {
    pub daemon_reachable: Option<bool>,
    /// The WayExpand IBus engine is live.
    pub ibus_active: bool,
    /// The daemon only serves the control socket (the IBus route captures).
    pub control_only: bool,
    /// GNOME, judged by its IBus input source.
    pub gnome_ibus: bool,
    pub source_registered: bool,
    pub source_active: bool,
    pub paused: bool,
    pub route_state: Option<runtime::RouteState>,
    pub build: Option<runtime::DaemonBuildState>,
    /// Replacement is admitted: atomic and verified, or explicitly allowed.
    pub replacement_allowed: bool,
    /// The last normal field refused replacement (no surrounding text).
    pub field_cannot_replace: bool,
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
        if s.daemon_reachable.is_none() && !s.ibus_active {
            return None;
        }
        if !s.source_registered {
            return Some((Problem::InputSourceNotSetUp, Action::RunSetup));
        }
        if !s.source_active {
            return Some((Problem::InputSourceNotSelected, Action::SelectInputSource));
        }
        if !s.replacement_allowed {
            return Some((Problem::ReplacementNotAllowed, Action::ReviewConsent));
        }
        return field();
    }
    // The IBus route without a typing daemon (KDE): the engine is the route.
    if s.ibus_active && (s.daemon_reachable != Some(true) || s.control_only) {
        if s.daemon_reachable == Some(true) && s.paused {
            return Some((Problem::Paused, Action::Resume));
        }
        if !s.replacement_allowed {
            return Some((Problem::ReplacementNotAllowed, Action::ReviewConsent));
        }
        return field();
    }
    match s.daemon_reachable {
        None => return None,
        Some(false) => return Some((Problem::ServiceOff, Action::TurnOn)),
        Some(true) if s.control_only => return None,
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
        let gnome_ibus = super::first_run::uses_gnome_ibus_route(
            wayexpand_backend_selection::Compositor::detect()
                == wayexpand_backend_selection::Compositor::Gnome,
            self.ibus_active,
            self.recommended_route
                .is_some_and(|route| route.setup_backend() == "ibus"),
        );
        let ibus_route = gnome_ibus || self.ibus_active && (!daemon_running || control_only);
        let replacement_allowed = if ibus_route {
            // Unknown until diagnostics run; only an explicit refusal counts.
            self.ibus_check_state("consent") != IbusCheckState::Fail
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
            ibus_active: self.ibus_active,
            control_only,
            gnome_ibus,
            source_registered: self.ibus_check_state("gnome-source-registered")
                == IbusCheckState::Pass,
            source_active: self.ibus_check_state("gnome-source-active") == IbusCheckState::Pass,
            paused: self.paused,
            route_state: self.route_state,
            build: daemon_running.then(|| runtime::daemon_build_state(&self.daemon_status)),
            replacement_allowed,
            field_cannot_replace: ibus_route
                && self.ibus_check_state("surrounding-text") == IbusCheckState::Fail,
        }
    }

    pub(crate) fn current_next_step(&self) -> Option<(Problem, Action)> {
        next_step(&self.situation())
    }

    /// Do `action`. Each runs an existing, confirmed path: guided setup,
    /// the service-restart confirmation, the GNOME source repair, the
    /// copy-only picker.
    pub(crate) fn perform_next_step(&mut self, action: Action) {
        match action {
            Action::TurnOn | Action::RunSetup | Action::ReviewConsent => self.start_guided_setup(),
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
    use super::{next_step, Action, Problem, Situation};
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
    fn nothing_is_claimed_while_the_state_is_unknown() {
        assert_eq!(next_step(&Situation::default()), None);
        assert_eq!(
            next_step(&Situation {
                gnome_ibus: true,
                ..Situation::default()
            }),
            None
        );
    }

    #[test]
    fn gnome_walks_through_its_input_source_in_order() {
        let gnome = Situation {
            gnome_ibus: true,
            ibus_active: true,
            replacement_allowed: true,
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
    fn an_app_that_cannot_replace_text_points_to_the_quick_picker() {
        let kde_ibus = Situation {
            ibus_active: true,
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
}
