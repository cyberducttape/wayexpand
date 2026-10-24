use std::{
    env,
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use wayexpand_backend_ibus::engine_available as ibus_engine_available;
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{load_organization_policy, BackendStatus};

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
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_millis(400);
    loop {
        if child.try_wait().ok().flatten().is_some() {
            let mut output = String::new();
            child
                .stdout
                .take()?
                .take(256)
                .read_to_string(&mut output)
                .ok()?;
            let line = output.lines().next()?.trim();
            return (!line.is_empty()).then(|| line.to_owned());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
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
    if capabilities.is_some() && !checks.is_empty() {
        format!(
            "available · {}",
            checks
                .iter()
                .map(|(name, permission)| format!("{name} {permission}"))
                .collect::<Vec<_>>()
                .join(" · ")
        )
    } else {
        "unknown · route permission was not reported".into()
    }
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
}
