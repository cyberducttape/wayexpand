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
    use super::classify_health;

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
