use std::env;

use wayexpand_backend_ibus::engine_available as ibus_engine_available;
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::load_organization_policy;

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
            "valid; safe_mode={}, allowed_backends={:?}",
            policy.safe_mode, policy.allowed_backends
        ),
        Ok(_) => "valid; default permissive policy is active".into(),
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
