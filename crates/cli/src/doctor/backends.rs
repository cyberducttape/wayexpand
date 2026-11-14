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

/// An enabled evdev unit without its consent marker never starts, and
/// systemd records that as a skipped condition rather than a failure.
pub(crate) fn print_evdev_consent_diagnostics() -> bool {
    let enabled = crate::setup::run_setup_command(
        "systemctl",
        &["--user", "is-enabled", "--quiet", "wayexpand-evdev.service"],
    )
    .is_ok_and(|status| status.success());
    if !enabled {
        return true;
    }
    match crate::setup::evdev_consent_marker() {
        Some(marker) if marker.exists() => true,
        marker => {
            println!("evdev service: enabled but inert; its consent marker is missing");
            let legacy = crate::setup::legacy_evdev_consent_marker()
                .filter(|legacy| Some(legacy) != marker.as_ref() && legacy.exists());
            match (&marker, legacy) {
                // Older installers honored XDG_CONFIG_HOME; the unit never did.
                (Some(marker), Some(legacy)) => println!(
                    "  Your consent is at its old location {}; move it where the service checks: mv {} {}",
                    legacy.display(),
                    legacy.display(),
                    marker.display()
                ),
                (Some(marker), None) => println!(
                    "  After reviewing docs/BACKENDS_SENSITIVE_FIELDS.md, create it with: install -m 0600 /dev/null {}",
                    marker.display()
                ),
                (None, _) => {}
            }
            println!("  Or disable it: systemctl --user disable wayexpand-evdev.service");
            false
        }
    }
}

/// One-line IBus verdict for doctor, naming the first broken link and its
/// fix; `wayexpand ibus status` prints every check.
fn print_ibus_route_summary(policy_allowed: bool) -> bool {
    let report = wayexpand_backend_ibus::chain::diagnose();
    if !report.installed_anywhere() {
        println!("IBus WayExpand engine: not installed (install the IBus component to use it)");
        return true;
    }
    if !policy_allowed {
        println!("IBus WayExpand engine: installed but disallowed by organization policy");
        return true;
    }
    let status = super::status::read_daemon_status()
        .ok()
        .and_then(|response| super::status::status_as_json(&response).ok());
    let overlap = crate::commands::ibus::RouteOverlap::from_status(status.as_ref());
    let route = crate::commands::ibus::report_json(&report, &overlap);
    if route["ready"].as_bool() == Some(true) {
        println!(
            "IBus route: ready (sensitive-field aware, verified non-atomic replacement); check an app with `wayexpand ibus watch`"
        );
        return true;
    } else {
        // A live-shell toolkit environment warning is the actionable cause
        // when IBus itself is selected but GTK/Qt apps are not sending input.
        // Prefer it over downstream "no field input yet" evidence.
        let first_issue = primary_ibus_issue(&report);
        let (title, detail, fix) = match first_issue {
            Some(check) => (check.title, check.detail.clone(), check.fix.clone()),
            None => (
                "No second route expands the same keys",
                route["other_route"]["detail"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
                route["other_route"]["fix"].as_str().map(str::to_owned),
            ),
        };
        let problem_prefix = first_issue
            .and_then(|check| check.problem_id)
            .map(|problem_id| format!("{problem_id}: "))
            .unwrap_or_default();
        println!("IBus route: NOT READY — {problem_prefix}{title}: {detail}");
        if let Some(fix) = fix {
            println!("  Fix: {fix}");
        }
        println!("  Run `wayexpand ibus status` for every check.");
    }
    !super::json::selected_ibus_has_problem(&route)
}

fn primary_ibus_issue(
    report: &wayexpand_backend_ibus::chain::IbusChainReport,
) -> Option<&wayexpand_backend_ibus::chain::IbusCheck> {
    // Preserve hard failures first; a toolkit warning is the next useful
    // cause when the engine is selected but has not received field input.
    let readiness = report.readiness();
    let actionable_environment = (readiness.registered && readiness.selected)
        .then(|| report.check("desktop-shell-environment"))
        .flatten()
        .filter(|check| check.state != wayexpand_backend_ibus::chain::IbusCheckState::Pass);
    let actionable_scope_warning = report
        .check("manifest-scope")
        .filter(|check| check.state == wayexpand_backend_ibus::chain::IbusCheckState::Warn);
    report
        .first_failure()
        .or(actionable_scope_warning)
        .or(actionable_environment)
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
    if let Some(report) = crate::gnome_extension::doctor_report() {
        println!(
            "GNOME window tracker: {} — {}",
            if report["ready"] == true {
                "ready"
            } else {
                "NOT READY"
            },
            report["detail"].as_str().unwrap_or("")
        );
        if let Some(build) = report.get("build_identity") {
            println!(
                "  extension build: {} (disk {}, live {})",
                build["runtime_state"].as_str().unwrap_or("unknown"),
                build["on_disk"].as_str().unwrap_or("unavailable"),
                build["live"].as_str().unwrap_or("unverified")
            );
        }
        if let Some(fix) = report["fix"].as_str() {
            println!("  Fix: {fix}");
        }
    }
    let ibus_policy_allowed = setup_backend_allowed(&policy, "ibus");
    let ibus_installed = ibus_engine_available();
    let ibus_route_healthy = print_ibus_route_summary(ibus_policy_allowed);
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
    // Portal authorization is interactive. This desktop-name check is only a
    // provider candidate hint and is never counted as interface availability,
    // authorization, EIS connection, or injection readiness.
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
    ibus_route_healthy && (ibus_available || !available_combinations.is_empty())
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

#[cfg(test)]
mod ibus_summary_tests {
    use super::primary_ibus_issue;
    use wayexpand_backend_ibus::chain::{IbusChainReport, IbusCheck, IbusCheckState, IbusDesktop};

    fn check(id: &'static str, state: IbusCheckState) -> IbusCheck {
        IbusCheck {
            id,
            title: id,
            state,
            problem_id: None,
            detail: String::new(),
            fix: None,
        }
    }

    fn report(shell: IbusCheckState, desktop: IbusCheckState) -> IbusChainReport {
        IbusChainReport {
            desktop: IbusDesktop::KdeWayland,
            checks: vec![
                check("registered", IbusCheckState::Pass),
                check("selected", IbusCheckState::Pass),
                check("desktop", desktop),
                check("field-input", IbusCheckState::Unknown),
                check("desktop-shell-environment", shell),
            ],
            activity: None,
        }
    }

    #[test]
    fn doctor_prefers_plasma_environment_cause_over_missing_field_activity() {
        let report = report(IbusCheckState::Warn, IbusCheckState::Pass);
        assert_eq!(
            primary_ibus_issue(&report).map(|check| check.id),
            Some("desktop-shell-environment")
        );
    }

    #[test]
    fn doctor_surfaces_a_shared_ibus_manifest_bound_to_one_users_home() {
        let mut report = report(IbusCheckState::Pass, IbusCheckState::Pass);
        report.checks.insert(
            0,
            IbusCheck {
                id: "manifest-scope",
                title: "Shared IBus registration is multi-user safe",
                state: IbusCheckState::Fail,
                problem_id: Some("WX-IBUS-MULTIUSER-001"),
                detail: "shared manifest resolves into a user's home".into(),
                fix: Some("repair the shared IBus registration".into()),
            },
        );
        assert_eq!(
            primary_ibus_issue(&report).map(|check| check.id),
            Some("manifest-scope")
        );
    }

    #[test]
    fn doctor_preserves_hard_ibus_failures_ahead_of_environment_warnings() {
        let report = report(IbusCheckState::Warn, IbusCheckState::Fail);
        assert_eq!(
            primary_ibus_issue(&report).map(|check| check.id),
            Some("desktop")
        );
    }
}
