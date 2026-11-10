//! Stable operational health contract for service and fleet monitoring.

use super::Args;
use crate::errors::{classified_error, CliErrorKind};
use crate::*;

pub(crate) const HEALTH_SCHEMA: u32 = 2;

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
    let enabled_expansions = snapshot
        .get("config_enabled_expansions")
        .and_then(serde_json::Value::as_u64);
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
    } else if matching && output_health == "ok" && enabled_expansions == Some(0) {
        (
            "configuration_problem",
            3,
            "no_enabled_expansions",
            matching,
        )
    } else if matching && output_health == "ok" && enabled_expansions.is_some() {
        ("ready", 0, "none", matching)
    } else {
        let reason = if matching && output_health == "ok" && enabled_expansions.is_none() {
            "status_incomplete"
        } else {
            match unavailable_reason {
                "capture_disconnected" => "capture_disconnected",
                "keyboard_layout_unavailable" => "keyboard_layout_unavailable",
                "replacement_route_unavailable" => "replacement_route_unavailable",
                "paused" => "paused",
                "configuration_unhealthy" => "configuration_unhealthy",
                "replacement_not_admitted" => "replacement_not_admitted",
                "none" => "output_not_healthy",
                _ => "status_incomplete",
            }
        };
        ("degraded", 1, reason, matching)
    }
}

fn application_coverage(snapshot: Option<&serde_json::Value>) -> serde_json::Value {
    let app_scope_only = snapshot
        .and_then(|status| status.get("terminal_apps_only"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if app_scope_only {
        serde_json::json!({
            "scope":"daemon_route_configured_applications_only",
            "terminal_only":null,
            "configured_app_scope_only":true,
            "desktop_wide":null,
            "browser_fields_covered":null,
            "daemon_route": {
                "desktop_wide":false,
                "browser_fields_covered":null
            },
            "detail":"the daemon route matches only its configured application allowlist, which may include explicitly assigned browsers as well as terminals; this status does not verify the active app ID, IBus arbitration, or insertion in a browser field"
        })
    } else {
        serde_json::json!({
            "scope":"active_route",
            "terminal_only":false,
            "configured_app_scope_only":false,
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
            "readiness":{"state":"unknown","ready":false},
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
    let production_certified = false;
    limitations.push("route_not_production_certified");
    let has_known_limitation = limitations.iter().any(|limitation| {
        matches!(
            *limitation,
            "input_privacy_unavailable"
                | "target_identity_not_exact"
                | "non_atomic_automatic_replacement_refused"
                | "non_atomic_compatibility_override_enabled"
        )
    });
    let has_unknown_requirement = sensitive_focus.is_none()
        || atomic_replace.is_none()
        || exact_identity.is_none()
        || limitations.iter().any(|limitation| {
            matches!(
                *limitation,
                "input_privacy_unknown"
                    | "target_identity_unknown"
                    | "replacement_guarantee_unknown"
            )
        });
    let readiness_state = if has_known_limitation {
        "limited"
    } else if has_unknown_requirement {
        "unknown"
    } else if production_certified {
        "certified"
    } else {
        "uncertified"
    };
    serde_json::json!({
        "sensitive_field_signal":sensitive_field_signal,
        "target_identity":target_identity,
        "replacement":replacement,
        "compatibility_mode":compatibility_mode,
        "production_certified":production_certified,
        "readiness":{
            "state":readiness_state,
            "ready":production_certified && !has_known_limitation && !has_unknown_requirement
        },
        "limitations":limitations
    })
}

fn readiness_summary(
    operational_state: &str,
    operational_exit_code: u8,
    route_safety: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "operational": {
            "state": operational_state,
            "ready": operational_exit_code == 0,
            "exit_code": operational_exit_code
        },
        "route_safety": route_safety["readiness"].clone()
    })
}

fn health_report(
    config_path: &std::path::Path,
    config_valid: bool,
    daemon: Option<&serde_json::Value>,
) -> (serde_json::Value, &'static str, u8, &'static str, bool) {
    let (state, code, reason, matching_available) = classify_health(config_valid, daemon);
    let safety = route_safety(daemon);
    let errors = if code == 0 {
        Vec::<serde_json::Value>::new()
    } else {
        let (id, component, repair_available) = match reason {
            "configuration_invalid" => ("WX-CONFIG-001", "configuration", false),
            "configuration_unhealthy" => ("WX-CONFIG-001", "configuration", true),
            "no_enabled_expansions" => ("WX-CONFIG-002", "configuration", false),
            "daemon_unreachable" => ("WX-SERVICE-001", "control-service", true),
            "authorization_required" => ("WX-PORTAL-001", "input-backend", false),
            "daemon_protocol_incompatible" => ("WX-VERSION-001", "daemon", true),
            "status_incomplete" => ("WX-STATUS-001", "daemon-status", true),
            "capture_disconnected" | "keyboard_layout_unavailable" => {
                ("WX-INPUT-001", "input-backend", false)
            }
            "paused" => ("WX-INPUT-001", "matching", true),
            "replacement_not_admitted" => ("WX-POLICY-001", "replacement-policy", false),
            _ => ("WX-OUTPUT-001", "output-backend", false),
        };
        vec![serde_json::json!({
            "id":id,
            "component":component,
            "message":match reason {
                "no_enabled_expansions" => "No enabled expansions are loaded by the daemon",
                "configuration_unhealthy" => "The daemon rejected or has not loaded the active configuration",
                "paused" => "Expansion matching is paused",
                "replacement_not_admitted" => "Automatic replacement is refused by the active safety policy",
                "daemon_protocol_incompatible" => "The running daemon status schema is incompatible with this CLI",
                _ => reason,
            },
            "repair_available":repair_available,
            "suggested_action":match reason {
                "no_enabled_expansions" => "Enable or add an expansion, then run `wayexpand reload`",
                "configuration_unhealthy" => "Run `wayexpand status --json` and `wayexpand validate`, correct configuration errors, then run `wayexpand reload`",
                "paused" => "Run `wayexpand resume`, then check `wayexpand health` again",
                "replacement_not_admitted" => "Run `wayexpand policy check --json` and inspect `wayexpand status`; authorize non-atomic replacement only if you explicitly accept its risks",
                "daemon_protocol_incompatible" => "Run `wayexpand upgrade --check`; restart or upgrade the daemon only after reviewing the reported process and compatibility details",
                "status_incomplete" => "Run `wayexpand status --json` and check that the daemon reports loaded configuration counts",
                _ => "Run `wayexpand doctor` for diagnostic details",
            },
        })]
    };

    let report = serde_json::json!({
        "schema_version": HEALTH_SCHEMA,
        "healthy": code == 0,
        "operational_state": state,
        "exit_code": code,
        "reason": reason,
        "readiness": readiness_summary(state, code, &safety),
        "errors": errors,
        "application_coverage": application_coverage(daemon),
        "route_safety": safety,
        "configuration": {
            "path": config_path,
            "valid": config_valid,
        },
        "daemon": daemon.map(|snapshot| serde_json::json!({
            "reachable": true,
            "status_schema": snapshot.get("status_schema"),
            "status_schema_compatible": snapshot.get("status_schema").and_then(serde_json::Value::as_u64) == Some(u64::from(CONTROL_STATUS_SCHEMA)),
            "commit": snapshot.get("daemon_commit"),
            "source": snapshot.get("source"),
            "backend": snapshot.get("backend"),
            "terminal_apps_only": snapshot.get("terminal_apps_only"),
            "enabled_expansions": snapshot.get("config_enabled_expansions"),
            "disabled_expansions": snapshot.get("config_disabled_expansions"),
            "state": snapshot.get("state"),
            "output_health": snapshot.get("output_health"),
            "matching_available": matching_available,
            "matching_unavailable_reason": snapshot.get("matching_unavailable_reason"),
            "production_certified": false,
        })).unwrap_or_else(|| serde_json::json!({"reachable": false})),
    });
    (report, state, code, reason, matching_available)
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
    let (report, state, code, reason, matching_available) =
        health_report(&config_path, config_valid, daemon.as_ref());
    let safety = &report["route_safety"];

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
    use super::{
        application_coverage, classify_health, health_report, readiness_summary, route_safety,
        HEALTH_SCHEMA,
    };

    #[test]
    fn emitted_health_report_matches_the_machine_readable_contract() {
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/contracts/cli-json-shapes.json"
        ))
        .expect("CLI JSON shape contract must be valid");
        let snapshot = serde_json::json!({
            "matching_available": true,
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA,
            "config_enabled_expansions": 1,
            "capture_sensitive_focus": false,
            "window_identity_exact": true,
            "inject_atomic_replace": false,
            "compatibility_mode": "off"
        });
        let (report, state, exit_code, reason, matching_available) = health_report(
            std::path::Path::new("/tmp/wayexpand/expansions.toml"),
            true,
            Some(&snapshot),
        );

        let keys = |value: &serde_json::Value| {
            value
                .as_object()
                .expect("contracted value must be an object")
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        let expected = |key: &str| {
            contract[key]
                .as_array()
                .expect("contract entry must be an array")
                .iter()
                .map(|field| {
                    field
                        .as_str()
                        .expect("contract field must be a string")
                        .to_owned()
                })
                .collect::<std::collections::BTreeSet<_>>()
        };

        assert_eq!(keys(&report), expected("health"));
        assert_eq!(keys(&report["daemon"]), expected("health_daemon"));
        assert_eq!(
            report["schema_version"].as_u64(),
            Some(u64::from(HEALTH_SCHEMA))
        );
        assert_eq!(report["schema_version"], contract["health_schema"]);
        assert_eq!(keys(&report["readiness"]), expected("health_readiness"));
        assert_eq!(
            keys(&report["readiness"]["operational"]),
            expected("health_operational_readiness")
        );
        assert_eq!(
            keys(&report["readiness"]["route_safety"]),
            expected("health_route_safety_readiness")
        );
        assert_eq!(state, "ready");
        assert_eq!(exit_code, 0);
        assert_eq!(reason, "none");
        assert!(matching_available);
        assert_eq!(report["readiness"]["operational"]["ready"], true);
        assert_eq!(report["readiness"]["route_safety"]["state"], "limited");
        assert_eq!(report["readiness"]["route_safety"]["ready"], false);
    }

    #[test]
    fn app_scoped_daemon_route_does_not_claim_terminal_only_or_browser_coverage() {
        let snapshot = serde_json::json!({
            "source":"evdev",
            "terminal_apps_only":true
        });
        let coverage = application_coverage(Some(&snapshot));
        assert_eq!(
            coverage["scope"],
            "daemon_route_configured_applications_only"
        );
        assert!(coverage["desktop_wide"].is_null());
        assert!(coverage["browser_fields_covered"].is_null());
        assert!(coverage["terminal_only"].is_null());
        assert_eq!(coverage["daemon_route"]["desktop_wide"], false);
        assert!(coverage["daemon_route"]["browser_fields_covered"].is_null());
        assert!(coverage["detail"].as_str().unwrap().contains("not verify"));
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
        assert_eq!(safety["readiness"]["state"], "limited");
        assert_eq!(safety["readiness"]["ready"], false);
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
        assert_eq!(atomic["readiness"]["state"], "uncertified");
        assert_eq!(atomic["readiness"]["ready"], false);
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
        assert_eq!(unknown_mode["readiness"]["state"], "unknown");
    }

    #[test]
    fn safety_readiness_never_promotes_missing_capabilities_or_uncertified_routes() {
        let unavailable = route_safety(None);
        assert_eq!(unavailable["readiness"]["state"], "unknown");
        assert_eq!(unavailable["readiness"]["ready"], false);

        let no_runtime_evidence = route_safety(Some(&serde_json::json!({})));
        assert_eq!(no_runtime_evidence["readiness"]["state"], "unknown");
        assert_eq!(no_runtime_evidence["readiness"]["ready"], false);

        let capable_but_uncertified = route_safety(Some(&serde_json::json!({
            "capture_sensitive_focus": true,
            "window_identity_exact": true,
            "inject_atomic_replace": true,
            "compatibility_mode": "off"
        })));
        assert_eq!(capable_but_uncertified["readiness"]["state"], "uncertified");
        assert_eq!(capable_but_uncertified["readiness"]["ready"], false);
    }

    #[test]
    fn operational_health_does_not_imply_route_safety_readiness() {
        let safety = route_safety(Some(&serde_json::json!({
            "capture_sensitive_focus": false,
            "window_identity_exact": true,
            "inject_atomic_replace": false,
            "compatibility_mode": "untracked-focus"
        })));
        let readiness = readiness_summary("ready", 0, &safety);
        assert_eq!(readiness["operational"]["ready"], true);
        assert_eq!(readiness["operational"]["exit_code"], 0);
        assert_eq!(readiness["route_safety"]["state"], "limited");
        assert_eq!(readiness["route_safety"]["ready"], false);
    }

    #[test]
    fn health_codes_distinguish_readiness_and_configuration() {
        let ready = serde_json::json!({
            "matching_available": true,
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA,
            "config_enabled_expansions": 2
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
    fn zero_enabled_expansions_is_a_configuration_problem() {
        let empty = serde_json::json!({
            "matching_available": true,
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA,
            "config_enabled_expansions": 0
        });
        let (report, state, exit_code, reason, matching_available) = health_report(
            std::path::Path::new("/tmp/wayexpand/expansions.toml"),
            true,
            Some(&empty),
        );

        assert_eq!(state, "configuration_problem");
        assert_eq!(exit_code, 3);
        assert_eq!(reason, "no_enabled_expansions");
        assert!(matching_available);
        assert_eq!(report["healthy"], false);
        assert_eq!(report["errors"][0]["id"], "WX-CONFIG-002");
        assert_eq!(report["errors"][0]["component"], "configuration");
        assert_eq!(
            report["errors"][0]["suggested_action"],
            "Enable or add an expansion, then run `wayexpand reload`"
        );
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/contracts/cli-json-shapes.json"
        ))
        .expect("CLI JSON shape contract must be valid");
        let error_keys = report["errors"][0]
            .as_object()
            .expect("health error must be an object")
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let expected_error_keys = contract["health_error"]
            .as_array()
            .expect("health error contract must be an array")
            .iter()
            .map(|field| {
                field
                    .as_str()
                    .expect("health error contract field must be a string")
                    .to_owned()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(error_keys, expected_error_keys);
        assert_eq!(report["daemon"]["enabled_expansions"], 0);
    }

    #[test]
    fn route_failure_takes_precedence_over_an_empty_library() {
        let snapshot = serde_json::json!({
            "matching_available": false,
            "matching_unavailable_reason": "capture_disconnected",
            "output_health": "ok",
            "state": "reconnecting",
            "status_schema": crate::CONTROL_STATUS_SCHEMA,
            "config_enabled_expansions": 0
        });
        assert_eq!(
            classify_health(true, Some(&snapshot)),
            ("degraded", 1, "capture_disconnected", false)
        );
    }

    #[test]
    fn health_does_not_claim_ready_when_expansion_count_is_missing() {
        let incomplete = serde_json::json!({
            "matching_available": true,
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA
        });
        let (report, state, code, reason, matching) = health_report(
            std::path::Path::new("/tmp/wayexpand/expansions.toml"),
            true,
            Some(&incomplete),
        );
        assert_eq!(
            (state, code, reason, matching),
            ("degraded", 1, "status_incomplete", true)
        );
        assert_eq!(report["errors"][0]["id"], "WX-STATUS-001");
        assert_eq!(report["errors"][0]["component"], "daemon-status");
    }

    #[test]
    fn incompatible_daemon_schema_points_to_upgrade_preflight() {
        let stale = serde_json::json!({
            "matching_available": true,
            "output_health": "ok",
            "state": "connected",
            "status_schema": crate::CONTROL_STATUS_SCHEMA - 1,
            "config_enabled_expansions": 1
        });
        let (report, state, exit_code, reason, matching) = health_report(
            std::path::Path::new("/tmp/wayexpand/expansions.toml"),
            true,
            Some(&stale),
        );
        let error = &report["errors"][0];

        assert_eq!(state, "degraded");
        assert_eq!(exit_code, 1);
        assert_eq!(reason, "daemon_protocol_incompatible");
        assert!(matching);
        assert_eq!(error["id"], "WX-VERSION-001");
        assert_eq!(
            error["message"],
            "The running daemon status schema is incompatible with this CLI"
        );
        assert_eq!(
            error["suggested_action"],
            "Run `wayexpand upgrade --check`; restart or upgrade the daemon only after reviewing the reported process and compatibility details"
        );
    }

    #[test]
    fn daemon_readiness_failures_map_to_their_own_diagnostic_domains() {
        let cases = [
            (
                "configuration_unhealthy",
                "WX-CONFIG-001",
                "configuration",
                "The daemon rejected or has not loaded the active configuration",
            ),
            (
                "paused",
                "WX-INPUT-001",
                "matching",
                "Expansion matching is paused",
            ),
            (
                "replacement_not_admitted",
                "WX-POLICY-001",
                "replacement-policy",
                "Automatic replacement is refused by the active safety policy",
            ),
        ];

        for (reason, expected_id, expected_component, expected_message) in cases {
            let snapshot = serde_json::json!({
                "matching_available": false,
                "matching_unavailable_reason": reason,
                "output_health": "ok",
                "state": "connected",
                "status_schema": crate::CONTROL_STATUS_SCHEMA,
                "config_enabled_expansions": 2
            });
            let (report, state, exit_code, reported_reason, matching) = health_report(
                std::path::Path::new("/tmp/wayexpand/expansions.toml"),
                true,
                Some(&snapshot),
            );
            let error = &report["errors"][0];

            assert_eq!(state, "degraded", "reason: {reason}");
            assert_eq!(exit_code, 1, "reason: {reason}");
            assert_eq!(reported_reason, reason, "reason: {reason}");
            assert!(!matching, "reason: {reason}");
            assert_eq!(error["id"], expected_id, "reason: {reason}");
            assert_eq!(error["component"], expected_component, "reason: {reason}");
            assert_eq!(error["message"], expected_message, "reason: {reason}");
            assert!(error["suggested_action"].as_str().is_some());
        }
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
    }
}
