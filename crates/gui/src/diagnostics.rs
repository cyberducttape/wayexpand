use std::{
    env,
    io::{self, Read},
    os::fd::AsRawFd,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use wayexpand_backend_ibus::engine_available as ibus_engine_available;
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{load_organization_policy, BackendStatus};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssessmentState {
    Ready,
    Warning,
    Blocked,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RouteAssessment {
    pub capture: AssessmentState,
    pub insertion: AssessmentState,
    pub permissions: AssessmentState,
    pub focus: AssessmentState,
}

/// Summarize the active route from daemon-reported state and capabilities.
/// This never upgrades missing evidence to a green status.
pub(crate) fn assess_route(
    daemon_reachable: Option<bool>,
    route_state: Option<crate::runtime::RouteState>,
    capabilities: Option<&crate::runtime::DaemonCapabilities>,
    permission_health: &str,
) -> RouteAssessment {
    use crate::runtime::ReplacementAssurance;
    use AssessmentState::{Blocked, Ready, Unknown, Warning};

    let capture = match (
        daemon_reachable,
        route_state,
        capabilities.and_then(|caps| caps.capture_exclusive),
    ) {
        (Some(false), _, _) => Blocked,
        (_, Some(crate::runtime::RouteState::Connected), Some(true)) => Ready,
        (_, Some(crate::runtime::RouteState::Connected), Some(false)) => Warning,
        (_, Some(crate::runtime::RouteState::Connected), None) => Unknown,
        (
            _,
            Some(
                crate::runtime::RouteState::PermissionRequired
                | crate::runtime::RouteState::PortalRevoked
                | crate::runtime::RouteState::Unsupported
                | crate::runtime::RouteState::Failed
                | crate::runtime::RouteState::Stopped,
            ),
            _,
        ) => Blocked,
        (
            _,
            Some(
                crate::runtime::RouteState::Reconnecting
                | crate::runtime::RouteState::Starting
                | crate::runtime::RouteState::Degraded,
            ),
            _,
        ) => Warning,
        _ => Unknown,
    };

    let insertion = match capabilities.and_then(|caps| caps.replacement_assurance) {
        Some(ReplacementAssurance::Verified) => {
            match capabilities.and_then(|caps| caps.inject_full_unicode) {
                Some(true) => Ready,
                Some(false) => Warning,
                None => Unknown,
            }
        }
        Some(ReplacementAssurance::FocusTrackedBestEffort | ReplacementAssurance::Unverified) => {
            Warning
        }
        Some(ReplacementAssurance::Blocked | ReplacementAssurance::Unavailable) => Blocked,
        None if daemon_reachable == Some(false) => Blocked,
        None => Unknown,
    };

    let permissions = if permission_health.starts_with("permission required")
        || permission_health.starts_with("portal authorization revoked")
    {
        Blocked
    } else if permission_health.starts_with("active route connected")
        || permission_health.starts_with("granted")
    {
        Ready
    } else if permission_health.starts_with("OS permission not reported/applicable")
        || permission_health.starts_with("unknown")
    {
        Unknown
    } else {
        Warning
    };

    let focus = match capabilities {
        Some(caps)
            if caps.window_tracker_connected == Some(true)
                && caps.window_identity_exact == Some(true) =>
        {
            Ready
        }
        Some(caps)
            if caps.window_tracker_connected == Some(false)
                || caps.window_identity_exact == Some(false) =>
        {
            Warning
        }
        Some(_) | None => Unknown,
    };

    RouteAssessment {
        capture,
        insertion,
        permissions,
        focus,
    }
}

pub(crate) fn compositor_info() -> String {
    let (name, command, args) = if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        ("Hyprland", Some("Hyprland"), vec!["--version"])
    } else if env::var_os("SWAYSOCK").is_some() {
        ("Sway", Some("sway"), vec!["--version"])
    } else if env::var_os("WAYFIRE_SOCKET").is_some() {
        ("Wayfire", Some("wayfire"), vec!["--version"])
    } else if env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|value| value.to_ascii_lowercase().contains("kde"))
        || env::var_os("KDE_FULL_SESSION").is_some()
    {
        ("KWin", Some("kwin_wayland"), vec!["--version"])
    } else if env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|value| value.to_ascii_lowercase().contains("gnome"))
    {
        ("Mutter", None, Vec::new())
    } else if wayland_session() {
        ("Wayland compositor", None, Vec::new())
    } else {
        ("No Wayland compositor detected", None, Vec::new())
    };
    let version = command.and_then(|program| bounded_version(program, &args));
    match version {
        Some(version) => format!("{name} · {version}"),
        None => format!("{name} · version unavailable"),
    }
}

/// Version probes run on the diagnostics worker and have a hard deadline and
/// a tiny output cap. They are informational and never influence route choice.
fn bounded_version(program: &str, args: &[&str]) -> Option<String> {
    const OUTPUT_LIMIT: usize = 256;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_process_group(&mut command);
    let child = command.spawn().ok()?;
    let mut supervisor = ChildSupervisor::new(child);
    let mut stdout = supervisor.take_stdout()?;
    let flags = unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        supervisor.kill_group();
        let _ = supervisor.reap();
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(400);
    let mut output = Vec::with_capacity(OUTPUT_LIMIT);
    let mut buffer = [0u8; 128];
    let mut eof = false;
    loop {
        if !eof {
            for _ in 0..4 {
                match stdout.read(&mut buffer) {
                    Ok(0) => {
                        eof = true;
                        break;
                    }
                    Ok(count) => {
                        let retain = count.min(OUTPUT_LIMIT.saturating_sub(output.len()));
                        output.extend_from_slice(&buffer[..retain]);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        supervisor.kill_group();
                        let _ = supervisor.reap();
                        return None;
                    }
                }
            }
        }
        if supervisor.has_exited().ok()? {
            // A version probe must not leave descendants holding its pipe
            // open. Terminate the probe's process group before reaping it.
            supervisor.kill_group();
            let _ = supervisor.reap();
            let output = String::from_utf8_lossy(&output);
            let line = output.lines().next()?.trim();
            return (!line.is_empty()).then(|| line.to_owned());
        }
        if Instant::now() >= deadline {
            supervisor.kill_group();
            let _ = supervisor.reap();
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn permission_health(
    backends: &[BackendStatus],
    capabilities: Option<&crate::runtime::DaemonCapabilities>,
    daemon_status: &str,
) -> String {
    let status = wayexpand_core::DaemonStatus::parse(daemon_status);
    let source = status.field("source").unwrap_or_default();
    let output_backend = status.field("backend").unwrap_or_default();
    if source.is_empty() || output_backend.is_empty() {
        return "unknown · daemon route unavailable".into();
    }
    if capabilities.is_some() {
        match status.route_state() {
            Some(wayexpand_core::DaemonRouteState::Connected) => {
                return format!("active route connected · {source} + {output_backend}");
            }
            Some(wayexpand_core::DaemonRouteState::PermissionRequired) => {
                return format!("permission required · {source} + {output_backend}");
            }
            Some(wayexpand_core::DaemonRouteState::PortalRevoked) => {
                return format!("portal authorization revoked · {source} + {output_backend}");
            }
            _ => {}
        }
    }
    let mut checks = Vec::new();
    for route in [source, output_backend] {
        if let Some(backend) = backends
            .iter()
            .find(|backend| backend.kind.to_string() == route)
        {
            checks.push((route, backend.permission()));
        }
    }
    if checks
        .iter()
        .any(|(_, permission)| *permission == "Required")
    {
        return checks
            .iter()
            .filter(|(_, permission)| *permission == "Required")
            .map(|(name, _)| format!("{name}: permission required"))
            .collect::<Vec<_>>()
            .join(" · ");
    }
    let granted = checks
        .iter()
        .filter(|(_, permission)| *permission == "Granted")
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    if !granted.is_empty() {
        return format!("granted · {}", granted.join(" · "));
    }
    // `NotApplicable` means the environment probe did not report a permission
    // result. In particular, it must not be promoted into a green health claim
    // merely because a daemon status response exists. Portal authorization is
    // a separate capability and is reflected by the active route status.
    if capabilities.is_some() && !checks.is_empty() {
        return format!(
            "OS permission not reported/applicable · {} (portal/compositor authorization is separate)",
            checks
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(" · ")
        );
    }
    "unknown · route permission was not reported".into()
}

/// Probe compositor protocols without mutating GUI state. Keeping this work
/// outside the application shell makes diagnostics easier to test and avoids
/// coupling rendering code to backend discovery.
fn wayland_session_from(display: bool, socket: bool) -> bool {
    display || socket
}

fn wayland_session() -> bool {
    wayland_session_from(
        env::var_os("WAYLAND_DISPLAY").is_some(),
        env::var_os("WAYLAND_SOCKET").is_some(),
    )
}

pub(crate) fn probe_protocols() -> Vec<(String, String)> {
    if !wayland_session() {
        return vec![
            ("Organization policy".into(), organization_policy_detail()),
            ("IBus WayExpand engine".into(), ibus_detail()),
            (
                "Wayland protocol probes".into(),
                "skipped: no Wayland session detected".into(),
            ),
        ];
    }

    vec![
        ("Organization policy".into(), organization_policy_detail()),
        ("IBus WayExpand engine".into(), ibus_detail()),
        (
            "input-method-v2".into(),
            match InputMethodSource::probe() {
                Ok(()) => "manager and seat available".into(),
                Err(error) => format!("unavailable: {error}"),
            },
        ),
        (
            "wlroots-virtual-keyboard".into(),
            match WlrootsInjector::probe() {
                Ok(()) => "manager and seat available".into(),
                Err(error) => format!("unavailable: {error}"),
            },
        ),
    ]
}

fn organization_policy_detail() -> String {
    match load_organization_policy() {
        Ok(policy) if policy.is_active() => format!(
            "valid; enforcement={}, safe_mode={}, allowed_backends={:?}",
            if policy.safe_mode {
                "enabled"
            } else {
                "audit-only"
            },
            policy.safe_mode,
            policy.allowed_backends
        ),
        Ok(_) => "valid; enforcement disabled; default permissive policy is active".into(),
        Err(error) => format!("invalid: daemon will refuse startup ({error})"),
    }
}

fn ibus_detail() -> String {
    ibus_detail_for(ibus_engine_available())
}

fn ibus_detail_for(available: bool) -> String {
    if available {
        "installed; available to configure, live GTK/Qt typing not certified".into()
    } else {
        "unsupported: component or runtime executable not discoverable".into()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use wayexpand_core::{BackendKind, BackendState, BackendStatus};

    fn backend(kind: BackendKind, state: BackendState) -> BackendStatus {
        BackendStatus {
            kind,
            state,
            detail: String::new(),
        }
    }

    fn route_status() -> &'static str {
        "source=stdin\nbackend=libei\n"
    }

    #[test]
    fn active_daemon_route_is_the_authority_for_current_permission_health() {
        let backends = [
            backend(BackendKind::Evdev, BackendState::Implemented),
            backend(BackendKind::Libei, BackendState::Implemented),
        ];
        let capabilities = crate::runtime::DaemonCapabilities::default();
        assert_eq!(
            super::permission_health(
                &backends,
                Some(&capabilities),
                "state=connected\nsource=evdev\nbackend=libei\n",
            ),
            "active route connected · evdev + libei"
        );
        assert_eq!(
            super::permission_health(
                &backends,
                Some(&capabilities),
                "state=permission_required\nsource=evdev\nbackend=libei\n",
            ),
            "permission required · evdev + libei"
        );
        assert_eq!(
            super::permission_health(
                &backends,
                Some(&capabilities),
                "state=portal_revoked\nsource=evdev\nbackend=libei\n",
            ),
            "portal authorization revoked · evdev + libei"
        );
    }

    #[test]
    fn permission_dashboard_does_not_call_not_applicable_available() {
        let backends = [backend(BackendKind::Libei, BackendState::Implemented)];
        let health = super::permission_health(
            &backends,
            Some(&crate::runtime::DaemonCapabilities::default()),
            route_status(),
        );
        assert!(
            health.starts_with("OS permission not reported/applicable"),
            "{health}"
        );
        assert!(!health.starts_with("available"), "{health}");
    }

    #[test]
    fn permission_dashboard_reports_grants_and_requirements_explicitly() {
        let capabilities = crate::runtime::DaemonCapabilities::default();
        let granted = [backend(BackendKind::Libei, BackendState::Available)];
        assert_eq!(
            super::permission_health(&granted, Some(&capabilities), route_status()),
            "granted · libei"
        );

        let required = [backend(
            BackendKind::Libei,
            BackendState::RequiresPermission,
        )];
        assert_eq!(
            super::permission_health(&required, Some(&capabilities), route_status()),
            "libei: permission required"
        );
    }

    #[test]
    fn route_assessment_never_overstates_unknown_or_best_effort_guarantees() {
        use super::AssessmentState::{Blocked, Ready, Unknown, Warning};
        use crate::runtime::{DaemonCapabilities, ReplacementAssurance, RouteState};

        let mut capabilities = DaemonCapabilities {
            replacement_assurance: Some(ReplacementAssurance::Verified),
            capture_exclusive: Some(true),
            window_tracker_connected: Some(true),
            window_identity_exact: Some(true),
            inject_full_unicode: Some(true),
            ..DaemonCapabilities::default()
        };
        let ready = super::assess_route(
            Some(true),
            Some(RouteState::Connected),
            Some(&capabilities),
            "active route connected · input-method-v2 + input-method-v2",
        );
        assert_eq!(ready.capture, Ready);
        assert_eq!(ready.insertion, Ready);
        assert_eq!(ready.permissions, Ready);
        assert_eq!(ready.focus, Ready);

        capabilities.capture_exclusive = Some(false);
        let nonexclusive_capture = super::assess_route(
            Some(true),
            Some(RouteState::Connected),
            Some(&capabilities),
            "active route connected · evdev + libei",
        );
        assert_eq!(nonexclusive_capture.capture, Warning);

        capabilities.capture_exclusive = None;
        let unreported_capture = super::assess_route(
            Some(true),
            Some(RouteState::Connected),
            Some(&capabilities),
            "active route connected · unknown route",
        );
        assert_eq!(unreported_capture.capture, Unknown);

        capabilities.capture_exclusive = Some(true);

        capabilities.inject_full_unicode = None;
        let unicode_unreported = super::assess_route(
            Some(true),
            Some(RouteState::Connected),
            Some(&capabilities),
            "active route connected · input-method-v2 + input-method-v2",
        );
        assert_eq!(unicode_unreported.insertion, Unknown);

        capabilities.inject_full_unicode = Some(false);
        let unicode_limited = super::assess_route(
            Some(true),
            Some(RouteState::Connected),
            Some(&capabilities),
            "active route connected · input-method-v2 + input-method-v2",
        );
        assert_eq!(unicode_limited.insertion, Warning);

        capabilities.replacement_assurance = Some(ReplacementAssurance::FocusTrackedBestEffort);
        capabilities.inject_full_unicode = Some(true);
        capabilities.window_identity_exact = Some(false);
        let best_effort = super::assess_route(
            Some(true),
            Some(RouteState::Connected),
            Some(&capabilities),
            "OS permission not reported/applicable · libei",
        );
        assert_eq!(best_effort.insertion, Warning);
        assert_eq!(best_effort.permissions, Unknown);
        assert_eq!(best_effort.focus, Warning);

        let blocked = super::assess_route(
            Some(true),
            Some(RouteState::PermissionRequired),
            Some(&DaemonCapabilities::default()),
            "permission required · evdev",
        );
        assert_eq!(blocked.capture, Blocked);
        assert_eq!(blocked.permissions, Blocked);
        assert_eq!(blocked.insertion, Unknown);
    }

    #[test]
    fn socket_based_wayland_sessions_are_recognized_by_the_probe_gate() {
        assert!(super::wayland_session_from(false, true));
        assert!(super::wayland_session_from(true, false));
        assert!(!super::wayland_session_from(false, false));
    }

    #[test]
    fn ibus_diagnostics_expose_availability_without_claiming_certification() {
        assert!(super::ibus_detail_for(true).contains("not certified"));
        assert!(super::ibus_detail_for(false).starts_with("unsupported:"));
    }

    #[cfg(unix)]
    #[test]
    fn compositor_version_probe_kills_descendant_holding_stdout() {
        let started = Instant::now();
        let version = super::bounded_version("sh", &["-c", "sleep 5 & printf 'KWin 99.1\\n'"]);
        assert_eq!(version.as_deref(), Some("KWin 99.1"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn compositor_version_probe_deadline_survives_continuous_output() {
        let started = Instant::now();
        let version = super::bounded_version("sh", &["-c", "while :; do printf x; done"]);
        assert_eq!(version, None);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "continuous output exceeded the version-probe deadline: {:?}",
            started.elapsed()
        );
    }
}
