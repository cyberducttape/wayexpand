use std::env;

use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_wlroots::WlrootsInjector;

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
        return vec![(
            "Wayland protocol probes".into(),
            "skipped: no Wayland session detected".into(),
        )];
    }

    vec![
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

#[cfg(test)]
mod tests {
    #[test]
    fn socket_based_wayland_sessions_are_recognized_by_the_probe_gate() {
        assert!(super::wayland_session_from(false, true));
        assert!(super::wayland_session_from(true, false));
        assert!(!super::wayland_session_from(false, false));
    }
}
