//! Complete machine-readable doctor report.

use super::backends::{
    automatic_selection_is_ready, backend_policy_allowed, capture_readiness,
    display_session_available,
};
use super::broker::broker_diagnostics;
use super::files::existing_control_socket_is_healthy;
use super::policy::{
    load_policy, print_capabilities_diagnostics_json, print_policy_diagnostics_json,
};
use super::status::{
    insertion_from_status, output_health_from_status, read_daemon_status,
    session_capabilities_from_status, status_as_json, status_schema_compatible,
};
use crate::*;

pub(crate) const GNOME_APP_FILTER_SUPPORT_DETAIL: &str = "Focused-window tracking is provided by the KWin bridge on KDE Plasma and by the WayExpand GNOME Shell extension, which is runtime-gated on GNOME Shell 46, 50, and 51; filtered snippets fail closed when tracking is unavailable.";

/// Stable, automation-friendly diagnostic output for service managers and
/// fleet health checks. Probes are bounded and non-interactive; portal
/// authorization and other state-changing checks are deliberately excluded.
pub(crate) fn print_json_diagnostics(path: &Path) -> Result<bool> {
    let config_result = Config::load(path);
    let config_ok = config_result.is_ok();
    let broker = broker_diagnostics(config_result.as_ref().ok());
    let socket_path = std::env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        });
    let socket_exists = socket_path.as_ref().is_some_and(|socket| socket.exists());
    let socket_valid = socket_path
        .as_deref()
        .is_none_or(existing_control_socket_is_healthy);
    let policy_result = load_policy();
    let policy = match &policy_result {
        Ok(policy) => policy.clone(),
        Err(_) => OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["none".into()],
            ..OrganizationPolicy::default()
        },
    };
    let backends: Vec<_> = discover_backends()
        .into_iter()
        .map(|status| {
            serde_json::json!({
                "kind": status.kind.to_string(),
                "state": format!("{:?}", status.state),
                "implementation": status.implementation(),
                "availability": status.availability(),
                "permission": status.permission(),
                "policy_allowed": backend_policy_allowed(status.kind, &policy),
                "detail": status.detail,
            })
        })
        .collect();
    let ibus_installed = ibus_engine_available();
    let policy_json = print_policy_diagnostics_json(&policy_result);
    let capabilities = print_capabilities_diagnostics_json();
    let live_capabilities = probe_capabilities();
    let wayland_session = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var_os("WAYLAND_SOCKET").is_some();
    let protocol_probes = protocol_probe_report(&live_capabilities, wayland_session);
    let (capture_state, capture_detail) =
        capture_readiness(&live_capabilities, ibus_installed, &policy);
    let recommendation =
        recommended_setup_backend_with_ibus(&live_capabilities, ibus_installed, &policy);
    let route_plan =
        wayexpand_backend_selection::plan_routes(&live_capabilities, ibus_installed, |route| {
            wayexpand_backend_selection::route_allowed_by_policy(&policy, route)
        });
    let ranked_routes: Vec<_> = route_plan.routes.iter().map(route_plan_entry).collect();
    let setup_recommendation = serde_json::json!({
        "mode": if recommendation.backend == "unavailable" { "none" } else if recommendation.backend == "evdev" { "maximum" } else { "recommended" },
        "backend": recommendation.backend,
        "label": recommendation.label,
        "detail": recommendation.detail,
        "ready": recommendation.backend != "unavailable",
    });
    let automatic_selection = wayexpand_backend_selection::auto_select(None, None)
        .map(|selection| {
            let policy_allowed = policy.backend_allowed(wayexpand_core::policy_backend_name(
                selection.pair.source(),
                selection.pair.backend(),
            ));
            serde_json::json!({
                "source": selection.pair.source(),
                "backend": selection.pair.backend(),
                "reason": selection.reason,
                "policy_allowed": policy_allowed,
                "ready": automatic_selection_is_ready(
                    selection.pair.source(),
                    selection.pair.backend(),
                    &policy,
                ),
            })
        })
        .unwrap_or_else(|error| {
            serde_json::json!({
                "source": serde_json::Value::Null,
                "backend": serde_json::Value::Null,
                "reason": error.to_string(),
                "policy_allowed": false,
                "ready": false,
            })
        });
    let selection_ok = automatic_selection["reason"].is_string()
        && automatic_selection["source"].is_string()
        && automatic_selection["ready"].as_bool().unwrap_or(false);
    let policy_ok = policy_json
        .get("policy")
        .and_then(|policy| policy.get("valid"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let services = crate::services::ServiceInventory::probe();
    let daemon_status_snapshot = read_daemon_status()
        .ok()
        .and_then(|response| status_as_json(&response).ok());
    let daemon = match read_daemon_status() {
        Ok(response) => match status_as_json(&response) {
            Ok(snapshot) => {
                let daemon_commit = snapshot
                    .get("daemon_commit")
                    .and_then(serde_json::Value::as_str);
                let daemon_version = snapshot
                    .get("daemon_version")
                    .and_then(serde_json::Value::as_str);
                serde_json::json!({
                    "available": true,
                    "compatible": status_schema_compatible(&snapshot),
                    "commit_matches": crate::doctor::status::daemon_commit_matches(daemon_commit),
                    "build_matches": crate::doctor::status::daemon_build_matches(daemon_commit, daemon_version),
                    "daemon_commit": daemon_commit,
                    "daemon_version": daemon_version,
                    "command_containment": snapshot.get("command_containment"),
                    "command_containment_mode": snapshot.get("command_containment_mode"),
                    "status_schema": snapshot.get("status_schema"),
                    "state": snapshot.get("state"),
                    "source": snapshot.get("source"),
                    "backend": snapshot.get("backend"),
                    "output": output_health_from_status(&snapshot, &services),
                    "insertion": insertion_from_status(&snapshot),
                    "session_capabilities": session_capabilities_from_status(&snapshot, &services),
                    "capabilities": {
                        "exclusive_capture": snapshot.get("capture_exclusive"),
                        "sensitive_field_detection": snapshot.get("capture_sensitive_focus"),
                        "composition_awareness": snapshot.get("capture_composition_aware"),
                        "local_compose_tracking": snapshot.get("capture_local_compose_aware"),
                        "layout_awareness": snapshot.get("capture_layout_aware"),
                        "layout_state": snapshot.get("capture_layout_state"),
                        "key_passthrough": snapshot.get("capture_key_passthrough")
                            .and_then(|capture| {
                                snapshot.get("inject_key_passthrough").map(|inject| {
                                    capture.as_bool().unwrap_or(false)
                                        && inject.as_bool().unwrap_or(false)
                                })
                            }),
                        "exact_window_identity": snapshot.get("window_identity_exact"),
                        "atomic_replacement": snapshot.get("inject_atomic_replace"),
                        "full_unicode": snapshot.get("inject_full_unicode"),
                    },
                })
            }
            Err(error) => serde_json::json!({
                "available": true,
                "compatible": false,
                "commit_matches": false,
                "error": error.to_string(),
                "session_capabilities": session_capabilities_from_status(&serde_json::Value::Null, &services),
            }),
        },
        Err(error) => serde_json::json!({
            "available": false,
            "compatible": false,
            "commit_matches": false,
            "error": error.to_string(),
            "session_capabilities": session_capabilities_from_status(&serde_json::Value::Null, &services),
        }),
    };
    let production_readiness = production_readiness(&daemon);
    let cleanup_degraded = daemon["command_cleanup_degraded"]
        .as_bool()
        .unwrap_or(false);
    let command_cleanup = serde_json::json!({
        "pending_jobs": daemon.get("command_cleanup_pending_jobs"),
        "failed_attempts": daemon.get("command_cleanup_failed_attempts"),
        "degraded": cleanup_degraded,
        "detail": if cleanup_degraded {
            "Command execution is refused until stale cgroup cleanup is resolved and the daemon is restarted."
        } else {
            "Cgroup cleanup reaper is within its configured backlog limit."
        },
    });
    let ibus_route = crate::commands::ibus::report_json(
        &wayexpand_backend_ibus::chain::diagnose(),
        &crate::commands::ibus::RouteOverlap::from_status(daemon_status_snapshot.as_ref()),
    );
    // `healthy` is consumed by monitoring and fleet automation. If IBus is
    // the selected desktop route, a diagnosed route failure must affect the
    // exit status too; otherwise a browser can receive no input while doctor
    // still reports a healthy machine. Unidentified field-level evidence
    // remains in the route report and does not by itself fail host health;
    // a stable route check marked unknown must fail closed.
    let selected_ibus_has_problem = selected_ibus_has_problem(&ibus_route);
    let healthy = config_ok
        && policy_ok
        && display_session_available()
        && (selection_ok || setup_recommendation["ready"].as_bool().unwrap_or(false))
        && socket_valid
        && broker["healthy"].as_bool().unwrap_or(false)
        && daemon["output"]["health"].as_str() != Some("stalled")
        && daemon["insertion"]["assurance"].as_str() != Some("blocked")
        && !selected_ibus_has_problem
        && !cleanup_degraded;
    println!(
        "{}",
        serde_json::json!({
            "wayexpand_version": build_info::VERSION,
            "wayexpand_commit": build_info::COMMIT,
            "desktop": live_capabilities.compositor.name(),
            "healthy": healthy,
            "wayland": std::env::var_os("WAYLAND_DISPLAY").is_some(),
            "config": {
                "path": path,
                "valid": config_ok,
                "error": config_result.err().map(|error| error.safe_summary()),
            },
            "control_socket": {
                "path": socket_path,
                "configured": socket_path.is_some(),
                "exists": socket_exists,
                "valid": socket_valid,
            },
            "daemon": daemon,
            "command_cleanup": command_cleanup,
            "production_readiness": production_readiness,
            "action_broker": broker,
            "ibus": {
                "installed": ibus_installed,
                "status": if ibus_installed { "available to configure" } else { "not installed" },
                "route": ibus_route,
            },
            "gnome_window_tracker": crate::gnome_extension::doctor_report(),
            "feature_support": {
                "ime_preedit": {
                    "status": "unsupported",
                    "detail": "WayExpand does not observe another IME's active preedit. Finish the composition and wait for committed text before typing a trigger. Local dead-key/Compose tracking is route-specific and does not provide external IME awareness.",
                    "remediation": "Commit or cancel the active composition, then type the trigger. For expansion during composition, use the IME's own phrase feature; composition-aware WayExpand integration is not available yet.",
                    "committed_unicode": "Committed Unicode can be matched and inserted only when the selected capture and output route support that text and the target permits safe replacement. This does not imply active-preedit support or universal application coverage.",
                },
                "app_filter": {
                    "status": "kwin_or_gnome_extension",
                    "detail": GNOME_APP_FILTER_SUPPORT_DETAIL,
                },
            },
            "policy": policy_json,
            "backends": backends,
            "protocol_probes": protocol_probes,
            "capabilities": capabilities,
            "automatic_selection": automatic_selection,
            "setup_recommendation": setup_recommendation,
            "route_plan": {
                "selection_basis": "current capabilities, organization policy, and catalog certification status",
                "routes": ranked_routes,
            },
            "capture_readiness": {
                "state": capture_state,
                "detail": capture_detail,
                "end_to_end_verified": false,
            },
        })
    );
    Ok(healthy)
}

fn route_plan_entry(planned: &wayexpand_backend_selection::PlannedRoute) -> serde_json::Value {
    let contract = planned.contract;
    serde_json::json!({
        "id": contract.id,
        "label": contract.label,
        "standing": planned.standing.as_str(),
        "reason": planned.reason,
        "capture": contract.capture,
        "injection": contract.injection,
        "certification": contract.status,
        "trigger_precondition": contract.trigger_precondition,
        "output_transaction": contract.output_transaction,
        "application_result": contract.application_result,
        "atomic_replace": contract.atomic_replace,
        "sensitive_field_protection": contract.sensitive_field_support.certified,
        "assessment": &planned.assessment,
    })
}

pub(crate) fn selected_ibus_has_problem(route: &serde_json::Value) -> bool {
    let Some(checks) = route["checks"].as_array() else {
        return false;
    };
    let selected = checks
        .iter()
        .any(|check| check["id"] == "selected" && check["state"] == "pass");
    selected
        && checks.iter().any(|check| {
            matches!(check["state"].as_str(), Some("fail" | "warn" | "unknown"))
                && check["problem_id"].as_str().is_some()
        })
}

/// Report live protocol checks separately from environment-level backend
/// discovery. A backend can be implemented while its compositor protocol is
/// unavailable in this session.
pub(crate) fn protocol_probe_report(
    capabilities: &wayexpand_backend_selection::Capabilities,
    wayland: bool,
) -> serde_json::Value {
    let wayland_probe = |available: bool| {
        if !wayland {
            "not_applicable"
        } else if available {
            "available"
        } else {
            "unavailable"
        }
    };
    let window_tracker = if !wayland {
        "not_applicable"
    } else if capabilities.has_window_tracker {
        "available"
    } else if capabilities.compositor == wayexpand_backend_selection::Compositor::KdePlasma {
        "unavailable"
    } else {
        "not_applicable"
    };
    serde_json::json!({
        "input_method_v2": {
            "status": wayland_probe(capabilities.has_input_method_v2),
            "detail": "input-method-v2 manager and Wayland seat probe",
        },
        "wlroots_virtual_keyboard": {
            "status": wayland_probe(capabilities.has_virtual_keyboard),
            "detail": "zwp_virtual_keyboard_manager_v1 probe",
        },
        "kwin_window_tracker": {
            "status": window_tracker,
            "detail": "KWin scripting D-Bus probe",
        },
    })
}

/// Derive the production-safety summary from the daemon probe without
/// treating connectivity or negotiated capabilities as certification.
pub(crate) fn production_readiness(daemon: &serde_json::Value) -> serde_json::Value {
    let daemon_capabilities = daemon
        .get("capabilities")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let capability = |name: &str| {
        daemon_capabilities
            .get(name)
            .and_then(serde_json::Value::as_bool)
    };
    let live_route = daemon
        .get("available")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        && daemon
            .get("compatible")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        && daemon
            .get("commit_matches")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        && daemon.get("state").and_then(serde_json::Value::as_str) == Some("connected");
    let mut readiness_blockers = Vec::new();
    if !live_route {
        readiness_blockers.push("no compatible, current, connected daemon route");
    }
    if daemon["output"]["health"].as_str() == Some("stalled") {
        readiness_blockers.push("stalled output worker blocks output reconnection");
    }
    if daemon["insertion"]["assurance"].as_str() == Some("blocked") {
        readiness_blockers
            .push("typed expansions are refused: keystroke route without compatibility consent");
    }
    for (name, label) in [
        ("exclusive_capture", "exclusive keyboard capture"),
        ("sensitive_field_detection", "sensitive-field detection"),
        ("composition_awareness", "external IME/preedit awareness"),
        ("layout_awareness", "runtime layout awareness"),
        ("key_passthrough", "key pass-through"),
        ("atomic_replacement", "atomic replacement"),
        ("full_unicode", "full Unicode injection"),
        ("exact_window_identity", "exact window identity"),
    ] {
        if capability(name) != Some(true) {
            readiness_blockers.push(label);
        }
    }
    // A live capability probe is never equivalent to reviewed real-client
    // evidence. Keep this blocker even when a future route satisfies every
    // negotiated capability above.
    readiness_blockers.push("reviewed compositor/client certification evidence");
    let readiness_status = if readiness_blockers.len() == 1 {
        "uncertified"
    } else if live_route {
        "limited"
    } else {
        "unavailable"
    };
    serde_json::json!({
        "status": readiness_status,
        "certified": false,
        "live_route": live_route,
        "capabilities": daemon_capabilities,
        "blockers": readiness_blockers,
    })
}

#[cfg(test)]
mod tests {
    use super::{route_plan_entry, selected_ibus_has_problem};
    use serde_json::json;

    #[test]
    fn doctor_route_contract_does_not_call_signal_emission_application_verification() {
        let plan = wayexpand_backend_selection::plan_routes(
            &wayexpand_backend_selection::Capabilities::default(),
            true,
            |_| true,
        );
        let ibus = plan
            .routes
            .iter()
            .find(|route| route.contract.id == "ibus")
            .expect("IBus route is present in the shared catalog");
        let report = route_plan_entry(ibus);

        assert_eq!(
            report["trigger_precondition"],
            "surrounding_text_matches_trigger"
        );
        assert_eq!(report["output_transaction"], "separate_delete_and_commit");
        assert_eq!(report["application_result"], "unobservable");
        assert_eq!(report["atomic_replace"], false);
        assert!(report.get("replacement_assurance").is_none());
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/contracts/cli-json-shapes.json"
        ))
        .unwrap();
        let expected = contract["doctor_route"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field.as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        let actual = report
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn selected_ibus_route_problem_blocks_overall_health() {
        let report = json!({
            "checks": [
                { "id": "selected", "state": "pass" },
                {
                    "id": "desktop-shell-environment",
                    "state": "warn",
                    "problem_id": "WX-IBUS-TOOLKIT-001"
                }
            ]
        });
        assert!(selected_ibus_has_problem(&report));
    }

    #[test]
    fn unknown_selected_ibus_route_with_a_stable_id_fails_closed() {
        let report = json!({
            "checks": [
                { "id": "selected", "state": "pass" },
                {
                    "id": "gnome-source-active",
                    "state": "unknown",
                    "problem_id": "WX-GNOME-IM-002"
                }
            ]
        });
        assert!(selected_ibus_has_problem(&report));
    }

    #[test]
    fn selected_ibus_daemon_failure_blocks_overall_health() {
        let report = json!({
            "checks": [
                { "id": "selected", "state": "pass" },
                {
                    "id": "daemon",
                    "state": "fail",
                    "problem_id": "WX-IBUS-004"
                }
            ]
        });
        assert!(selected_ibus_has_problem(&report));
    }

    #[test]
    fn unrelated_or_unidentified_ibus_warnings_do_not_fail_health() {
        let unselected = json!({
            "checks": [
                { "id": "selected", "state": "fail" },
                {
                    "id": "desktop-shell-environment",
                    "state": "warn",
                    "problem_id": "WX-IBUS-TOOLKIT-001"
                }
            ]
        });
        let unknown_field_activity = json!({
            "checks": [
                { "id": "selected", "state": "pass" },
                { "id": "field-input", "state": "unknown" },
                { "id": "target-insertion", "state": "warn" }
            ]
        });
        assert!(!selected_ibus_has_problem(&unselected));
        assert!(!selected_ibus_has_problem(&unknown_field_activity));
    }
}
