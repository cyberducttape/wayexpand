//! Reproducible, privacy-preserving desktop compatibility report.

use super::Args;
use crate::*;

const SCHEMA_VERSION: u32 = 1;

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

    let rows = serde_json::json!({
        "ibus_registration": check(if readiness.registered {"pass"} else if readiness.component_installed {"fail"} else {"not_applicable"}, if readiness.registered {"IBus registry reports the WayExpand engine"} else {"IBus engine is not registered"}),
        "ibus_activation": check(if readiness.selected && readiness.engine_running {"pass"} else if readiness.registered {"not_tested"} else {"not_applicable"}, if readiness.selected && readiness.engine_running {"WayExpand is selected and its engine is reporting activity"} else {"Selection and live engine activity have not both been observed"}),
        "control_service": check(if control_service_active || route_service_active {"pass"} else {"fail"}, if control_service_active {"the control service is active"} else if route_service_active {"the daemon service for the active capture route is active"} else {"no active control or capture-route service was found"}),
        // Runtime status does not identify or verify an application toolkit.
        "gtk_insertion": check("not_tested", "no controlled GTK target insertion was performed by this report"),
        "qt_insertion": check("not_tested", "no controlled Qt target insertion was performed by this report"),
        "browser_insertion": check("not_tested", "no controlled browser target insertion was performed by this report"),
        "unicode_insertion": check("not_tested", "Unicode capability negotiation is not proof of successful target insertion"),
    });
    let observed_success = readiness.expanded;
    let overall = if observed_success {
        "working_in_observed_scenarios"
    } else {
        "not_yet_verified_end_to_end"
    };
    serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "wayexpand_version": crate::build_info::VERSION,
        "os": os,
        "desktop": desktop,
        "session": session,
        "installation": installation_method(),
        "checks": rows,
        "observed_expansion_since_ibus_engine_start": observed_success,
        "overall": overall,
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
        for (key, label) in [
            ("ibus_registration", "IBus registration"),
            ("ibus_activation", "IBus activation"),
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
        println!("Full desktop certification incomplete");
    }
    Ok(())
}
