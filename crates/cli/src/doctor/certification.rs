//! Compositor certification scenarios and selection status.

use super::status::{
    read_daemon_status, runtime_capabilities_from_status, status_as_json, status_schema_compatible,
};
use crate::*;

pub(crate) fn print_certification(json: bool) -> Result<bool> {
    let required_scenarios = certification_scenarios()?;
    let capabilities = probe_capabilities();
    let policy_result = wayexpand_core::load_organization_policy();
    let policy_allows = |backend: &str| {
        policy_result
            .as_ref()
            .is_ok_and(|policy| policy.backend_allowed(backend))
    };
    let policy_allows_ibus = policy_allows("input-method-v2");
    let selection = wayexpand_backend_selection::auto_select(None, None)
        .ok()
        .filter(|selection| {
            policy_result.as_ref().is_ok_and(|policy| {
                policy.backend_allowed(wayexpand_core::policy_backend_name(
                    selection.pair.source(),
                    selection.pair.backend(),
                ))
            })
        });
    let ibus_installed = ibus_engine_available();
    let recommendation = recommended_route(&capabilities, ibus_installed, |route| {
        policy_result
            .as_ref()
            .is_ok_and(|policy| route_allowed_by_policy(policy, route))
    });
    let ibus = recommendation.is_some_and(|route| route.id() == "ibus");
    let selected_label = match recommendation {
        Some(route) => route.setup_backend(),
        None => selection
            .as_ref()
            .map(|selection| selection.pair.source())
            .unwrap_or("none"),
    };
    let mut checks = Vec::new();
    let mut add_check = |category: &str, name: &str, status: &str, detail: &str| {
        checks.push(serde_json::json!({
            "category": category,
            "name": name,
            "status": status,
            "detail": detail,
        }));
    };

    let certification_config = env::var_os("WAYEXPAND_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    match Config::load(&certification_config) {
        Ok(_) => add_check(
            "configuration",
            "active configuration",
            "verified",
            "the configured expansion file parses and passes security validation",
        ),
        Err(error) => add_check(
            "configuration",
            "active configuration",
            "failed",
            &format!("configuration is not usable: {}", error.safe_summary()),
        ),
    }
    match &policy_result {
        Ok(_) => add_check(
            "policy",
            "organization policy",
            "verified",
            "the shared secure organization-policy loader accepted the policy state",
        ),
        Err(error) => add_check(
            "policy",
            "organization policy",
            "failed",
            &format!("organization policy blocks startup: {error}"),
        ),
    }

    if capabilities.compositor != wayexpand_backend_selection::Compositor::Unknown {
        add_check(
            "environment",
            "desktop identified",
            "verified",
            capabilities.compositor.name(),
        );
    } else {
        add_check(
            "environment",
            "desktop identified",
            "unknown",
            "XDG_CURRENT_DESKTOP is not a recognized compositor",
        );
    }
    if ibus {
        add_check(
            "input-path",
            "IBus engine installed",
            "available",
            "IBus is a candidate for Recommended mode",
        );
    } else if ibus_installed && !policy_allows_ibus {
        add_check(
            "input-path",
            "IBus engine installed",
            "unsupported",
            "IBus is installed but input-method-v2 is disallowed by organization policy",
        );
    } else {
        add_check(
            "input-path",
            "IBus engine installed",
            "unsupported",
            "the WayExpand IBus component is not installed or discoverable",
        );
    }
    if capabilities.has_input_method_v2 && policy_allows("input-method-v2") {
        add_check(
            "input-path",
            "input-method-v2 protocol",
            "available",
            "protocol manager and seat probe succeeded; live key pass-through remains untested",
        );
    } else if capabilities.has_input_method_v2 {
        add_check(
            "input-path",
            "input-method-v2 protocol",
            "unsupported",
            "the compositor exposed the protocol, but organization policy disallows this backend",
        );
    } else {
        add_check(
            "input-path",
            "input-method-v2 protocol",
            "unsupported",
            "the compositor did not expose a usable input-method-v2 interface",
        );
    }
    if capabilities.has_virtual_keyboard && policy_allows("wlroots") {
        add_check(
            "output-path",
            "wlroots virtual keyboard",
            "available",
            "virtual keyboard globals were found; end-to-end insertion remains untested",
        );
    } else if capabilities.has_virtual_keyboard {
        add_check(
            "output-path",
            "wlroots virtual keyboard",
            "unsupported",
            "the compositor exposed the protocol, but organization policy disallows this backend",
        );
    } else {
        add_check(
            "output-path",
            "wlroots virtual keyboard",
            "unsupported",
            "the compositor did not expose zwp_virtual_keyboard_v1",
        );
    }
    if capabilities.has_direct_libei_socket && policy_allows("libei") {
        add_check(
            "output-path",
            "libei/EIS transport",
            "available",
            "an explicit LIBEI_SOCKET is present; portal authorization was not re-requested",
        );
    } else if capabilities.has_direct_libei_socket {
        add_check(
            "output-path",
            "libei/EIS transport",
            "unsupported",
            "an EIS socket was detected, but organization policy disallows this backend",
        );
    } else {
        add_check("output-path", "libei/EIS transport", "authorization-required", "portal probing is intentionally non-interactive; run the selected mode to authorize it");
    }

    let selected_capture = if ibus { "ibus" } else { selected_label };
    let selection_status = certification_selection_status(selected_capture);
    let selection_detail = if selected_capture == "stdin" {
        "automatic selection is the conservative stdin-only fallback; no keyboard capture path is configured".to_owned()
    } else {
        format!("automatic selection currently resolves to {selected_capture}")
    };
    add_check(
        "selection",
        "automatic mode selection",
        selection_status,
        &selection_detail,
    );

    // Certification is often run after a user has explicitly enabled a
    // backend. Report the live daemon route separately from automatic
    // selection so a connected evdev/libei service is not presented as if the
    // machine were currently stdin-only. A status response is observational;
    // it never changes the certification result or promotes a probe to proof.
    let active_daemon = match read_daemon_status() {
        Ok(response) => {
            let snapshot = status_as_json(&response)?;
            let state = snapshot
                .get("state")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let source = snapshot.get("source").and_then(serde_json::Value::as_str);
            let backend = snapshot.get("backend").and_then(serde_json::Value::as_str);
            let connected = state == "connected" && source.is_some() && backend.is_some();
            let status_schema = snapshot
                .get("status_schema")
                .and_then(serde_json::Value::as_u64);
            let compatible = status_schema_compatible(&snapshot);
            let runtime_capabilities = runtime_capabilities_from_status(&snapshot);
            let detail = match (source, backend, compatible) {
                (Some(source), Some(backend), true) => {
                    format!("state={state}; compatible active route is {source} + {backend}")
                }
                (Some(source), Some(backend), false) => format!(
                    "route {source} + {backend} is connected, but daemon status schema is {:?}; expected {}. Restart/update the daemon before relying on these diagnostics",
                    status_schema,
                    CONTROL_STATUS_SCHEMA
                ),
                _ => format!("daemon returned state={state}, but no complete active route"),
            };
            add_check(
                "runtime",
                "active daemon route",
                if connected && compatible {
                    "verified"
                } else {
                    "unavailable"
                },
                &detail,
            );
            serde_json::json!({
                "connected": connected,
                "compatible": compatible,
                "status_schema": status_schema,
                "state": state,
                "source": source,
                "backend": backend,
                "capabilities": runtime_capabilities,
            })
        }
        Err(error) => {
            add_check(
                "runtime",
                "active daemon route",
                "unavailable",
                &format!("could not query the daemon: {error}"),
            );
            serde_json::json!({
                "connected": false,
                "state": "unavailable",
                "source": null,
                "backend": null,
                "capabilities": null,
            })
        }
    };

    // These checks intentionally remain NOT RUN until a compositor-specific
    // harness drives real GTK/Qt/Wayland clients. A preflight must never turn
    // protocol availability into a false CERTIFIED claim.
    for name in &required_scenarios {
        let category = certification_scenario_category(name);
        add_check(
            category,
            name,
            "not-run",
            "requires the compositor certification harness; no claim is made from a static probe",
        );
    }

    let certified = checks
        .iter()
        .all(|check| check["status"] == "verified" || check["status"] == "available")
        && !checks.is_empty();
    let limitations = if ibus {
        vec![
            "GTK and Qt client behavior still requires live certification",
            "IME/preedit composition is not supported",
            "surrounding-text behavior depends on the client toolkit",
        ]
    } else if selected_capture == "stdin" {
        if active_daemon["connected"] == true {
            if active_daemon["compatible"] == true {
                vec![
                    "a manually configured daemon route is connected; automatic selection remains stdin-only",
                    "desktop client behavior still requires certification scenarios",
                ]
            } else {
                vec![
                    "the running daemon status contract is missing or incompatible; restart/update it before relying on runtime diagnostics",
                    "automatic selection remains stdin-only and desktop client behavior still requires certification scenarios",
                ]
            }
        } else {
            vec![
                "no automatic keyboard input path is selected",
                "text expansion is available only through the stdin test harness",
            ]
        }
    } else {
        wayexpand_core::all_capabilities()
            .into_iter()
            .filter(|caps| caps.backend_name == selected_capture)
            .flat_map(|caps| caps.limitations.iter().copied())
            .collect::<Vec<_>>()
    };
    let report = serde_json::json!({
        "schema": 1,
        "wayexpand_version": build_info::VERSION,
        "wayexpand_commit": build_info::COMMIT,
        "certified": certified,
        "desktop": capabilities.compositor.name(),
        "config_path": certification_config,
        "selected_mode": selected_capture,
        "active_daemon": active_daemon,
        "required_scenarios": required_scenarios,
        "checks": checks,
        "limitations": limitations,
    });
    if json {
        println!("{report}");
    } else {
        println!("WayExpand Desktop Certification");
        println!(
            "  WayExpand: {} (commit {})",
            build_info::VERSION,
            build_info::COMMIT
        );
        println!("  Desktop: {}", report["desktop"]);
        println!("  Selected mode: {}", report["selected_mode"]);
        println!("  Active daemon: {}", report["active_daemon"]);
        for check in report["checks"].as_array().into_iter().flatten() {
            println!(
                "  [{:18}] {:32} {}",
                check["status"].as_str().unwrap_or("unknown"),
                check["name"].as_str().unwrap_or("unknown"),
                check["detail"].as_str().unwrap_or("")
            );
        }
        println!(
            "\nResult: {}",
            if certified {
                "CERTIFIED"
            } else {
                "NOT CERTIFIED"
            }
        );
        println!("Run the compositor harness before treating this record as a support claim.");
    }
    Ok(certified)
}

pub(crate) fn certification_selection_status(selected_capture: &str) -> &'static str {
    match selected_capture {
        "none" => "failed",
        "stdin" => "unsupported",
        _ => "available",
    }
}

pub(crate) fn certification_scenarios() -> Result<Vec<String>> {
    let matrix: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/certification/compositor-matrix.json"
    ))
    .context("checked-in compositor certification matrix is invalid")?;
    matrix["required_scenarios"]
        .as_array()
        .context("certification matrix has no required_scenarios array")?
        .iter()
        .map(|scenario| {
            scenario
                .as_str()
                .map(str::to_owned)
                .context("certification matrix contains a non-string scenario")
        })
        .collect()
}

pub(crate) fn certification_scenario_category(scenario: &str) -> &'static str {
    match scenario {
        "printable-press-release"
        | "held-keys-repeat"
        | "modifier-navigation"
        | "application-shortcuts"
        | "compositor-shortcuts"
        | "media-keys"
        | "held-modifier-unsupported-key"
        | "caps-lock"
        | "fast-typing" => "typing-integrity",
        "unicode-combining" | "multiline-rapid" => "text-integrity",
        "password-field"
        | "focus-cross-window"
        | "focus-change-during-expansion"
        | "target-closes-during-expansion" => "safety",
        "config-reload" | "daemon-restart" | "compositor-restart" | "failed-insertion" => {
            "recovery"
        }
        "dead-key-committed-text"
        | "compose-committed-text"
        | "expansion-after-committed-composition" => "input-method",
        _ => "other",
    }
}
