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
use super::status::{read_daemon_status, status_as_json, status_schema_compatible};
use crate::*;

/// Stable, automation-friendly diagnostic output for service managers and
/// fleet health checks. It deliberately avoids compositor probes that can
/// block or mutate session state; those remain in the human doctor output.
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
    let (capture_state, capture_detail) =
        capture_readiness(&live_capabilities, ibus_installed, &policy);
    let recommendation = recommended_setup_backend(&live_capabilities, &policy);
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
    let daemon = match read_daemon_status() {
        Ok(response) => match status_as_json(&response) {
            Ok(snapshot) => {
                let daemon_commit = snapshot
                    .get("daemon_commit")
                    .and_then(serde_json::Value::as_str);
                serde_json::json!({
                    "available": true,
                    "compatible": status_schema_compatible(&snapshot),
                    "commit_matches": daemon_commit == Some(build_info::COMMIT),
                    "daemon_commit": daemon_commit,
                    "status_schema": snapshot.get("status_schema"),
                    "state": snapshot.get("state"),
                    "source": snapshot.get("source"),
                    "backend": snapshot.get("backend"),
                })
            }
            Err(error) => serde_json::json!({
                "available": true,
                "compatible": false,
                "commit_matches": false,
                "error": error.to_string(),
            }),
        },
        Err(error) => serde_json::json!({
            "available": false,
            "compatible": false,
            "commit_matches": false,
            "error": error.to_string(),
        }),
    };
    let healthy = config_ok
        && policy_ok
        && display_session_available()
        && (selection_ok || setup_recommendation["ready"].as_bool().unwrap_or(false))
        && socket_valid
        && broker["healthy"].as_bool().unwrap_or(false);
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
            "action_broker": broker,
            "ibus": {
                "installed": ibus_installed,
                "status": if ibus_installed { "available to configure" } else { "not installed" },
            },
            "feature_support": {
                "ime_preedit": {
                    "status": "unsupported",
                    "detail": "Only committed text is processed; finish IME, dead-key, or Compose composition before typing a trigger.",
                },
                "app_filter": {
                    "status": "kwin_only",
                    "detail": "Focused-window tracking is currently provided by the KWin bridge; filtered snippets fail closed when tracking is unavailable.",
                },
            },
            "policy": policy_json,
            "backends": backends,
            "capabilities": capabilities,
            "automatic_selection": automatic_selection,
            "setup_recommendation": setup_recommendation,
            "capture_readiness": {
                "state": capture_state,
                "detail": capture_detail,
                "end_to_end_verified": false,
            },
        })
    );
    Ok(healthy)
}
