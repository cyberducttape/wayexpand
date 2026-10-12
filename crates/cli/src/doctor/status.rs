//! Daemon status reading and its JSON representation.

use super::broker::broker_diagnostics;
use crate::*;

pub(crate) fn read_daemon_status() -> Result<String> {
    wayexpand_core::DaemonClient::from_environment()?
        .status()
        .map_err(Into::into)
}

pub(crate) fn status_as_json(response: &str) -> Result<serde_json::Value> {
    let mut object = serde_json::Map::new();
    let status = wayexpand_core::DaemonStatus::parse(response);
    if !status.response().is_empty() {
        object.insert("response".into(), status.response().into());
    }
    for (key, value) in status.fields() {
        let value = match value {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            value
                if matches!(
                    key,
                    "command_queue_depth"
                        | "command_in_flight"
                        | "expansion_command_queue_depth"
                        | "expansion_command_in_flight"
                        | "hotkey_queue_depth"
                        | "hotkey_in_flight"
                        | "command_queue_rejected_total"
                        | "command_timeout_total"
                        | "command_failure_total"
                        | "status_schema"
                        | "inject_max_text_chars"
                        | "inject_expected_throughput_chars_per_sec"
                        | "injection_latency_sample_count"
                        | "injection_latency_window_count"
                        | "injection_latency_p50_us"
                        | "injection_latency_p95_us"
                        | "injection_latency_p99_us"
                        | "matcher_latency_sample_count"
                        | "matcher_latency_window_count"
                        | "matcher_latency_p50_us"
                        | "matcher_latency_p95_us"
                        | "matcher_latency_p99_us"
                        | "usage_flush_max_duration_us"
                        | "usage_flush_failures_total"
                        | "usage_queue_rejected_total"
                        | "output_uncertain_total"
                        | "output_retiring_workers"
                        | "output_stalled_ms"
                        | "output_refused_total"
                ) =>
            {
                match value.parse::<u64>() {
                    Ok(number) => serde_json::Value::Number(number.into()),
                    Err(_) => value.into(),
                }
            }
            "injection_latency_profiles" => serde_json::from_str(value)
                .unwrap_or_else(|_| serde_json::Value::String(value.into())),
            _ => value.into(),
        };
        object.insert(key.to_owned(), value);
    }
    object.insert(
        "action_broker".into(),
        broker_diagnostics(Config::load(default_config_path()).ok().as_ref()),
    );
    Ok(serde_json::Value::Object(object))
}

pub(crate) fn runtime_capabilities_from_status(snapshot: &serde_json::Value) -> serde_json::Value {
    let value = |key: &str| {
        snapshot
            .get(key)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    serde_json::json!({
        "capture": {
            "sensitive_focus": value("capture_sensitive_focus"),
            "exclusive": value("capture_exclusive"),
            "reliable_key_state": value("capture_reliable_key_state"),
            "key_passthrough": value("capture_key_passthrough"),
            "composition_aware": value("capture_composition_aware"),
            "local_compose_aware": value("capture_local_compose_aware"),
            "layout_aware": value("capture_layout_aware"),
        },
        "window_context": {
            "tracker_connected": value("window_tracker_connected"),
        },
        "injection": {
            "insertion_mode": value("inject_insertion_mode"),
            "max_text_chars": value("inject_max_text_chars"),
            "expected_throughput_chars_per_sec": value("inject_expected_throughput_chars_per_sec"),
            "atomic_replace": value("inject_atomic_replace"),
            "full_unicode": value("inject_full_unicode"),
            "cursor_reposition": value("inject_cursor_reposition"),
            "key_passthrough": value("inject_key_passthrough"),
        },
    })
}

/// Output-route health from a daemon status snapshot, with recovery
/// guidance when a retired output worker blocks reconnection.
pub(crate) fn output_health_from_status(snapshot: &serde_json::Value) -> serde_json::Value {
    let health = snapshot
        .get("output_health")
        .and_then(serde_json::Value::as_str);
    let unit = match snapshot.get("source").and_then(serde_json::Value::as_str) {
        Some("evdev") => "wayexpand-evdev.service",
        Some("input-method") => "wayexpand-input-method.service",
        _ => "wayexpand.service",
    };
    let recovery = (health == Some("stalled")).then(|| {
        format!(
            "a retired output worker is still inside a compositor or portal call, so output \
             will not reconnect (a late operation could otherwise type into another window); \
             restart the daemon with `systemctl --user restart {unit}`"
        )
    });
    serde_json::json!({
        "health": health,
        "uncertain_transactions_total": snapshot.get("output_uncertain_total"),
        "retiring_workers": snapshot.get("output_retiring_workers"),
        "stalled_ms": snapshot.get("output_stalled_ms"),
        "recovery": recovery,
    })
}

/// How the daemon's output route is assured, with a plain explanation of
/// what that means for the user's text.
pub(crate) fn insertion_from_status(snapshot: &serde_json::Value) -> serde_json::Value {
    let text = |key: &str| snapshot.get(key).and_then(serde_json::Value::as_str);
    let assurance = text("insertion_assurance");
    let mode = text("compatibility_mode").unwrap_or("off");
    let focus = text("focus_assurance");
    let explanation = match assurance {
        Some("verified") => Some(
            "verified: the input method checks the field's own text before replacing a \
             trigger, and the edit is bound to that field"
                .to_owned(),
        ),
        Some("compatibility") => Some(format!(
            "compatibility mode ({mode}): keystrokes erase the trigger without seeing the \
             field's text; {}",
            if focus == Some("tracker") {
                "the window tracker confirms focus immediately before typing"
            } else {
                "focus cannot be confirmed right now, so the text may land in whatever has focus"
            }
        )),
        Some("blocked") => Some(
            "blocked: this keystroke route cannot verify what it erases, so typed expansions \
             are refused. Use a verified route (`wayexpand setup`), or opt in with \
             `compatibility_mode = \"tracked-focus\"` (requires the KDE window tracker) or \
             `\"untracked-focus\"` under [settings]"
                .to_owned(),
        ),
        Some("unavailable") => Some("no output route is connected".to_owned()),
        _ => None,
    };
    let last_refusal = text("output_last_refusal").filter(|reason| *reason != "none");
    serde_json::json!({
        "assurance": assurance,
        "compatibility_mode": text("compatibility_mode"),
        "focus": focus,
        "refused_total": snapshot.get("output_refused_total"),
        "last_refusal": last_refusal,
        "explanation": explanation,
    })
}

/// Print the running daemon's output health. Returns false when the daemon
/// reports a stalled output route or refuses all typed expansions; an
/// unreachable daemon is not a failure here (doctor also runs before the
/// service is started).
pub(crate) fn print_output_health_diagnostics() -> bool {
    let Some(snapshot) = read_daemon_status()
        .ok()
        .and_then(|response| status_as_json(&response).ok())
    else {
        return true;
    };
    let insertion = insertion_from_status(&snapshot);
    if let Some(explanation) = insertion["explanation"].as_str() {
        println!("Insertion: {explanation}");
    }
    if let Some(refused) = insertion["refused_total"]
        .as_u64()
        .filter(|total| *total > 0)
    {
        println!(
            "Insertion refusals: {refused} expansion(s) refused before typing (last: {}); \
             nothing was erased or inserted for them",
            insertion["last_refusal"].as_str().unwrap_or("unknown")
        );
    }
    let insertion_ok = insertion["assurance"].as_str() != Some("blocked");
    let output = output_health_from_status(&snapshot);
    let Some(health) = output["health"].as_str() else {
        return insertion_ok;
    };
    println!(
        "Output route: {health} ({} uncertain transaction(s) since start)",
        output["uncertain_transactions_total"].as_u64().unwrap_or(0)
    );
    if output["uncertain_transactions_total"].as_u64().unwrap_or(0) > 0 {
        println!(
            "Output warning: some expansions may have been partially typed; they were not \
             retried. Check recently edited text."
        );
    }
    if let Some(recovery) = output["recovery"].as_str() {
        println!(
            "Output problem: stalled for {}ms; {recovery}",
            output["stalled_ms"].as_u64().unwrap_or(0)
        );
        return false;
    }
    insertion_ok
}

pub(crate) fn status_schema_compatible(snapshot: &serde_json::Value) -> bool {
    snapshot
        .get("status_schema")
        .and_then(serde_json::Value::as_u64)
        == Some(u64::from(CONTROL_STATUS_SCHEMA))
}
