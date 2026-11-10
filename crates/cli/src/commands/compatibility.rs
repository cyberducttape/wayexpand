//! Reproducible, privacy-preserving desktop compatibility report.

use super::Args;
use crate::*;

const SCHEMA_VERSION: u32 = 3;

fn environment_value(key: &str) -> Option<String> {
    let contents = fs::read_to_string("/etc/os-release").ok()?;
    contents.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        if name != key {
            return None;
        }
        Some(value.trim_matches('"').to_owned())
    })
}

fn installation_method() -> &'static str {
    let Ok(executable) = env::current_exe() else {
        return "unknown";
    };
    let path = executable.to_string_lossy();
    if path.contains("/.local/lib/wayexpand/") {
        "user-release"
    } else if path.starts_with("/usr/bin/") || path.starts_with("/usr/local/bin/") {
        "system-installation (package manager not verified)"
    } else {
        "unknown"
    }
}

fn check(status: &'static str, evidence: impl Into<String>) -> serde_json::Value {
    serde_json::json!({"status": status, "evidence": evidence.into()})
}

fn ibus_activation_check(
    readiness: wayexpand_backend_ibus::chain::IbusReadiness,
    active_source: Option<&str>,
    terminal_apps_only: Option<bool>,
) -> serde_json::Value {
    if readiness.selected && readiness.engine_running {
        match active_source {
            Some("input-method") => check(
                "pass",
                "IBus is the daemon's input-method route, and its engine is selected and reporting activity",
            ),
            Some("evdev") if terminal_apps_only == Some(true) => check(
                "pass",
                "IBus is selected outside the configured evdev-owned applications; raw evdev still observes global keyboard input and has no sensitive-field signal",
            ),
            Some(source) => check(
                "not_active_route",
                format!("the IBus engine is selected and reporting activity, but the daemon reports {source} as its active capture route"),
            ),
            None => check(
                "not_tested",
                "the IBus engine is selected and reporting activity, but the active daemon route is unknown",
            ),
        }
    } else if readiness.registered {
        check(
            "not_tested",
            "selection and live engine activity have not both been observed",
        )
    } else {
        check(
            "not_applicable",
            "the WayExpand IBus engine is not registered",
        )
    }
}

fn active_route_report(daemon: Option<&serde_json::Value>) -> serde_json::Value {
    let field = |name: &str| daemon.and_then(|value| value.get(name)).cloned();
    let optional_bool = |name: &str| field(name).and_then(|value| value.as_bool());
    let source = field("source");
    let backend = field("backend");
    let route_id = field("route_id");
    let state = field("state");
    let route_available = optional_bool("route_available");
    let matching_available = optional_bool("matching_available");
    let terminal_apps_only = optional_bool("terminal_apps_only");
    let connected = state.as_ref().and_then(|value| value.as_str()) == Some("connected");
    let matching_ready = matching_available == Some(true);
    let input_privacy = match optional_bool("capture_sensitive_focus") {
        Some(true) => "sensitive_field_signal_reported",
        Some(false) => "no_sensitive_field_signal",
        None => "unknown",
    };
    let insertion_atomicity = match optional_bool("inject_atomic_replace") {
        Some(true) => "atomic",
        Some(false) => "non_atomic",
        None => "unknown",
    };

    serde_json::json!({
        "source": source,
        "backend": backend,
        "route_id": route_id,
        "backend_mode": field("backend_mode"),
        "state": state,
        "route_available": route_available,
        "terminal_apps_only": terminal_apps_only,
        "capture_scope": match source.as_ref().and_then(|value| value.as_str()) {
            Some("evdev") => "global_keyboard_devices",
            Some("input-method") => "ibus_input_contexts",
            Some("stdin") => "simulated_stdin_input",
            Some(_) => "backend_defined",
            None => "unknown",
        },
        "expansion_scope": match terminal_apps_only {
            Some(true) => "configured_applications",
            Some(false) => "not_limited_to_configured_applications",
            None => "unknown",
        },
        "matching_available": matching_available,
        "matching_unavailable_reason": field("matching_unavailable_reason"),
        "input_privacy": input_privacy,
        "insertion_atomicity": insertion_atomicity,
        "target_identity_exact": field("window_identity_exact"),
        "operational_status": if daemon.is_none() {
            "not_observed"
        } else if connected && route_available == Some(true) && matching_ready {
            "connected_and_matching"
        } else {
            "unavailable_or_degraded"
        },
        "operational_evidence_scope": "daemon status only; this does not verify text delivery in a target application",
    })
}

fn report() -> serde_json::Value {
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    let session = env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| {
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            "wayland".into()
        } else if env::var_os("DISPLAY").is_some() {
            "x11".into()
        } else {
            "unknown".into()
        }
    });
    let os = environment_value("PRETTY_NAME").unwrap_or_else(|| "unknown".into());
    let ibus = wayexpand_backend_ibus::chain::diagnose();
    let readiness = ibus.readiness();
    let services = crate::services::ServiceInventory::probe();
    let daemon = crate::doctor::status::read_daemon_status()
        .ok()
        .and_then(|raw| crate::doctor::status::status_as_json(&raw).ok());
    let source = daemon
        .as_ref()
        .and_then(|value| value.get("source"))
        .and_then(serde_json::Value::as_str);
    let control_service_active = services.state(crate::services::UserService::Control).active;
    let route_service_active = source
        .and_then(crate::services::UserService::for_source)
        .is_some_and(|service| services.state(service).active);
    let active_route = active_route_report(daemon.as_ref());
    let active_source = active_route
        .get("source")
        .and_then(serde_json::Value::as_str);
    let terminal_apps_only = active_route
        .get("terminal_apps_only")
        .and_then(serde_json::Value::as_bool);

    let rows = serde_json::json!({
        "ibus_registration": check(if readiness.registered {"pass"} else if readiness.component_installed {"fail"} else {"not_applicable"}, if readiness.registered {"IBus registry reports the WayExpand engine"} else {"IBus engine is not registered"}),
        "ibus_activation": ibus_activation_check(readiness, active_source, terminal_apps_only),
        "active_route": check(
            if daemon.is_none() {"not_tested"} else if active_route["operational_status"] == "connected_and_matching" {"pass"} else {"fail"},
            if daemon.is_none() {"daemon status is unavailable"} else if active_route["operational_status"] == "connected_and_matching" {"the reported active route is connected and matching is available; target-application insertion remains untested"} else {"the daemon does not report a connected route with matching available"},
        ),
        "control_service": check(if control_service_active || route_service_active {"pass"} else {"fail"}, if control_service_active {"the control service is active"} else if route_service_active {"the daemon service for the active capture route is active"} else {"no active control or capture-route service was found"}),
        // Runtime status does not identify or verify an application toolkit.
        "gtk_insertion": check("not_tested", "no controlled GTK target insertion was performed by this report"),
        "qt_insertion": check("not_tested", "no controlled Qt target insertion was performed by this report"),
        "browser_insertion": check("not_tested", "no controlled browser target insertion was performed by this report"),
        "unicode_insertion": check("not_tested", "Unicode capability negotiation is not proof of successful target insertion"),
    });
    // Engine telemetry proves only that the engine attempted an expansion. This
    // report does not observe the target widget, so it cannot certify insertion.
    let engine_expansion_activity_observed = readiness.expanded;
    serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "wayexpand_version": crate::build_info::VERSION,
        "os": os,
        "desktop": desktop,
        "session": session,
        "installation": installation_method(),
        "active_route": active_route,
        "checks": rows,
        "ibus_engine_expansion_activity_since_start": engine_expansion_activity_observed,
        "observed_expansion_since_ibus_engine_start": engine_expansion_activity_observed,
        "overall": "not_yet_verified_end_to_end",
        "certification": "full_desktop_certification_incomplete",
        "privacy": "report contains environment and operational status only; no keystrokes or snippet contents",
    })
}

pub(crate) fn compatibility_report(args: Args) -> Result<()> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            _ => usage_bail!("usage: wayexpand compatibility-report [--json]"),
        }
    }
    let report = report();
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("WayExpand Compatibility Report\n");
        println!("OS: {}", report["os"].as_str().unwrap_or("unknown"));
        println!(
            "Desktop: {}",
            report["desktop"].as_str().unwrap_or("unknown")
        );
        println!(
            "Session: {}",
            report["session"].as_str().unwrap_or("unknown")
        );
        println!(
            "Installation: {}\n",
            report["installation"].as_str().unwrap_or("unknown")
        );
        let active_route = &report["active_route"];
        println!("DAEMON CAPTURE ROUTE");
        println!(
            "  Source: {}",
            active_route["source"].as_str().unwrap_or("unknown")
        );
        println!(
            "  Backend: {}",
            active_route["backend"].as_str().unwrap_or("unknown")
        );
        println!(
            "  Route ID: {}",
            active_route["route_id"].as_str().unwrap_or("unknown")
        );
        println!(
            "  Capture scope: {}",
            active_route["capture_scope"].as_str().unwrap_or("unknown")
        );
        println!(
            "  Expansion scope: {}",
            active_route["expansion_scope"]
                .as_str()
                .unwrap_or("unknown")
        );
        println!(
            "  Matching: {}",
            if active_route["matching_available"] == true {
                "available"
            } else {
                "unavailable or unknown"
            }
        );
        println!(
            "  Sensitive-field signal: {}",
            active_route["input_privacy"].as_str().unwrap_or("unknown")
        );
        println!(
            "  Replacement: {}\n",
            active_route["insertion_atomicity"]
                .as_str()
                .unwrap_or("unknown")
        );
        for (key, label) in [
            ("ibus_registration", "IBus registration"),
            ("ibus_activation", "IBus engine activation"),
            ("active_route", "Active route matching"),
            ("control_service", "Control service"),
            ("gtk_insertion", "GTK insertion"),
            ("qt_insertion", "Qt insertion"),
            ("browser_insertion", "Browser insertion"),
            ("unicode_insertion", "Unicode insertion"),
        ] {
            let row = &report["checks"][key];
            println!(
                "{label}: {}",
                row["status"].as_str().unwrap_or("unknown").to_uppercase()
            );
        }
        println!(
            "\nOverall: {}",
            report["overall"]
                .as_str()
                .unwrap_or("unknown")
                .replace('_', " ")
        );
        println!("Capture and expansion scopes are distinct; evdev reads global keyboard input.");
        println!("Full desktop certification incomplete");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected_running_ibus() -> wayexpand_backend_ibus::chain::IbusReadiness {
        wayexpand_backend_ibus::chain::IbusReadiness {
            selected: true,
            engine_running: true,
            registered: true,
            ..Default::default()
        }
    }

    #[test]
    fn ibus_activation_is_not_mislabeled_as_active_when_evdev_is_selected() {
        let result = ibus_activation_check(selected_running_ibus(), Some("evdev"), Some(false));
        assert_eq!(result["status"], "not_active_route");
        assert!(result["evidence"].as_str().unwrap().contains("evdev"));
    }

    #[test]
    fn ibus_activation_is_primary_when_evdev_is_terminal_scoped() {
        let result = ibus_activation_check(selected_running_ibus(), Some("evdev"), Some(true));
        assert_eq!(result["status"], "pass");
        assert!(result["evidence"]
            .as_str()
            .unwrap()
            .contains("evdev still observes global keyboard input"));
        assert!(result["evidence"]
            .as_str()
            .unwrap()
            .contains("has no sensitive-field signal"));
    }

    #[test]
    fn ibus_activation_passes_only_when_ibus_is_the_reported_route() {
        let result =
            ibus_activation_check(selected_running_ibus(), Some("input-method"), Some(false));
        assert_eq!(result["status"], "pass");
    }

    #[test]
    fn ibus_activation_is_unknown_when_active_route_is_missing() {
        let result = ibus_activation_check(selected_running_ibus(), None, None);
        assert_eq!(result["status"], "not_tested");
    }

    #[test]
    fn route_report_exposes_capture_privacy_and_insertion_atomicity() {
        let daemon = serde_json::json!({
            "source": "evdev",
            "backend": "libei",
            "route_id": "kde-evdev-libei",
            "terminal_apps_only": true,
            "state": "connected",
            "route_available": true,
            "matching_available": true,
            "capture_sensitive_focus": false,
            "inject_atomic_replace": false
        });
        let report = active_route_report(Some(&daemon));
        assert_eq!(report["operational_status"], "connected_and_matching");
        assert_eq!(report["capture_scope"], "global_keyboard_devices");
        assert_eq!(report["expansion_scope"], "configured_applications");
        assert_eq!(report["input_privacy"], "no_sensitive_field_signal");
        assert_eq!(report["insertion_atomicity"], "non_atomic");
        assert!(report["operational_evidence_scope"]
            .as_str()
            .unwrap()
            .contains("does not verify text delivery"));
    }

    #[test]
    fn route_report_does_not_pass_when_route_is_explicitly_unavailable() {
        let daemon = serde_json::json!({
            "source": "evdev",
            "state": "connected",
            "route_available": false,
            "matching_available": true
        });
        let report = active_route_report(Some(&daemon));
        assert_eq!(report["operational_status"], "unavailable_or_degraded");
    }
}
