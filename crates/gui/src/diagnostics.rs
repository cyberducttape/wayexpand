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
    let stdout = supervisor.child_mut()?.stdout.as_mut()?;
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
                match supervisor.child_mut()?.stdout.as_mut()?.read(&mut buffer) {
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
