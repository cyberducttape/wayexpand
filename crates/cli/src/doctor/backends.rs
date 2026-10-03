//! Backend availability, session detection, capture readiness, and automatic selection.

use super::policy::load_policy;
use crate::*;

pub(crate) fn session_description() -> &'static str {
    match env::var("XDG_SESSION_TYPE").ok().as_deref() {
        Some("wayland") if env::var_os("DISPLAY").is_some() => "Wayland (XWayland available)",
        Some("wayland") => "Wayland",
        Some("x11") => "X11",
        _ if env::var_os("WAYLAND_DISPLAY").is_some() => "Wayland",
        _ if env::var_os("DISPLAY").is_some() => "X11 or XWayland",
        _ => "not detected",
    }
}

pub(crate) fn print_backend_diagnostics(include_experimental_input_method: bool) -> bool {
    let backends = discover_backends();
    for status in &backends {
        let detail = format!(
            "{}; implementation={}, availability={}, permission={}",
            status.detail,
            status.implementation(),
            status.availability(),
            status.permission()
        );
        println!("{:28} {:?} ({})", status.kind, status.state, detail);
    }
    let policy = load_policy().unwrap_or_else(|_| OrganizationPolicy {
        safe_mode: true,
        allowed_backends: vec!["none".into()],
        ..OrganizationPolicy::default()
    });
    let ibus_policy_allowed = setup_backend_allowed(&policy, "ibus");
    let ibus_installed = ibus_engine_available();
    if ibus_installed && ibus_policy_allowed {
        println!(
            "IBus WayExpand engine: installed and ready to configure (sensitive-field aware, non-atomic replacement)"
        );
    } else if ibus_installed {
        println!("IBus WayExpand engine: installed but disallowed by organization policy");
    } else {
        println!("IBus WayExpand engine: not installed or discoverable (install the IBus component to use it)");
    }
    // Doctor is also used in CI and for validating a config outside a desktop
    // session. Explain the session limitation; the caller still reports an
    // unhealthy result when no usable path can be established.
    if !display_session_available() {
        if std::env::var_os("DISPLAY").is_some() {
            println!(
                "X11 session: no native X11 global-capture backend is implemented; evdev + libei "
            );
            println!(
                "is the available cross-session route and requires explicit input permission."
            );
        } else {
            println!("No active Wayland or X11 display detected; backend probes were skipped.");
        }
        return false;
    }
    let live_capabilities = probe_capabilities();
    let wlroots_available = if live_capabilities.has_virtual_keyboard {
        println!("wlroots probe: virtual keyboard globals available");
        true
    } else {
        match WlrootsInjector::probe() {
            Ok(_) => {
                println!("wlroots probe: virtual keyboard globals available");
                true
            }
            Err(error) => {
                println!("wlroots probe: unavailable ({error})");
                false
            }
        }
    };
    let input_method_available = if include_experimental_input_method {
        if live_capabilities.has_input_method_v2 {
            println!("input-method-v2 probe: manager and seat connection succeeded");
            true
        } else {
            match InputMethodSource::probe() {
                Ok(_) => {
                    println!("input-method-v2 probe: manager and seat connection succeeded");
                    true
                }
                Err(error) => {
                    println!("input-method-v2 probe: unavailable ({error})");
                    false
                }
            }
        }
    } else {
        false
    };
    let evdev_readable = live_capabilities.has_dev_input;
    // libei needs a RemoteDesktop portal with EIS support (or an explicit
    // LIBEI_SOCKET); probing the portal would pop a consent dialog, so this
    // only recognizes desktops known to ship one.
    let libei_plausible = libei_portal_candidate();

    // These are non-invasive protocol probes, not end-to-end typing tests.
    // Keep that distinction visible: a successful globals/seat probe cannot
    // prove that a real GTK or Qt client will preserve every key event.
    let mut available_combinations = Vec::new();
    let mut trial_combinations = Vec::new();
    let ibus_available = ibus_installed && ibus_policy_allowed;
    if input_method_available && policy.backend_allowed("input-method-v2") {
        available_combinations
            .push("--source=input-method (protocol probe passed; live typing unverified)");
    }
    if evdev_readable && wlroots_available && policy.backend_allowed("wlroots") {
        available_combinations.push(
            "--source=evdev --backend=wlroots (protocol probe passed; live insertion unverified)",
        );
    }
    if evdev_readable && libei_plausible && policy.backend_allowed("libei") {
        trial_combinations.push(
            "--source=evdev --backend=libei (available to try; interactive authorization required)",
        );
    }

    if !capture_path_available(
        ibus_available,
        available_combinations.len(),
        trial_combinations.len(),
    ) {
        println!("Capture readiness: NOT READY (no source+backend combination detected)");
        if evdev_readable && !libei_plausible {
            println!(
                "evdev is readable, but no output backend was detected: the wlroots probe \
                 failed and this desktop is not known to provide a libei portal."
            );
        } else {
            println!(
                "Next step: use a compositor with input-method-v2 support, or \
                 `--source=evdev` (prefer active-seat logind/uaccess ACLs; the broader `input` \
                 group is a legacy fallback; see SECURITY.md for the \
                 sensitive-field tradeoff) paired with `--backend=wlroots` or `--backend=libei`."
            );
        }
        return false;
    }
    if ibus_available && available_combinations.is_empty() && trial_combinations.is_empty() {
        println!(
            "Capture readiness: AVAILABLE TO TRY (IBus is installed; live client typing is not verified)"
        );
    } else if available_combinations.is_empty() && !trial_combinations.is_empty() {
        println!(
            "Capture readiness: AUTHORIZATION REQUIRED (libei was detected heuristically; interactive portal consent is required)"
        );
    } else if available_combinations.is_empty() {
        println!(
            "Capture readiness: AVAILABLE TO TRY (no backend was verified; interactive authorization required)"
        );
    } else {
        println!(
            "Capture readiness: AVAILABLE TO TRY (protocol probe passed; end-to-end typing is not verified)"
        );
    }
    if ibus_available {
        println!(
            "  available to try: IBus committed-text path (password/PIN awareness; live typing unverified)"
        );
    }
    println!(
        "Automatic selection: stdin + libei (raw evdev capture is disabled unless `--source=evdev` is explicitly selected)"
    );
    if evdev_readable {
        println!(
            "evdev probe: readable, but not enabled automatically; explicit `--source=evdev` acknowledges global keyboard visibility"
        );
    }
    for combination in &available_combinations {
        println!("  available to try: wayexpand-daemon {combination}");
    }
    for combination in &trial_combinations {
        println!("  available to try: wayexpand-daemon {combination}");
    }
    println!("Setup guidance:");
    if input_method_available {
        println!(
            "  experimental opt-in: systemctl --user enable --now wayexpand-input-method.service"
        );
        println!("  warning: unsupported non-text keys may be lost");
    }
    if evdev_readable {
        println!(
            "  explicit evdev: prefer the active-seat installer/uaccess grant; the input group is a broader legacy fallback, and password-field signaling is unavailable"
        );
    }
    if libei_plausible && evdev_readable {
        if let Some(path) = portal_token_path() {
            println!(
                "  libei portal token: {} (use `wayexpand portal reset` to re-authorize)",
                if path.is_file() {
                    "present"
                } else {
                    "not present"
                }
            );
        }
    }
    ibus_available || !available_combinations.is_empty()
}

pub(crate) fn print_backend_selection_explain() {
    match explain_auto_selection() {
        Ok(explanation) => print!("{explanation}"),
        Err(error) => eprintln!("backend selection failed: {error}"),
    }
}

pub(crate) fn capture_path_available(
    ibus_available: bool,
    available_count: usize,
    trial_count: usize,
) -> bool {
    ibus_available || available_count > 0 || trial_count > 0
}

pub(crate) fn display_session_available() -> bool {
    display_session_flags(
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("WAYLAND_SOCKET").is_some(),
        std::env::var_os("DISPLAY").is_some(),
    )
}

pub(crate) fn display_session_flags(
    wayland_display: bool,
    wayland_socket: bool,
    x11_display: bool,
) -> bool {
    wayland_display || wayland_socket || x11_display
}

pub(crate) fn backend_policy_allowed(
    kind: BackendKind,
    policy: &OrganizationPolicy,
) -> Option<bool> {
    kind.policy_name()
        .map(|backend| policy.backend_allowed(backend))
}

pub(crate) fn automatic_selection_is_ready(
    source: &str,
    backend: &str,
    policy: &OrganizationPolicy,
) -> bool {
    source != "stdin"
        && policy.backend_allowed(wayexpand_core::policy_backend_name(source, backend))
}

/// Classify non-invasive session probes without calling them an end-to-end
/// guarantee. A protocol/global probe can establish that a path is worth
/// trying, but only a compositor/client harness can verify typing integrity.
pub(crate) fn capture_readiness(
    capabilities: &wayexpand_backend_selection::Capabilities,
    ibus_installed: bool,
    policy: &OrganizationPolicy,
) -> (&'static str, &'static str) {
    if (ibus_installed || capabilities.has_input_method_v2)
        && policy.backend_allowed("input-method-v2")
    {
        return (
            "available-to-try",
            "a protocol or IBus probe succeeded; live client typing is not verified",
        );
    }
    if capabilities.has_dev_input
        && capabilities.has_virtual_keyboard
        && policy.backend_allowed("wlroots")
    {
        return (
            "available-to-try",
            "evdev capture and a virtual-keyboard output probe succeeded; live typing is not verified",
        );
    }
    if capabilities.has_dev_input
        && capabilities.has_direct_libei_socket
        && policy.backend_allowed("libei")
    {
        return (
            "available-to-try",
            "evdev capture and a direct EIS socket were detected; live typing is not verified",
        );
    }
    if capabilities.has_dev_input && libei_portal_candidate() && policy.backend_allowed("libei") {
        return (
            "authorization-required",
            "evdev is readable and a libei portal candidate was detected; interactive authorization is required",
        );
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("WAYLAND_SOCKET").is_none()
    {
        return (
            "not-probed",
            "no active Wayland session was detected; backend probes were skipped",
        );
    }
    (
        "unavailable",
        "no non-invasive source and output path was detected",
    )
}
