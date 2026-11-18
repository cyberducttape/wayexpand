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
    InstallationIncomplete,
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
    GnomeSourceStatusUnavailable,
    IbusManifestScopeInvalid,
    IbusDesktopRouteUnavailable,
    IbusDesktopRouteRecentlyCrashed,
    IbusEngineNotRegistered,
    InputSourceNotSelected,
    InputStatusPending,
    IbusEngineUnavailable,
    IbusInputNotObserved,
    InputStatusStale,
    ReplacementNotAllowed,
    ReplacementConsentUnknown,
    Connecting,
    RouteFailed,
    Unsupported,
    FieldCannotReplace,
    FieldTextMismatch,
    FieldPolicyRefusal,
    SensitiveField,
    IbusFieldTypeUnknown,
    TriggerNotMatched,
    TriggerMatchUnknown,
    NoEnabledExpansions,
    ApplicationDeliveryUnverified,
    TerminalRouteNoInput,
    OutputUnverified,
    PartialOutput,
    OutputNotApplied,
    OutputBlockedByPolicy,
    OutputInterrupted,
    SnippetActionFailed,
    OutputStatusUnknown,
    TerminalFocusUnknown,
    TerminalCompositorIdentity,
    StatusUnknown,
    RouteStatusUnknown,
}

/// The one thing to do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    CheckInstallation,
    TurnOn,
    Resume,
    RunSetup,
    CheckUpgrade,
    SelectInputSource,
    ReviewConsent,
    CheckAgain,
    QuickPicker,
    OpenMatcherPreview,
    TestInApplication,
    InspectText,
    ReviewSnippet,
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
    SourceStatusUnavailable,
    ManifestScopeInvalid,
    EngineNotRegistered,
    SourceNotSelected,
    EngineUnavailable,
    ReplacementNotAllowed,
    ReplacementConsentUnknown,
}

/// The facts `next_step` decides from, gathered from the GUI's last status
/// and diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Situation {
    /// Installed-component readiness from the versioned startup audit. Missing
    /// report data stays unknown rather than appearing healthy.
    pub installation_ready: Option<bool>,
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
    /// IBus is the selected or explicitly recommended desktop route, even if
    /// its engine has not started yet. This prevents KDE IBus failures from
    /// falling through to the unrelated generic daemon-off diagnosis.
    pub ibus_route_expected: bool,
    /// Preserve PASS/FAIL/UNKNOWN from the IBus chain. Missing evidence must
    /// never be presented as a confirmed user configuration error.
    pub source_registered: IbusCheckState,
    /// The IBus component registry can be healthy independently of GNOME's
    /// selectable input-source list.
    pub ibus_engine_registered: IbusCheckState,
    /// The shared component does not execute a user-owned home-directory
    /// binary. This is separate from whether the engine happens to run now.
    pub ibus_manifest_scope: IbusCheckState,
    /// KWin's IBus Wayland helper has a distinct route-health check; a recent
    /// helper crash can explain missing input even when the engine is live.
    pub ibus_desktop_state: IbusCheckState,
    pub ibus_desktop_problem_id: Option<&'static str>,
    pub source_active: IbusCheckState,
    pub engine_state: IbusCheckState,
    pub consent_state: IbusCheckState,
    /// None means no diagnostics have arrived yet; false means the report is
    /// old or belongs to a different IBus engine process.
    pub diagnostics_fresh: Option<bool>,
    pub paused: bool,
    pub route_state: Option<runtime::RouteState>,
    pub build: Option<runtime::DaemonBuildState>,
    /// Replacement is admitted: atomic and verified, or explicitly allowed.
    pub replacement_allowed: bool,
    /// Fresh, content-free verdict for the currently focused IBus field that
    /// received input. The text, trigger, and replacement are never retained.
    pub ibus_field_verdict: Option<wayexpand_backend_ibus::IbusFieldVerdict>,
    /// A user-confirmed insertion in an application still covers this
    /// desktop session, build, and route. Matcher previews and backend
    /// acknowledgements do not satisfy this condition.
    pub application_delivery_verified: bool,
    /// The raw-capture route is limited to configured terminal applications.
    pub terminal_apps_only: bool,
    /// The daemon reports application_scope_unknown: the restricted route is
    /// failing closed because it has no focused application identity.
    pub terminal_focus_unknown: bool,
    /// KWin returned its own process ID instead of the focused client ID.
    pub terminal_compositor_identity: bool,
    /// Events accepted by this daemon since startup, when the status schema
    /// exposes the counter. Zero is useful evidence only for that interval.
    pub input_events_total: Option<u64>,
    /// Count reported by the current daemon build. Unknown or stale reports
    /// must not be used to blame the user's snippet library.
    pub enabled_expansions: Option<usize>,
    /// The latest content-free output outcome from the current daemon. This
    /// is deliberately separate from route-level application verification.
    pub recent_outcome: Option<wayexpand_core::OutcomeKind>,
    /// Latest focused-field matcher state for the desktop-managed IBus route.
    /// This is absent unless diagnostics are fresh and the field received
    /// typing in the current engine process.
    pub ibus_latest_match: Option<wayexpand_backend_ibus::telemetry::IbusLatestMatch>,
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
    match s.source_registered {
        IbusCheckState::Fail => {
            return GnomeIbusReadiness::SourceNotSetUp;
        }
        IbusCheckState::Warn | IbusCheckState::Unknown | IbusCheckState::Skip => {
            return GnomeIbusReadiness::DiagnosticsPending;
        }
        IbusCheckState::Pass => {}
    }
    match s.ibus_engine_registered {
        IbusCheckState::Fail => {
            return GnomeIbusReadiness::EngineNotRegistered;
        }
        IbusCheckState::Warn | IbusCheckState::Unknown | IbusCheckState::Skip => {
            return GnomeIbusReadiness::DiagnosticsPending;
        }
        IbusCheckState::Pass => {}
    }
    match s.ibus_manifest_scope {
        IbusCheckState::Fail => return GnomeIbusReadiness::ManifestScopeInvalid,
        IbusCheckState::Warn | IbusCheckState::Unknown | IbusCheckState::Skip => {
            return GnomeIbusReadiness::DiagnosticsPending;
        }
        IbusCheckState::Pass => {}
    }
    match s.source_active {
        IbusCheckState::Fail | IbusCheckState::Warn => {
            return GnomeIbusReadiness::SourceNotSelected;
        }
        IbusCheckState::Unknown => {
            return GnomeIbusReadiness::SourceStatusUnavailable;
        }
        IbusCheckState::Skip => {
            return GnomeIbusReadiness::DiagnosticsPending;
        }
        IbusCheckState::Pass => {}
    }
    match s.engine_state {
        IbusCheckState::Fail | IbusCheckState::Warn => {
            return GnomeIbusReadiness::EngineUnavailable;
        }
        IbusCheckState::Unknown | IbusCheckState::Skip => {
            return GnomeIbusReadiness::DiagnosticsPending;
        }
        IbusCheckState::Pass if !s.ibus_active => {
            return GnomeIbusReadiness::DiagnosticsPending;
        }
        IbusCheckState::Pass => {}
    }
    // The engine must be alive before replacement consent can matter. Keep
    // this ordering aligned with `wayexpand why` so simultaneous failures
    // point to the first actionable stage in the input path.
    match s.consent_state {
        IbusCheckState::Fail => {
            return GnomeIbusReadiness::ReplacementNotAllowed;
        }
        IbusCheckState::Warn | IbusCheckState::Unknown | IbusCheckState::Skip => {
            return GnomeIbusReadiness::ReplacementConsentUnknown;
        }
        IbusCheckState::Pass => {}
    }
    GnomeIbusReadiness::Ready
}

/// Capture and engine setup are complete even when the user has not granted
/// non-atomic replacement consent. Keep that policy gate separate so first
/// run can say "configured, awaiting consent/test" instead of "setup failed".
pub(crate) fn gnome_ibus_route_configured(s: &Situation) -> bool {
    matches!(
        gnome_ibus_readiness(s),
        GnomeIbusReadiness::Ready | GnomeIbusReadiness::ReplacementNotAllowed
    )
}

/// Whether the configured evdev route is limited to terminal applications.
/// Keep this classification shared between the route adviser and onboarding.
pub(super) fn terminal_route_is_scoped(daemon_status: &str) -> bool {
    let terminal_only =
        runtime::status_field(daemon_status, "terminal_apps_only").as_deref() == Some("true");
    let raw_fallback_enabled = runtime::status_field(daemon_status, "raw_capture_fallback_enabled")
        .as_deref()
        == Some("true");
    let raw_capture_app_count = runtime::status_field(daemon_status, "raw_capture_app_count")
        .and_then(|value| value.parse::<usize>().ok());
    terminal_only && !(raw_fallback_enabled && raw_capture_app_count.is_none_or(|count| count > 0))
}

/// The most important problem and its action, or `None` only when a real
/// application insertion has been confirmed for the current route.
pub(crate) fn next_step(s: &Situation) -> Option<(Problem, Action)> {
    use runtime::RouteState as R;
    use wayexpand_core::OutcomeKind as O;
    if s.installation_ready == Some(false) {
        return Some((Problem::InstallationIncomplete, Action::CheckInstallation));
    }
    let field = || {
        // A transaction result is more specific than the most recent field's
        // general capability state, and must not be hidden by it.
        if let Some(outcome) = s.recent_outcome {
            match outcome {
                O::Partial => Some((Problem::PartialOutput, Action::InspectText)),
                O::Unknown | O::Expanded | O::ExpandedCursorMisplaced => {
                    Some((Problem::OutputUnverified, Action::InspectText))
                }
                O::NotApplied => Some((Problem::OutputNotApplied, Action::InspectText)),
                O::BlockedByPolicy => Some((Problem::OutputBlockedByPolicy, Action::InspectText)),
                O::InterruptedByTyping => Some((Problem::OutputInterrupted, Action::InspectText)),
                O::SnippetActionFailed => {
                    Some((Problem::SnippetActionFailed, Action::ReviewSnippet))
                }
                O::Other => Some((Problem::OutputStatusUnknown, Action::InspectText)),
            }
        } else if s.enabled_expansions == Some(0)
            && (s.input_events_total.is_some_and(|count| count > 0)
                || s.ibus_field_verdict.is_some()
                || s.ibus_latest_match.is_some())
        {
            Some((Problem::NoEnabledExpansions, Action::ReviewSnippet))
        } else if let Some(verdict) = s.ibus_field_verdict {
            use wayexpand_backend_ibus::IbusFieldVerdict as V;
            match verdict {
                V::Sensitive => Some((Problem::SensitiveField, Action::TestInApplication)),
                V::NoSurroundingText => Some((Problem::FieldCannotReplace, Action::QuickPicker)),
                V::Mismatch => Some((Problem::FieldTextMismatch, Action::OpenMatcherPreview)),
                V::Policy => Some((Problem::FieldPolicyRefusal, Action::InspectText)),
                V::NoContentType => {
                    Some((Problem::IbusFieldTypeUnknown, Action::TestInApplication))
                }
                V::OutputEmittedUnverified => {
                    Some((Problem::OutputUnverified, Action::InspectText))
                }
                V::Idle => Some((Problem::IbusInputNotObserved, Action::TestInApplication)),
                V::ReplacementContextObserved
                    if s.ibus_latest_match
                        == Some(wayexpand_backend_ibus::telemetry::IbusLatestMatch::NoMatch) =>
                {
                    Some((Problem::TriggerNotMatched, Action::OpenMatcherPreview))
                }
                V::ReplacementContextObserved if s.application_delivery_verified => {
                    Some((Problem::TriggerMatchUnknown, Action::OpenMatcherPreview))
                }
                V::ReplacementContextObserved => Some((
                    Problem::ApplicationDeliveryUnverified,
                    Action::TestInApplication,
                )),
            }
        } else if !s.application_delivery_verified
            && (s.gnome_ibus || s.ibus_route_expected)
            && s.ibus_latest_match.is_none()
        {
            Some((Problem::IbusInputNotObserved, Action::TestInApplication))
        } else if !s.application_delivery_verified
            && s.ibus_latest_match
                == Some(wayexpand_backend_ibus::telemetry::IbusLatestMatch::NoMatch)
        {
            Some((Problem::TriggerNotMatched, Action::OpenMatcherPreview))
        } else if !s.application_delivery_verified {
            Some((
                Problem::ApplicationDeliveryUnverified,
                Action::TestInApplication,
            ))
        } else {
            None
        }
    };
    // Route failures take priority over old output history. Once the current
    // route is ready, a fresh output outcome takes priority over general
    // route-level verification so a partial write is never hidden by an old
    // successful setup test.
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
            GnomeIbusReadiness::SourceStatusUnavailable => {
                Some((Problem::GnomeSourceStatusUnavailable, Action::InspectText))
            }
            GnomeIbusReadiness::EngineNotRegistered => {
                Some((Problem::IbusEngineNotRegistered, Action::RunSetup))
            }
            GnomeIbusReadiness::ManifestScopeInvalid => {
                Some((Problem::IbusManifestScopeInvalid, Action::RunSetup))
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
            GnomeIbusReadiness::ReplacementConsentUnknown => {
                Some((Problem::ReplacementConsentUnknown, Action::InspectText))
            }
        };
    }
    // The IBus route without a typing daemon (KDE): the engine is the route.
    if s.ibus_route_expected
        || s.ibus_active && (s.daemon_reachable != Some(true) || s.control_only)
    {
        if s.daemon_reachable == Some(true) && s.paused {
            return Some((Problem::Paused, Action::Resume));
        }
        match s.diagnostics_fresh {
            Some(false) => return Some((Problem::InputStatusStale, Action::CheckAgain)),
            None => return Some((Problem::InputStatusPending, Action::CheckAgain)),
            Some(true) => {}
        }
        match s.ibus_engine_registered {
            IbusCheckState::Fail => {
                return Some((Problem::IbusEngineNotRegistered, Action::RunSetup));
            }
            IbusCheckState::Warn | IbusCheckState::Unknown | IbusCheckState::Skip => {
                return Some((Problem::InputStatusPending, Action::CheckAgain));
            }
            IbusCheckState::Pass => {}
        }
        match s.ibus_desktop_problem_id {
            Some("WX-KWIN-IBUS-001") if s.ibus_desktop_state == IbusCheckState::Fail => {
                return Some((Problem::IbusDesktopRouteUnavailable, Action::RunSetup));
            }
            Some("WX-KWIN-IBUS-002")
                if s.ibus_desktop_state == IbusCheckState::Warn
                    && s.ibus_field_verdict.is_none() =>
            {
                return Some((
                    Problem::IbusDesktopRouteRecentlyCrashed,
                    Action::TestInApplication,
                ));
            }
            _ => {}
        }
        match s.ibus_manifest_scope {
            IbusCheckState::Fail => {
                return Some((Problem::IbusManifestScopeInvalid, Action::RunSetup));
            }
            IbusCheckState::Warn | IbusCheckState::Unknown | IbusCheckState::Skip => {
                return Some((Problem::InputStatusPending, Action::CheckAgain));
            }
            IbusCheckState::Pass => {}
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
        match s.engine_state {
            IbusCheckState::Fail | IbusCheckState::Warn => {
                return Some((Problem::IbusEngineUnavailable, Action::RunSetup));
            }
            IbusCheckState::Unknown | IbusCheckState::Skip => {
                return Some((Problem::InputStatusPending, Action::CheckAgain));
            }
            IbusCheckState::Pass if !s.ibus_active => {
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
        None => return Some((Problem::StatusUnknown, Action::CheckAgain)),
        Some(false) => return Some((Problem::ServiceOff, Action::TurnOn)),
        Some(true) => {}
    }
    // Version identity must be established before trusting daemon-provided
    // focus, match, pause, or output history. The IBus branches above use
    // their own process-correlated readiness evidence and remain independent
    // of the companion control daemon's build.
    match s.build {
        Some(runtime::DaemonBuildState::RestartRequired) => {
            return Some((Problem::RestartNeeded, Action::CheckUpgrade));
        }
        Some(runtime::DaemonBuildState::Unverified) => {
            return Some((Problem::BuildUnknown, Action::CheckUpgrade));
        }
        _ => {}
    }
    // The control-only service exists to sit beside IBus; once its status is
    // trusted, reaching here means the IBus engine is not live, so nothing
    // reads typing.
    if s.control_only {
        return Some((Problem::NoTypingRoute, Action::RunSetup));
    }
    // A known pause is actionable regardless of whether this build has a
    // route-state record. Do not let missing route telemetry hide the Resume
    // action from the user's diagnosis.
    if s.paused {
        return Some((Problem::Paused, Action::Resume));
    }
    // This route limitation is now read only after the daemon build has been
    // trusted above. It is actionable even when there are no input counters.
    if s.terminal_apps_only && s.terminal_focus_unknown && s.recent_outcome.is_none() {
        return Some((
            if s.terminal_compositor_identity {
                Problem::TerminalCompositorIdentity
            } else {
                Problem::TerminalFocusUnknown
            },
            Action::QuickPicker,
        ));
    }
    match s.route_state {
        None if terminal_route_has_no_input(s) => {
            Some((Problem::TerminalRouteNoInput, Action::TestInApplication))
        }
        None => Some((Problem::RouteStatusUnknown, Action::CheckAgain)),
        Some(R::Connected | R::Degraded) => {
            if !s.replacement_allowed {
                Some((Problem::ReplacementNotAllowed, Action::ReviewConsent))
            } else if terminal_route_has_no_input(s) {
                Some((Problem::TerminalRouteNoInput, Action::TestInApplication))
            } else {
                field()
            }
        }
        Some(R::PermissionRequired) => Some((Problem::PermissionNeeded, Action::RunSetup)),
        Some(R::PortalRevoked) => Some((Problem::PermissionRevoked, Action::RunSetup)),
        Some(R::Starting | R::Reconnecting) => Some((Problem::Connecting, Action::CheckAgain)),
        Some(R::Failed | R::Stopped) => Some((Problem::RouteFailed, Action::RunSetup)),
        Some(R::Unsupported) => Some((Problem::Unsupported, Action::QuickPicker)),
    }
}

fn terminal_route_has_no_input(s: &Situation) -> bool {
    s.terminal_apps_only && s.input_events_total == Some(0) && !s.application_delivery_verified
}

impl GuiApp {
    /// The daemon may keep a terminal-scoped evdev companion running while
    /// IBus owns typing in other applications. Report and test the effective
    /// capture route, not merely the presence of that companion.
    pub(crate) fn active_route_is_terminal_scoped(&self) -> bool {
        terminal_route_is_scoped(&self.daemon_status) && !self.ibus_route_active()
    }

    /// Offer an explicit alternative when terminal-scoped evdev cannot
    /// establish the focused client identity and setup recommends IBus. The
    /// guided setup begins with a read-only preflight; switching still
    /// requires the existing consent review.
    pub(crate) fn can_offer_recommended_ibus_setup(&self, problem: Problem) -> bool {
        if !matches!(
            problem,
            Problem::TerminalFocusUnknown | Problem::TerminalCompositorIdentity
        ) {
            return false;
        }
        let recommends_ibus = self
            .recommended_route
            .is_some_and(|route| route.setup_backend() == "ibus");
        let configured_backend = super::setup::current_guided_setup_backend(
            self.daemon_reachable,
            &self.daemon_status,
            self.route_plan.as_ref(),
        );
        recommends_ibus && configured_backend.is_some_and(|backend| backend != "ibus")
    }

    pub(super) fn ibus_check_state(&self, id: &str) -> IbusCheckState {
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
            && self.ibus_check_state("engine") == IbusCheckState::Pass
            && (wayexpand_backend_selection::Compositor::detect()
                != wayexpand_backend_selection::Compositor::Gnome
                || self.ibus_check_state("gnome-source-active") == IbusCheckState::Pass)
    }

    pub(crate) fn situation(&self) -> Situation {
        let daemon_running = self.daemon_reachable == Some(true);
        let daemon_status_trusted = daemon_running
            && runtime::daemon_build_state(&self.daemon_status)
                == runtime::DaemonBuildState::Current;
        let control_only =
            runtime::status_field(&self.daemon_status, "source").as_deref() == Some("stdin");
        let active_daemon_source = daemon_running
            .then(|| runtime::status_field(&self.daemon_status, "source"))
            .flatten();
        let compositor = wayexpand_backend_selection::Compositor::detect();
        let gnome_session = compositor == wayexpand_backend_selection::Compositor::Gnome
            || self.ibus_route.as_ref().is_some_and(|report| {
                report.desktop == wayexpand_backend_ibus::chain::IbusDesktop::Gnome
            });
        let terminal_only_daemon = daemon_running && terminal_route_is_scoped(&self.daemon_status);
        let gnome_ibus = super::first_run::uses_gnome_ibus_route_with_scope(
            gnome_session,
            self.ibus_check_state("gnome-source-active") == IbusCheckState::Pass,
            self.ibus_check_state("gnome-source-registered") == IbusCheckState::Pass,
            self.recommended_route
                .is_some_and(|route| route.setup_backend() == "ibus"),
            self.daemon_reachable,
            active_daemon_source.as_deref(),
            terminal_only_daemon,
        );
        let no_active_typing_daemon = self.daemon_reachable == Some(false)
            || daemon_running && (active_daemon_source.as_deref() == Some("stdin") || control_only);
        let kde_ibus_expected = compositor == wayexpand_backend_selection::Compositor::KdePlasma
            // A terminal-restricted daemon does not cover browser/editor
            // fields. Keep the KDE IBus route in the readiness model beside
            // it instead of letting the running terminal daemon mask an
            // IBus registration or selection failure.
            && (no_active_typing_daemon || terminal_only_daemon)
            && (self.ibus_active
                || self.ibus_route.as_ref().is_some_and(|report| {
                    report.desktop == wayexpand_backend_ibus::chain::IbusDesktop::KdeWayland
                })
                || self
                    .recommended_route
                    .is_some_and(|route| route.setup_backend() == "ibus"));
        let ibus_route_expected = gnome_ibus || kde_ibus_expected;
        // Keep process health separate from source selection: GNOME can have
        // the source selected while its engine process has crashed.
        let ibus_active = self.ibus_active;
        let ibus_route = ibus_route_expected || ibus_active && (!daemon_running || control_only);
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
        let matching_block =
            runtime::status_field(&self.daemon_status, "matching_unavailable_reason")
                .as_deref()
                .and_then(wayexpand_core::MatchingBlock::from_reason);
        // The chain-level `surrounding-text` check summarizes several recent
        // IBus fields. It cannot tell us that the currently focused application
        // lacks surrounding text. Use only the focused field that received
        // keys, and only while its engine diagnostics still belong to the live
        // process. A formerly focused field is historical evidence, not the
        // current field's diagnosis.
        let latest_ibus_field_verdict = (self.ibus_diagnostics_fresh() == Some(true))
            .then_some(self.ibus_route.as_ref())
            .flatten()
            .and_then(|report| report.activity.as_ref())
            .filter(|activity| activity.live)
            .and_then(|activity| {
                activity
                    .status
                    .recent_fields
                    .iter()
                    .rev()
                    .find(|field| field.focused && field.keys > 0)
            })
            .map(wayexpand_backend_ibus::IbusFieldVerdict::of);
        let ibus_latest_match = (ibus_route && self.ibus_diagnostics_fresh() == Some(true))
            .then_some(self.ibus_route.as_ref())
            .flatten()
            .and_then(|report| report.activity.as_ref())
            .filter(|activity| activity.live)
            .and_then(|activity| {
                activity
                    .status
                    .recent_fields
                    .iter()
                    .rev()
                    .find(|field| field.focused && field.keys > 0)
            })
            .map(|field| field.latest_match);
        // The control daemon may retain a recent terminal expansion while
        // IBus is the route being diagnosed for a browser/editor field. That
        // transaction cannot explain the IBus field, so don't let it outrank
        // current field evidence in `next_step`.
        let daemon_outcome_applies = !(ibus_route && (control_only || terminal_only_daemon));
        let recent_outcome = daemon_outcome_applies
            .then(|| self.recent_output_outcome())
            .flatten();
        Situation {
            installation_ready: self.installation_audit.ready,
            daemon_reachable: self.daemon_reachable,
            ibus_active,
            ibus_selection: if self.ibus_diagnostics_fresh() == Some(true) {
                self.ibus_check_state("selected")
            } else {
                IbusCheckState::Unknown
            },
            control_only,
            gnome_ibus,
            ibus_route_expected,
            source_registered: self.ibus_check_state("gnome-source-registered"),
            ibus_engine_registered: if self.ibus_diagnostics_fresh() == Some(true) {
                self.ibus_check_state("registered")
            } else {
                IbusCheckState::Unknown
            },
            ibus_manifest_scope: if self.ibus_diagnostics_fresh() == Some(true) {
                self.ibus_check_state("manifest-scope")
            } else {
                IbusCheckState::Unknown
            },
            ibus_desktop_state: if self.ibus_diagnostics_fresh() == Some(true) {
                self.ibus_check_state("desktop")
            } else {
                IbusCheckState::Unknown
            },
            ibus_desktop_problem_id: if self.ibus_diagnostics_fresh() == Some(true) {
                self.ibus_route
                    .as_ref()
                    .and_then(|report| report.check("desktop"))
                    .and_then(|check| check.problem_id)
            } else {
                None
            },
            source_active: self.ibus_check_state("gnome-source-active"),
            engine_state: self.ibus_check_state("engine"),
            consent_state: self.ibus_check_state("consent"),
            diagnostics_fresh: self.ibus_diagnostics_fresh(),
            paused: self.paused || matching_block == Some(wayexpand_core::MatchingBlock::Paused),
            route_state: self.route_state,
            // The helper returns Unverified for an incompatible or ambiguous
            // status schema, even if its build strings happen to match.
            build: daemon_running.then(|| runtime::daemon_build_state(&self.daemon_status)),
            replacement_allowed,
            ibus_field_verdict: ibus_route.then_some(latest_ibus_field_verdict).flatten(),
            application_delivery_verified: self.expansion_observed(),
            terminal_apps_only: terminal_route_is_scoped(&self.daemon_status),
            terminal_focus_unknown: runtime::status_field(
                &self.daemon_status,
                "matching_unavailable_reason",
            )
            .as_deref()
                == Some("application_scope_unknown"),
            terminal_compositor_identity: runtime::status_field(
                &self.daemon_status,
                "window_process_identity_state",
            )
            .as_deref()
                == Some("compositor-process-id"),
            input_events_total: runtime::status_field(&self.daemon_status, "input_events_total")
                .and_then(|value| value.parse::<u64>().ok()),
            enabled_expansions: daemon_status_trusted
                .then(|| runtime::status_field(&self.daemon_status, "config_enabled_expansions"))
                .flatten()
                .and_then(|value| value.parse::<usize>().ok()),
            recent_outcome,
            ibus_latest_match,
        }
    }

    fn recent_output_outcome(&self) -> Option<wayexpand_core::OutcomeKind> {
        if self.daemon_reachable != Some(true) {
            return None;
        }
        let status = wayexpand_core::DaemonStatus::parse(&self.daemon_status);
        if status.status_schema() != Some(wayexpand_core::CONTROL_STATUS_SCHEMA) {
            return None;
        }
        let diagnosis = wayexpand_core::ExpansionDiagnosis::from_status(&status);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        diagnosis
            .recent
            .first()
            .filter(|outcome| {
                now.saturating_sub(outcome.at_unix) <= wayexpand_core::RECENT_OUTCOME_SECS
            })
            .map(|outcome| outcome.kind)
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

    /// Run the concrete repair or verification associated with the finding.
    pub(crate) fn perform_next_step(&mut self, action: Action) {
        match action {
            Action::CheckInstallation => self.check_upgrade_runtime(),
            Action::TurnOn | Action::RunSetup => self.start_guided_setup(),
            Action::ReviewConsent => self.review_replacement_consent(),
            Action::Resume => self.toggle_pause(),
            Action::CheckUpgrade => self.check_upgrade_runtime(),
            Action::SelectInputSource => self.retry_gnome_source_activation(),
            Action::CheckAgain => self.refresh_diagnostics(true),
            Action::QuickPicker => self.open_copy_only_picker(),
            Action::OpenMatcherPreview => {
                if self.onboarding_visible {
                    self.open_onboarding_matcher_preview();
                } else {
                    self.diagnostics_open = false;
                    self.try_live_open = true;
                }
            }
            Action::TestInApplication => self.start_desktop_integration_test(),
            Action::InspectText => {
                self.diagnostics_open = true;
                self.refresh_diagnostics(true);
            }
            Action::ReviewSnippet => {
                self.diagnostics_open = false;
            }
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
        let ibus_route = situation.gnome_ibus
            || situation.ibus_route_expected
            || configured_backend.is_none()
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
                    ui.label(
                        self.strings
                            .next_step_instruction(problem, self.situation().gnome_ibus),
                    );
                    if ui
                        .add_enabled(!busy, egui::Button::new(self.strings.action_label(action)))
                        .clicked()
                    {
                        self.perform_next_step(action);
                    }
                    // Keep the diagnosed repair as the primary action, but
                    // make the safe fallback available without sending the
                    // user to a separate diagnostics screen. The picker uses
                    // copy-only delivery here; it does not claim that the
                    // target application received or applied the text.
                    if action != Action::QuickPicker
                        && (problem != Problem::NoEnabledExpansions
                            || self.config.expansion.iter().any(|entry| entry.enabled))
                        && ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(self.strings.action_label(Action::QuickPicker)),
                            )
                            .clicked()
                    {
                        self.perform_next_step(Action::QuickPicker);
                    }
                    // A matcher diagnosis can identify a rule problem, but
                    // it cannot establish that this application's field
                    // receives or applies output. Keep the real-app check
                    // available beside matcher review so users can verify
                    // the separate delivery stage without treating preview
                    // as proof.
                    if matches!(
                        problem,
                        Problem::TriggerNotMatched
                            | Problem::TriggerMatchUnknown
                            | Problem::FieldTextMismatch
                    ) && ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(self.strings.action_label(Action::TestInApplication)),
                        )
                        .clicked()
                    {
                        self.perform_next_step(Action::TestInApplication);
                    }
                    if self.can_offer_recommended_ibus_setup(problem)
                        && ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(
                                    self.strings.onboarding_set_up_recommended_ibus(),
                                ),
                            )
                            .clicked()
                    {
                        self.start_guided_setup();
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
            application_delivery_verified: true,
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
                    ibus_engine_registered: IbusCheckState::Pass,
                    ibus_manifest_scope: IbusCheckState::Pass,
                    ibus_selection: IbusCheckState::Pass,
                    engine_state: IbusCheckState::Pass,
                    diagnostics_fresh: Some(true),
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
                Some((Problem::RestartNeeded, Action::CheckUpgrade)),
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
        assert_eq!(
            next_step(&Situation::default()),
            Some((Problem::StatusUnknown, Action::CheckAgain))
        );
        assert_eq!(
            next_step(&Situation {
                gnome_ibus: true,
                ..Situation::default()
            }),
            Some((Problem::InputStatusPending, Action::CheckAgain))
        );
    }

    #[test]
    fn gnome_onboarding_state_transitions_keep_actions_aligned_with_evidence() {
        // Model the states a user can observe while setup progresses or the
        // desktop changes underneath the GUI. Each refreshed snapshot must
        // produce both the same readiness classification and the matching
        // single next action.
        let mut state = Situation {
            gnome_ibus: true,
            ibus_route_expected: true,
            daemon_reachable: Some(true),
            control_only: true,
            diagnostics_fresh: None,
            ..Situation::default()
        };
        assert_eq!(
            gnome_ibus_readiness(&state),
            GnomeIbusReadiness::DiagnosticsPending
        );
        assert_eq!(
            next_step(&state),
            Some((Problem::InputStatusPending, Action::CheckAgain))
        );

        // IBus registered the engine after the GUI's first refresh, but the
        // input source is not yet registered with GNOME.
        state.diagnostics_fresh = Some(true);
        state.source_registered = IbusCheckState::Fail;
        assert_eq!(
            gnome_ibus_readiness(&state),
            GnomeIbusReadiness::SourceNotSetUp
        );
        assert_eq!(
            next_step(&state),
            Some((Problem::InputSourceNotSetUp, Action::RunSetup))
        );

        // The next diagnostics refresh observes registration and selection,
        // but the engine process has not started yet.
        state.source_registered = IbusCheckState::Pass;
        state.ibus_engine_registered = IbusCheckState::Pass;
        state.ibus_manifest_scope = IbusCheckState::Pass;
        state.source_active = IbusCheckState::Pass;
        state.engine_state = IbusCheckState::Fail;
        assert_eq!(
            gnome_ibus_readiness(&state),
            GnomeIbusReadiness::EngineUnavailable
        );
        assert_eq!(
            next_step(&state),
            Some((Problem::IbusEngineUnavailable, Action::RunSetup))
        );

        // Once IBus starts and consent is present, configuration is ready,
        // while actual application delivery still needs its own test.
        state.engine_state = IbusCheckState::Pass;
        state.ibus_active = true;
        state.consent_state = IbusCheckState::Pass;
        assert_eq!(gnome_ibus_readiness(&state), GnomeIbusReadiness::Ready);
        assert_eq!(
            next_step(&state),
            Some((Problem::IbusInputNotObserved, Action::TestInApplication))
        );

        // A service/engine restart invalidates the previous process-correlated
        // report; old registration and insertion evidence cannot mask it.
        state.diagnostics_fresh = Some(false);
        state.ibus_active = false;
        assert_eq!(
            gnome_ibus_readiness(&state),
            GnomeIbusReadiness::DiagnosticsStale
        );
        assert_eq!(
            next_step(&state),
            Some((Problem::InputStatusStale, Action::CheckAgain))
        );

        // A fresh report after permission revocation returns to the explicit
        // consent gate, even if the previous application test had succeeded.
        state.diagnostics_fresh = Some(true);
        state.ibus_active = true;
        state.consent_state = IbusCheckState::Fail;
        state.application_delivery_verified = true;
        assert_eq!(
            gnome_ibus_readiness(&state),
            GnomeIbusReadiness::ReplacementNotAllowed
        );
        assert_eq!(
            next_step(&state),
            Some((Problem::ReplacementNotAllowed, Action::ReviewConsent))
        );
    }

    #[test]
    fn stale_terminal_scope_status_cannot_override_the_build_diagnosis() {
        let stale = Situation {
            daemon_reachable: Some(true),
            terminal_apps_only: true,
            terminal_focus_unknown: true,
            build: Some(DaemonBuildState::RestartRequired),
            ..Situation::default()
        };
        assert_eq!(
            next_step(&stale),
            Some((Problem::RestartNeeded, Action::CheckUpgrade))
        );

        let stale_control = Situation {
            control_only: true,
            ..stale
        };
        assert_eq!(
            next_step(&stale_control),
            Some((Problem::RestartNeeded, Action::CheckUpgrade))
        );

        let unknown = Situation {
            build: Some(DaemonBuildState::Unverified),
            ..stale
        };
        assert_eq!(
            next_step(&unknown),
            Some((Problem::BuildUnknown, Action::CheckUpgrade))
        );

        let current = Situation {
            build: Some(DaemonBuildState::Current),
            ..stale
        };
        assert_eq!(
            next_step(&current),
            Some((Problem::TerminalFocusUnknown, Action::QuickPicker))
        );
    }

    #[test]
    fn gnome_walks_through_its_input_source_in_order() {
        let gnome = Situation {
            gnome_ibus: true,
            ibus_active: true,
            source_registered: IbusCheckState::Fail,
            ibus_engine_registered: IbusCheckState::Pass,
            ibus_manifest_scope: IbusCheckState::Pass,
            source_active: IbusCheckState::Fail,
            engine_state: IbusCheckState::Pass,
            consent_state: IbusCheckState::Pass,
            replacement_allowed: true,
            diagnostics_fresh: Some(true),
            application_delivery_verified: true,
            ..Situation::default()
        };
        assert_eq!(
            next_step(&gnome),
            Some((Problem::InputSourceNotSetUp, Action::RunSetup))
        );
        let registered = Situation {
            source_registered: IbusCheckState::Pass,
            ..gnome
        };
        assert_eq!(
            next_step(&registered),
            Some((Problem::InputSourceNotSelected, Action::SelectInputSource))
        );
        let selected = Situation {
            source_active: IbusCheckState::Pass,
            ..registered
        };
        assert_eq!(next_step(&selected), None);
        assert_eq!(
            next_step(&Situation {
                ibus_field_verdict: Some(
                    wayexpand_backend_ibus::IbusFieldVerdict::NoSurroundingText
                ),
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
            source_registered: IbusCheckState::Pass,
            ibus_engine_registered: IbusCheckState::Pass,
            ibus_manifest_scope: IbusCheckState::Pass,
            source_active: IbusCheckState::Pass,
            engine_state: IbusCheckState::Fail,
            consent_state: IbusCheckState::Pass,
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
            source_registered: IbusCheckState::Pass,
            ibus_engine_registered: IbusCheckState::Pass,
            ibus_manifest_scope: IbusCheckState::Pass,
            source_active: IbusCheckState::Pass,
            engine_state: IbusCheckState::Pass,
            consent_state: IbusCheckState::Unknown,
            diagnostics_fresh: Some(true),
            replacement_allowed: false,
            ..Situation::default()
        };
        assert_eq!(
            gnome_ibus_readiness(&unknown_consent),
            GnomeIbusReadiness::ReplacementConsentUnknown
        );
        assert_eq!(
            next_step(&unknown_consent),
            Some((Problem::ReplacementConsentUnknown, Action::InspectText))
        );
    }

    #[test]
    fn gnome_source_deselection_has_a_corrective_action() {
        let deselected = Situation {
            gnome_ibus: true,
            ibus_active: true,
            source_registered: IbusCheckState::Pass,
            ibus_engine_registered: IbusCheckState::Pass,
            ibus_manifest_scope: IbusCheckState::Pass,
            source_active: IbusCheckState::Warn,
            engine_state: IbusCheckState::Pass,
            consent_state: IbusCheckState::Pass,
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
            source_registered: IbusCheckState::Pass,
            ibus_engine_registered: IbusCheckState::Pass,
            ibus_manifest_scope: IbusCheckState::Pass,
            source_active: IbusCheckState::Pass,
            engine_state: IbusCheckState::Pass,
            consent_state: IbusCheckState::Pass,
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
            source_registered: IbusCheckState::Pass,
            ibus_engine_registered: IbusCheckState::Pass,
            ibus_manifest_scope: IbusCheckState::Pass,
            source_active: IbusCheckState::Pass,
            engine_state: IbusCheckState::Fail,
            consent_state: IbusCheckState::Pass,
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
        state.engine_state = IbusCheckState::Pass;
        assert_eq!(gnome_ibus_readiness(&state), GnomeIbusReadiness::Ready);
        state.application_delivery_verified = true;
        assert_eq!(next_step(&state), None);
    }

    #[test]
    fn an_app_that_cannot_replace_text_points_to_the_quick_picker() {
        let kde_ibus = Situation {
            ibus_active: true,
            ibus_engine_registered: IbusCheckState::Pass,
            ibus_manifest_scope: IbusCheckState::Pass,
            ibus_selection: IbusCheckState::Pass,
            engine_state: IbusCheckState::Pass,
            daemon_reachable: Some(false),
            replacement_allowed: true,
            diagnostics_fresh: Some(true),
            ibus_field_verdict: Some(wayexpand_backend_ibus::IbusFieldVerdict::NoSurroundingText),
            application_delivery_verified: true,
            ..Situation::default()
        };
        assert_eq!(
            next_step(&kde_ibus),
            Some((Problem::FieldCannotReplace, Action::QuickPicker))
        );
        // The route itself failing outranks one app's field.
        assert_eq!(
            next_step(&Situation {
                ibus_field_verdict: Some(
                    wayexpand_backend_ibus::IbusFieldVerdict::NoSurroundingText
                ),
                ..running(RouteState::Failed)
            }),
            Some((Problem::RouteFailed, Action::RunSetup))
        );
    }

    #[test]
    fn kde_live_engine_is_not_ready_when_another_ibus_engine_is_selected() {
        let deselected = Situation {
            ibus_active: true,
            ibus_engine_registered: IbusCheckState::Pass,
            ibus_manifest_scope: IbusCheckState::Pass,
            ibus_selection: IbusCheckState::Fail,
            engine_state: IbusCheckState::Pass,
            daemon_reachable: Some(false),
            replacement_allowed: true,
            diagnostics_fresh: Some(true),
            application_delivery_verified: true,
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
