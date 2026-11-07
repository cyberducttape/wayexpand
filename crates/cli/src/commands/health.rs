//! Stable operational health contract for service and fleet monitoring.

use super::Args;
use crate::errors::{classified_error, CliErrorKind};
use crate::*;

const HEALTH_SCHEMA: u32 = 1;

fn classify_health(
    config_valid: bool,
    snapshot: Option<&serde_json::Value>,
) -> (&'static str, u8, &'static str, bool) {
    if !config_valid {
        return ("configuration_problem", 3, "configuration_invalid", false);
    }
    let Some(snapshot) = snapshot else {
        return ("unavailable", 2, "daemon_unreachable", false);
    };
    let matching = snapshot
        .get("matching_available")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let raw_state = snapshot.get("state").and_then(serde_json::Value::as_str);
    let unavailable_reason = snapshot
        .get("matching_unavailable_reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("status_incomplete");
    let output_health = snapshot
        .get("output_health")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let schema_compatible = snapshot
        .get("status_schema")
        .and_then(serde_json::Value::as_u64)
        == Some(u64::from(CONTROL_STATUS_SCHEMA));
    if matches!(
        raw_state,
        Some("authorization_required" | "waiting_for_authorization")
    ) {
        (
            "waiting_for_authorization",
            4,
            "authorization_required",
            matching,
        )
    } else if !schema_compatible {
        ("degraded", 1, "daemon_protocol_incompatible", matching)
    } else if matching && output_health == "ok" {
        ("ready", 0, "none", matching)
    } else {
        let reason = match unavailable_reason {
            "capture_disconnected" => "capture_disconnected",
            "keyboard_layout_unavailable" => "keyboard_layout_unavailable",
            "replacement_route_unavailable" => "replacement_route_unavailable",
            "paused" => "paused",
            "configuration_unhealthy" => "configuration_unhealthy",
            "replacement_not_admitted" => "replacement_not_admitted",
            "none" => "output_not_healthy",
            _ => "status_incomplete",
        };
        ("degraded", 1, reason, matching)
    }
}

fn application_coverage(snapshot: Option<&serde_json::Value>) -> serde_json::Value {
    let terminal_only = snapshot
        .and_then(|status| status.get("terminal_apps_only"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if terminal_only {
        serde_json::json!({
            "scope":"daemon_route_configured_terminals_only",
            "terminal_only":true,
            "desktop_wide":null,
            "browser_fields_covered":null,
            "daemon_route": {
                "desktop_wide":false,
                "browser_fields_covered":false
            },
            "detail":"the daemon route matches only configured terminal applications; another route such as IBus may cover browser fields, but this command does not verify it"
        })
    } else {
        serde_json::json!({
            "scope":"active_route",
            "terminal_only":false,
            "desktop_wide":null,
            "browser_fields_covered":null,
            "detail":"health describes the active daemon route; it does not certify input delivery in every application"
        })
    }
}

/// Summarize route-specific safety limits without making operational health
/// pretend to be the independent security-readiness report. These fields make
/// important limitations visible to monitoring that consumes `health` rather
/// than `status` or `doctor --security`.
fn route_safety(snapshot: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(snapshot) = snapshot else {
        return serde_json::json!({
            "sensitive_field_signal":"unknown",
            "target_identity":"unknown",
            "replacement":"unknown",
            "production_certified":false,
            "limitations":["daemon_status_unavailable"]
        });
    };

    let sensitive_focus = snapshot
        .get("capture_sensitive_focus")
        .and_then(serde_json::Value::as_bool);
    let atomic_replace = snapshot
        .get("inject_atomic_replace")
        .and_then(serde_json::Value::as_bool);
    let exact_identity = snapshot
        .get("window_identity_exact")
        .and_then(serde_json::Value::as_bool);
    let compatibility_mode = snapshot
        .get("compatibility_mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let mut limitations = Vec::new();

    let sensitive_field_signal = match sensitive_focus {
        Some(true) => "available",
        Some(false) => {
            limitations.push("input_privacy_unavailable");
            "unavailable"
        }
        None => {
            limitations.push("input_privacy_unknown");
            "unknown"
        }
    };

    let target_identity = match exact_identity {
        Some(true) => "exact",
        Some(false) => {
            limitations.push("target_identity_not_exact");
            "not_exact"
        }
        None => {
            limitations.push("target_identity_unknown");
            "unknown"
        }
    };

    let replacement = match atomic_replace {
        Some(true) => "atomic",
        Some(false) if compatibility_mode == "off" => {
            limitations.push("non_atomic_automatic_replacement_refused");
            "non_atomic_refused_by_default"
        }
        Some(false) if matches!(compatibility_mode, "tracked-focus" | "untracked-focus") => {
            limitations.push("non_atomic_compatibility_override_enabled");
            "non_atomic_compatibility_override"
        }
        Some(false) | None => {
            limitations.push("replacement_guarantee_unknown");
            "unknown"
        }
    };

    // No runtime capability probe is a substitute for reviewed, real-session
    // certification. Keep this false until such evidence is explicitly
    // connected to the active route.
    limitations.push("route_not_production_certified");
    serde_json::json!({
        "sensitive_field_signal":sensitive_field_signal,
        "target_identity":target_identity,
        "replacement":replacement,
        "compatibility_mode":compatibility_mode,
        "production_certified":false,
        "limitations":limitations
    })
}

pub(crate) fn health_command(args: Args) -> Result<()> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            _ => usage_bail!("usage: wayexpand health [--json]"),
        }
    }

    let config_path = default_config_path();
    let config_valid = Config::load(&config_path).is_ok();
    let daemon_snapshot = crate::doctor::status::read_daemon_status()
        .and_then(|body| crate::doctor::status::status_as_json(&body));

    let daemon = daemon_snapshot.ok();
    let (state, code, reason, matching_available) = classify_health(config_valid, daemon.as_ref());
    let errors = if code == 0 {
        Vec::<serde_json::Value>::new()
    } else {
        let (id, component, repair_available) = match reason {
            "configuration_invalid" => ("WX-CONFIG-001", "configuration", false),
            "daemon_unreachable" => ("WX-SERVICE-001", "control-service", true),
            "authorization_required" => ("WX-PORTAL-001", "input-backend", false),
            "daemon_protocol_incompatible" => ("WX-VERSION-001", "daemon", true),
            "capture_disconnected" | "keyboard_layout_unavailable" => {
                ("WX-INPUT-001", "input-backend", false)
            }
            _ => ("WX-OUTPUT-001", "output-backend", false),
        };
        vec![serde_json::json!({
            "id":id,
            "component":component,
            "message":reason,
            "repair_available":repair_available,
        })]
    };

    let report = serde_json::json!({
        "schema_version": HEALTH_SCHEMA,
        "healthy": code == 0,
        "operational_state": state,
        "exit_code": code,
        "reason": reason,
        "errors": errors,
        "application_coverage": application_coverage(daemon.as_ref()),
        "route_safety": route_safety(daemon.as_ref()),
        "configuration": {
            "path": config_path,
            "valid": config_valid,
        },
        "daemon": daemon.as_ref().map(|snapshot| serde_json::json!({
            "reachable": true,
            "status_schema": snapshot.get("status_schema"),
            "status_schema_compatible": snapshot.get("status_schema").and_then(serde_json::Value::as_u64) == Some(u64::from(CONTROL_STATUS_SCHEMA)),
            "commit": snapshot.get("daemon_commit"),
            "source": snapshot.get("source"),
            "backend": snapshot.get("backend"),
            "terminal_apps_only": snapshot.get("terminal_apps_only"),
            "state": snapshot.get("state"),
            "output_health": snapshot.get("output_health"),
            "matching_available": matching_available,
            "matching_unavailable_reason": snapshot.get("matching_unavailable_reason"),
            "production_certified": false,
        })).unwrap_or_else(|| serde_json::json!({"reachable": false})),
    });

    if json {
        println!("{report}");
    } else {
        println!("WayExpand operational health: {state}");
        println!("Reason: {reason}");
        println!(
            "Configuration: {}",
            if config_valid { "valid" } else { "invalid" }
        );
        if let Some(daemon) = daemon.as_ref() {
            println!(
                "Daemon: {} / {} (matching available: {}; production certified: no)",
                daemon
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown"),
                daemon
                    .get("backend")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown"),
                matching_available,
            );
            let coverage = application_coverage(Some(daemon));
            println!(
                "Application scope: {} — {}",
                coverage["scope"].as_str().unwrap_or("unknown"),
                coverage["detail"]
                    .as_str()
                    .unwrap_or("coverage unavailable"),
            );
            let safety = route_safety(Some(daemon));
            let limitations = safety["limitations"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("Route safety: {limitations}");
        } else {
            println!("Daemon: unavailable");
        }
    }

    if code == 0 {
        Ok(())
    } else {
        Err(classified_error(
            CliErrorKind::Health(code),
            format!("health state is {state}: {reason}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{application_coverage, classify_health, route_safety};

    #[test]
    fn terminal_only_daemon_route_does_not_claim_global_browser_coverage() {
        let snapshot = serde_json::json!({
            "source":"evdev",
            "terminal_apps_only":true
        });
        let coverage = application_coverage(Some(&snapshot));
        assert_eq!(coverage["scope"], "daemon_route_configured_terminals_only");
        assert!(coverage["desktop_wide"].is_null());
        assert!(coverage["browser_fields_covered"].is_null());
        assert_eq!(coverage["daemon_route"]["desktop_wide"], false);
        assert_eq!(coverage["daemon_route"]["browser_fields_covered"], false);
        assert!(coverage["detail"]
            .as_str()
            .unwrap()
            .contains("does not verify it"));
    }

    #[test]
    fn older_or_missing_status_does_not_invent_application_coverage() {
        let coverage = application_coverage(None);
        assert_eq!(coverage["scope"], "active_route");
        assert!(coverage["desktop_wide"].is_null());
        assert!(coverage["browser_fields_covered"].is_null());
    }

    #[test]
    fn health_exposes_non_atomic_compatibility_and_privacy_limits() {
        let snapshot = serde_json::json!({
            "capture_sensitive_focus": false,
            "window_identity_exact": false,
            "inject_atomic_replace": false,
            "compatibility_mode": "untracked-focus"
        });
        let safety = route_safety(Some(&snapshot));
        assert_eq!(safety["sensitive_field_signal"], "unavailable");
        assert_eq!(safety["target_identity"], "not_exact");
        assert_eq!(safety["replacement"], "non_atomic_compatibility_override");
        assert_eq!(safety["production_certified"], false);
        assert!(safety["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "non_atomic_compatibility_override_enabled"));
        assert!(safety["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "input_privacy_unavailable"));
    }

    #[test]
    fn health_distinguishes_atomic_and_default_refusal_routes() {
        let atomic = route_safety(Some(&serde_json::json!({
            "capture_sensitive_focus": true,
            "window_identity_exact": true,
            "inject_atomic_replace": true,
            "compatibility_mode": "off"
        })));
        assert_eq!(atomic["sensitive_field_signal"], "available");
        assert_eq!(atomic["target_identity"], "exact");
        assert_eq!(atomic["replacement"], "atomic");
        let refused = route_safety(Some(&serde_json::json!({
            "capture_sensitive_focus": false,
            "inject_atomic_replace": false,
            "compatibility_mode": "off"
        })));
        assert_eq!(refused["replacement"], "non_atomic_refused_by_default");
        let unknown_mode = route_safety(Some(&serde_json::json!({
            "inject_atomic_replace": false
        })));
        assert_eq!(unknown_mode["replacement"], "unknown");
        assert!(unknown_mode["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "replacement_guarantee_unknown"));
    }

    #[test]
    fn health_codes_distinguish_readiness_and_configuration() {
        let ready = serde_json::json!({
            "matching_available": true,
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA
        });
        assert_eq!(
            classify_health(true, Some(&ready)),
            ("ready", 0, "none", true)
        );
        assert_eq!(
            classify_health(false, Some(&ready)),
            ("configuration_problem", 3, "configuration_invalid", false)
        );
        assert_eq!(
            classify_health(true, None),
            ("unavailable", 2, "daemon_unreachable", false)
        );
    }

    #[test]
    fn health_does_not_confuse_degraded_or_authorization_states_with_ready() {
        let degraded = serde_json::json!({
            "matching_available": false,
            "matching_unavailable_reason": "keyboard_layout_unavailable",
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA
        });
        assert_eq!(
            classify_health(true, Some(&degraded)),
            ("degraded", 1, "keyboard_layout_unavailable", false)
        );
        let authorization = serde_json::json!({"state":"waiting_for_authorization"});
        assert_eq!(
            classify_health(true, Some(&authorization)),
            (
                "waiting_for_authorization",
                4,
                "authorization_required",
                false
            )
        );
        let stale = serde_json::json!({
            "matching_available": true,
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA - 1
        });
        assert_eq!(
            classify_health(true, Some(&stale)),
            ("degraded", 1, "daemon_protocol_incompatible", true)
        );
        let stale = serde_json::json!({
            "matching_available": true,
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA - 1
        });
        assert_eq!(
            classify_health(true, Some(&stale)),
            ("degraded", 1, "daemon_protocol_incompatible", true)
        );
    }
}
