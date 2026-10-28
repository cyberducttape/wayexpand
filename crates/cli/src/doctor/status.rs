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
                        | "injection_latency_mean_us"
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
                        | "usage_worker_failures_total"
                        | "usage_events_recorded_total"
                        | "usage_events_persisted_total"
                        | "usage_events_dropped_total"
                        | "clipboard_worker_threads"
                        | "clipboard_reads_in_flight"
                        | "clipboard_reads_total"
                        | "clipboard_read_failures_total"
                        | "clipboard_deadline_misses_total"
                        | "clipboard_read_max_duration_us"
                        | "output_uncertain_total"
                        | "output_retiring_workers"
                        | "output_stalled_ms"
                        | "output_reconnect_total"
                        | "output_unsupported_total"
                        | "input_reconnect_total"
                        | "input_overflow_total"
                        | "control_rejected_total"
                        | "memory_rss_kib"
                        | "memory_high_water_kib"
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
            "recent_expansion_failures" => serde_json::from_str(value)
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
    let assurance = text("replacement_assurance").or_else(|| text("insertion_assurance"));
    let mode = text("compatibility_mode").unwrap_or("off");
    let focus = text("focus_assurance");
    let explanation = match assurance {
        Some("verified") => Some(
            "target-text verified: the input method checks the field's own text before replacing a \
             trigger, and the edit is bound to that field"
                .to_owned(),
        ),
        Some("focus-tracked-best-effort") => Some(
            "focus-tracked best effort: the tracker confirms the focused window, but the route \
             cannot verify that the text at the cursor still matches the trigger"
                .to_owned(),
        ),
        Some("unverified") => Some(
            "unverified replacement: this explicitly allowed keystroke route cannot verify the \
             target field, focus, or text it erases"
                .to_owned(),
        ),
        Some("compatibility") => Some(format!(
            "legacy compatibility status ({mode}): keystrokes erase the trigger without seeing \
             the field's text; {}",
            match focus {
                Some("tracker") =>
                    "a window tracker confirms focus, but text and cursor remain unchecked",
                Some("unconfirmed") => "focus cannot be confirmed",
                Some("protocol") => "the protocol binds output to the text field",
                _ => "focus assurance is unknown",
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
        "replacement_assurance": assurance,
        "compatibility_mode": text("compatibility_mode"),
        "focus": focus,
        "refused_total": snapshot.get("output_refused_total"),
        "last_refusal": last_refusal,
        "explanation": explanation,
    })
}

/// Explain the five operational capabilities users most need to understand.
/// These describe the daemon's current report and deliberately do not claim
/// real-client certification based on protocol discovery alone.
pub(crate) fn session_capabilities_from_status(snapshot: &serde_json::Value) -> serde_json::Value {
    let text = |key: &str| snapshot.get(key).and_then(serde_json::Value::as_str);
    let boolean = |key: &str| snapshot.get(key).and_then(serde_json::Value::as_bool);
    let connected = matches!(text("state"), Some("connected" | "running"));
    let source = text("source");
    let backend = text("backend");
    let source_name = source.unwrap_or("unknown source");
    let backend_name = backend.unwrap_or("unknown backend");
    let unavailable = !connected;

    let capture = if unavailable {
        capability(
            "unavailable",
            "No connected capture route is reported by the daemon.",
            "Physical keyboard capture is not confirmed in this session.",
            vec!["Start the user daemon with `systemctl --user start wayexpand.service`, then run `wayexpand doctor` again."],
        )
    } else if source == Some("stdin") {
        capability(
            "simulated",
            "The active source is stdin; this does not capture physical keyboard input.",
            "stdin is useful for tests and cannot establish desktop capture readiness.",
            vec!["Configure a supported input source with `wayexpand setup`, then inspect the new active route."],
        )
    } else if source.is_none() {
        capability(
            "unknown",
            "The daemon is connected but did not report its capture source.",
            "Capture behavior cannot be attributed to a route.",
            vec!["Check `wayexpand upgrade-status` and update the CLI and daemon together."],
        )
    } else {
        let exclusive = boolean("capture_exclusive");
        capability(
            match exclusive {
                Some(true) => "active-exclusive",
                Some(false) => "active-shared",
                None => "active-capability-unknown",
            },
            &format!("The daemon reports {source_name} capture connected."),
            &format!("Exclusive capture: {}. Key state reliable: {}. Layout aware: {}.", display_bool(exclusive), display_bool(boolean("capture_reliable_key_state")), display_bool(boolean("capture_layout_aware"))),
            vec!["Try a harmless test snippet in the application you use; this live status does not certify physical key delivery across clients."],
        )
    };

    let insertion_mode = text("inject_insertion_mode");
    let injection = if unavailable {
        capability(
            "unavailable",
            "No connected injection route is reported by the daemon.",
            "Text insertion cannot be confirmed in this session.",
            vec!["Start or reconnect the daemon with `systemctl --user restart wayexpand.service`, then run `wayexpand doctor` again."],
        )
    } else if let Some(mode) = insertion_mode {
        let full_unicode = boolean("inject_full_unicode");
        let summary = if mode.contains("keysym fallback") {
            format!("{backend_name} is inserting with {mode}; text is synthesized key by key and may be slow or limited for Unicode.")
        } else {
            format!("{backend_name} is inserting with {mode}.")
        };
        let steps = if mode.contains("keysym fallback") || full_unicode != Some(true) {
            vec!["Check portal authorization and `wayexpand backend`; use a direct-text insertion mode when the compositor offers one.", "Confirm exact output with a harmless ASCII and Unicode test in the target application."]
        } else {
            vec!["Confirm exact output in the target application with a harmless short snippet; negotiated support is not a real-client certification."]
        };
        capability(
            match full_unicode {
                Some(true) => "active",
                Some(false) => "active-limited",
                None => "active-capability-unknown",
            },
            &summary,
            &format!(
                "Full Unicode reported: {}. Atomic replace reported: {}. Insertion mode: {mode}.",
                display_bool(full_unicode),
                display_bool(boolean("inject_atomic_replace"))
            ),
            steps,
        )
    } else {
        capability(
            "unknown",
            "The daemon is connected but did not report its insertion mode.",
            "The route's text-insertion behavior is unknown to this doctor version.",
            vec!["Check `wayexpand upgrade-status` and update the CLI and daemon together."],
        )
    };

    let tracker = boolean("window_tracker_connected");
    let exact_identity = boolean("window_identity_exact");
    let focus_assurance = text("focus_assurance");
    let focus_tracking = if unavailable {
        capability(
            "unavailable",
            "Focus tracking cannot be assessed without a connected daemon route.",
            "No current focus observation is available.",
            vec!["Reconnect the daemon, then inspect focus tracking again."],
        )
    } else if focus_assurance == Some("protocol") {
        capability(
            "protocol-bound",
            "The active protocol binds output to the target text field.",
            "This is field-bound protocol behavior, not a window-tracker observation.",
            vec!["Use `wayexpand doctor --certify` for isolated route evidence and test the target client separately."],
        )
    } else if tracker == Some(true) && exact_identity == Some(true) {
        capability(
            "exact-window-tracked",
            "A connected tracker reports an exact focused-window identity.",
            "Window identity does not prove that the caret stayed beside the trigger.",
            vec!["Avoid clicking or switching focus while a delayed expansion is running; verify the result in your target client."],
        )
    } else if tracker == Some(true) {
        capability(
            "partial",
            "A tracker is connected, but exact window identity is not available.",
            "Application or title matching alone cannot distinguish every window.",
            vec!["Use a route that supplies exact window identity, or keep focus-tracked expansions disabled by policy."],
        )
    } else if tracker == Some(false) {
        capability(
            "unavailable",
            "No window-focus tracker is connected for this route.",
            "The input route may still have protocol-bound focus, but no independent window tracking is reported.",
            vec!["On KDE, start or repair the WayExpand KWin bridge; otherwise use a protocol-bound input-method route."],
        )
    } else {
        capability(
            "unknown",
            "The daemon did not report window-tracker state or protocol-bound focus assurance.",
            "Focus tracking must not be inferred from a connected capture or injection route.",
            vec!["Check `wayexpand upgrade-status` and update the CLI and daemon together."],
        )
    };

    let sensitive = boolean("capture_sensitive_focus");
    let password_protection = if unavailable {
        capability(
            "unknown",
            "Password-field protection cannot be assessed without an active capture route.",
            "No sensitive-focus capability report is available.",
            vec!["Reconnect the daemon and check `capture_sensitive_focus` in `wayexpand doctor --json`."],
        )
    } else if sensitive == Some(true) {
        capability(
            "signal-available-unverified",
            "The active input source reports sensitive-field awareness.",
            "The signal is available, but this status alone does not prove every target client exposes password fields correctly.",
            vec!["Treat this as an available signal, not a verified safety guarantee; check route-specific certification and never test with real credentials."],
        )
    } else if sensitive == Some(false) {
        capability(
            "unavailable",
            "The active input source cannot identify password or sensitive fields.",
            "Automatic expansion may observe and act on keystrokes in credential fields on this route.",
            vec!["Switch to input-method-v2 or an IBus route with sensitive-field signaling; otherwise disable global evdev capture when this risk is unacceptable."],
        )
    } else {
        capability(
            "unknown",
            "The daemon did not report sensitive-field awareness.",
            "Do not assume that password fields are protected.",
            vec!["Update CLI and daemon together, then inspect the active source's sensitive-focus capability."],
        )
    };

    let replacement = insertion_from_status(snapshot);
    let assurance = replacement["assurance"].as_str();
    let replacement_assurance = match assurance {
        Some("verified") => capability(
            "verified-protocol",
            replacement["explanation"].as_str().unwrap_or("Target text is verified by the active protocol."),
            "The protocol-level guarantee does not replace compositor and client certification.",
            vec!["Run a harmless replacement in each supported target application and review compositor certification evidence."],
        ),
        Some("focus-tracked-best-effort") => capability(
            "best-effort",
            replacement["explanation"].as_str().unwrap_or("Focus is tracked, but target text is not verified."),
            "The route cannot prove that the characters being erased still equal the trigger.",
            vec!["Use a verified or atomic route for higher assurance; otherwise enable only after accepting best-effort replacement."],
        ),
        Some("unverified") | Some("compatibility") => capability(
            "unverified",
            replacement["explanation"].as_str().unwrap_or("Replacement is not verified."),
            "The route cannot verify target text before erasing it.",
            vec!["Prefer a verified/atomic route; avoid enabling unverified replacement for important or destructive text."],
        ),
        Some("blocked") => capability(
            "blocked",
            replacement["explanation"].as_str().unwrap_or("Typed replacement is blocked by the current safety policy."),
            "No trigger text is erased when the route refuses an expansion.",
            vec!["Run `wayexpand setup` to select a safer route, or explicitly change compatibility policy only if best-effort behavior is acceptable."],
        ),
        Some("unavailable") => capability(
            "unavailable",
            "No injection route is currently connected.",
            "Replacement cannot run until output reconnects.",
            vec!["Restart the user daemon and inspect `wayexpand backend` for the failed route."],
        ),
        _ => capability(
            "unknown",
            "Replacement assurance was not reported by the daemon.",
            "Do not infer atomicity from backend availability.",
            vec!["Update the CLI and daemon together, then inspect `wayexpand doctor --json`."],
        ),
    };

    serde_json::json!({
        "session_state": text("state"),
        "capture_source": source,
        "injection_backend": backend,
        "end_to_end_certified": false,
        "certification_note": "Live capabilities are session status, not proof from an end-to-end test in each application.",
        "capture": capture,
        "injection": injection,
        "focus_tracking": focus_tracking,
        "password_field_protection": password_protection,
        "replacement_assurance": replacement_assurance,
    })
}

fn capability(
    status: &str,
    summary: &str,
    details: &str,
    next_steps: Vec<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "summary": summary,
        "details": details,
        "next_steps": next_steps,
    })
}

fn display_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "not reported",
    }
}

pub(crate) fn runtime_route_summary(snapshot: &serde_json::Value) -> serde_json::Value {
    let text = |key: &str| snapshot.get(key).and_then(serde_json::Value::as_str);
    let number = |key: &str| snapshot.get(key).and_then(serde_json::Value::as_u64);
    let insertion_mode = text("inject_insertion_mode").or_else(|| text("backend_mode"));
    let latency = |prefix: &str| {
        serde_json::json!({
            "p50_us": number(&format!("{prefix}_latency_p50_us")),
            "p95_us": number(&format!("{prefix}_latency_p95_us")),
            "p99_us": number(&format!("{prefix}_latency_p99_us")),
            "sample_count": number(&format!("{prefix}_latency_sample_count")),
        })
    };
    serde_json::json!({
        "capture_source": text("source"),
        "injection_backend": text("backend"),
        "backend_mode": text("backend_mode"),
        "insertion_mode": insertion_mode,
        "keysym_fallback": insertion_mode.is_some_and(|mode| mode.contains("keysym fallback")),
        "sensitive_field_detection": snapshot.get("capture_sensitive_focus"),
        "injection_latency": latency("injection"),
        "matcher_latency": latency("matcher"),
        "injection_latency_profiles": snapshot
            .get("injection_latency_profiles")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "daemon_compatibility": upgrade_status_from_snapshot(snapshot),
    })
}

fn print_percentiles(label: &str, metrics: &serde_json::Value) {
    if let Some(line) = percentile_line(label, metrics) {
        println!("{line}");
    }
}

fn percentile_line(label: &str, metrics: &serde_json::Value) -> Option<String> {
    if metrics["sample_count"].as_u64().unwrap_or(0) == 0 {
        return None;
    }
    let percentile = |name: &str| match metrics[name].as_u64() {
        Some(value) => format!("{value} µs"),
        None => "n/a".to_owned(),
    };
    Some(format!(
        "{label}: p50 {}, p95 {}, p99 {}, {} samples",
        percentile("p50_us"),
        percentile("p95_us"),
        metrics["p99_us"]
            .as_u64()
            .map(|value| format!("{value} µs"))
            .unwrap_or_else(|| "n/a".to_owned()),
        metrics["sample_count"].as_u64().unwrap_or(0)
    ))
}

fn latency_profile_lines(profiles: &serde_json::Value) -> Vec<String> {
    let Some(modes) = profiles.as_object() else {
        return Vec::new();
    };
    modes
        .iter()
        .flat_map(|(mode, buckets)| {
            buckets
                .as_object()
                .into_iter()
                .flat_map(move |buckets| buckets.iter())
                .filter_map(move |(bucket, metrics)| {
                    percentile_line(
                        &format!("Backend latency ({mode}, {bucket} expansions)"),
                        metrics,
                    )
                })
        })
        .collect()
}

/// Print the running daemon's route and output health. Returns false when the
/// daemon reports a stalled output route, refuses typed expansions, or is
/// stale/incompatible with this CLI. An unreachable daemon is not a failure
/// here (doctor also runs before the service is started).
pub(crate) fn print_output_health_diagnostics() -> bool {
    let Some(snapshot) = read_daemon_status()
        .ok()
        .and_then(|response| status_as_json(&response).ok())
    else {
        print_session_capabilities(&serde_json::Value::Null);
        return true;
    };
    let route = runtime_route_summary(&snapshot);
    print_session_capabilities(&snapshot);
    if let (Some(source), Some(backend)) = (
        route["capture_source"].as_str(),
        route["injection_backend"].as_str(),
    ) {
        println!("Active route: {source} capture → {backend} injection");
    }
    if let Some(mode) = route["insertion_mode"].as_str() {
        println!("Injection mode: {mode}");
    }
    if let Some(detail) = route["backend_mode"].as_str() {
        if Some(detail) != route["insertion_mode"].as_str() {
            println!("Injection detail: {detail}");
        }
    }
    if route["keysym_fallback"] == true {
        println!(
            "Injection warning: keysym fallback types text key by key; Unicode coverage is limited and longer expansions take longer."
        );
    }
    match route["sensitive_field_detection"].as_bool() {
        Some(true) => println!("Sensitive-field detection: available on the active capture route"),
        Some(false) => println!(
            "Sensitive-field warning: the active capture route cannot identify password fields"
        ),
        None => println!("Sensitive-field detection: not reported by the active daemon"),
    }
    print_percentiles("Backend injection latency", &route["injection_latency"]);
    print_percentiles("Matcher latency", &route["matcher_latency"]);
    print_latency_profiles(&route["injection_latency_profiles"]);
    let daemon_status = route["daemon_compatibility"]["status"]
        .as_str()
        .unwrap_or("unverified");
    if daemon_status != "current" {
        println!(
            "Daemon compatibility: {daemon_status} — {}",
            route["daemon_compatibility"]["action"]
                .as_str()
                .unwrap_or("verify the daemon before relying on these diagnostics")
        );
    }
    let insertion = insertion_from_status(&snapshot);
    if let Some(explanation) = insertion["explanation"].as_str() {
        println!("Replacement assurance: {explanation}");
    }
    if let Some(refused) = insertion["refused_total"]
        .as_u64()
        .filter(|total| *total > 0)
    {
        println!(
            "Replacement refusals: {refused} expansion(s) refused before typing (last: {}); \
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
    insertion_ok && matches!(daemon_status, "current" | "unverified")
}

fn print_session_capabilities(snapshot: &serde_json::Value) {
    let report = session_capabilities_from_status(snapshot);
    println!("Current-session capabilities (live status, not end-to-end certification):");
    for (key, label) in [
        ("capture", "Capture"),
        ("injection", "Injection"),
        ("focus_tracking", "Focus tracking"),
        ("password_field_protection", "Password-field protection"),
        ("replacement_assurance", "Replacement assurance"),
    ] {
        let check = &report[key];
        println!(
            "  {label}: {} — {}",
            check["status"].as_str().unwrap_or("unknown"),
            check["summary"].as_str().unwrap_or("No details reported")
        );
        if let Some(details) = check["details"].as_str() {
            println!("    {details}");
        }
        if let Some(steps) = check["next_steps"].as_array() {
            for step in steps.iter().filter_map(serde_json::Value::as_str) {
                println!("    Next: {step}");
            }
        }
    }
}

fn print_latency_profiles(profiles: &serde_json::Value) {
    for line in latency_profile_lines(profiles) {
        println!("{line}");
    }
}

pub(crate) fn status_schema_compatible(snapshot: &serde_json::Value) -> bool {
    snapshot
        .get("status_schema")
        .and_then(serde_json::Value::as_u64)
        == Some(u64::from(CONTROL_STATUS_SCHEMA))
}

/// Whether the daemon was built from the same commit as this CLI. A build
/// from a source archive without Git metadata (distribution packages, unless
/// `WAYEXPAND_BUILD_SHA` is set) reports `unknown`, and two such builds of
/// different releases would otherwise "match"; an unknown commit therefore
/// never counts as a match.
pub(crate) fn daemon_commit_matches(daemon_commit: Option<&str>) -> bool {
    commits_match(daemon_commit, crate::build_info::COMMIT)
}

/// Whether either side has no commit to compare, so a match cannot be shown.
pub(crate) fn daemon_commit_unknown(daemon_commit: Option<&str>) -> bool {
    commit_unknown(daemon_commit, crate::build_info::COMMIT)
}

fn commit_unknown(daemon: Option<&str>, cli: &str) -> bool {
    cli == "unknown" || daemon.is_none_or(|commit| commit == "unknown")
}

fn commits_match(daemon: Option<&str>, cli: &str) -> bool {
    !commit_unknown(daemon, cli) && daemon == Some(cli)
}

/// Compare the running daemon with the CLI that is reporting its status.
/// A responsive socket is not sufficient: the daemon may still be an older
/// installation with different safety or output semantics.
pub(crate) fn upgrade_status_from_snapshot(snapshot: &serde_json::Value) -> serde_json::Value {
    let daemon_commit = snapshot
        .get("daemon_commit")
        .and_then(serde_json::Value::as_str);
    let compatible = status_schema_compatible(snapshot);
    let commit_matches = daemon_commit_matches(daemon_commit);
    let status = if !compatible {
        "incompatible"
    } else if daemon_commit_unknown(daemon_commit) {
        "unverified"
    } else if !commit_matches {
        "restart-required"
    } else {
        "current"
    };
    let action = match status {
        "incompatible" => {
            "The running daemon does not speak this CLI's status protocol; stop it and start the installed daemon."
        }
        "restart-required" => {
            "The running daemon is from a different build; stop and restart it using the same launch method, and ensure the updated binary is used. If systemd manages it, restart the corresponding user service before relying on this CLI."
        }
        "unverified" => {
            "This CLI or the daemon was built without a commit ID (for example from a source archive), so it cannot tell whether the daemon is current; stop and restart it using the same launch method after every upgrade, and ensure the updated binary is used. If systemd manages it, restart the corresponding user service."
        }
        _ => "The running daemon matches this CLI build.",
    };
    serde_json::json!({
        "status": status,
        "action": action,
        "cli": {
            "version": crate::build_info::VERSION,
            "commit": crate::build_info::COMMIT,
        },
        "daemon": {
            "available": true,
            "commit": daemon_commit,
            "status_schema": snapshot.get("status_schema"),
            "state": snapshot.get("state"),
            "source": snapshot.get("source"),
            "backend": snapshot.get("backend"),
        },
        "compatible": compatible,
        "commit_matches": commit_matches,
    })
}

pub(crate) fn upgrade_status_unavailable() -> serde_json::Value {
    serde_json::json!({
        "status": "unavailable",
        "action": "No daemon responded; start the appropriate user service, then rerun this command.",
        "cli": {
            "version": crate::build_info::VERSION,
            "commit": crate::build_info::COMMIT,
        },
        "daemon": {
            "available": false,
            "commit": serde_json::Value::Null,
            "status_schema": serde_json::Value::Null,
            "state": serde_json::Value::Null,
            "source": serde_json::Value::Null,
            "backend": serde_json::Value::Null,
        },
        "compatible": false,
        "commit_matches": false,
    })
}

#[cfg(test)]
mod runtime_route_tests {
    use super::*;

    #[test]
    fn runtime_route_summary_separates_fallback_and_latency_metrics() {
        let snapshot = serde_json::json!({
            "source": "evdev",
            "backend": "libei",
            "backend_mode": "ei_keyboard keysym fallback (12ms key pacing)",
            "inject_insertion_mode": "libei keysym fallback",
            "capture_sensitive_focus": false,
            "injection_latency_p50_us": 300_000,
            "injection_latency_p95_us": 1_200_000,
            "injection_latency_p99_us": 1_300_000,
            "injection_latency_sample_count": 12,
            "injection_latency_profiles": {
                "libei_keysym_fallback": {
                    "small": {
                        "sample_count": 8,
                        "window_count": 8,
                        "p50_us": 250_000,
                        "p95_us": 500_000,
                        "p99_us": 500_000
                    },
                    "medium": {
                        "sample_count": 4,
                        "window_count": 4,
                        "p50_us": 900_000,
                        "p95_us": 1_200_000,
                        "p99_us": 1_200_000
                    }
                }
            },
            "matcher_latency_p50_us": 4,
            "matcher_latency_p95_us": 12,
            "matcher_latency_p99_us": 20,
            "matcher_latency_sample_count": 500,
            "status_schema": CONTROL_STATUS_SCHEMA,
            "daemon_commit": crate::build_info::COMMIT,
        });

        let summary = runtime_route_summary(&snapshot);

        assert_eq!(summary["capture_source"], "evdev");
        assert_eq!(summary["injection_backend"], "libei");
        assert_eq!(summary["keysym_fallback"], true);
        assert_eq!(summary["sensitive_field_detection"], false);
        assert_eq!(summary["injection_latency"]["p95_us"], 1_200_000);
        assert_eq!(summary["injection_latency"]["sample_count"], 12);
        assert_eq!(
            summary["injection_latency_profiles"]["libei_keysym_fallback"]["medium"]["p95_us"],
            1_200_000
        );
        assert_eq!(
            latency_profile_lines(&summary["injection_latency_profiles"]),
            [
                "Backend latency (libei_keysym_fallback, medium expansions): p50 900000 µs, p95 1200000 µs, p99 1200000 µs, 4 samples",
                "Backend latency (libei_keysym_fallback, small expansions): p50 250000 µs, p95 500000 µs, p99 500000 µs, 8 samples",
            ]
        );
        assert_eq!(summary["matcher_latency"]["p95_us"], 12);
        assert_eq!(summary["matcher_latency"]["sample_count"], 500);
        let expected_status = if crate::build_info::COMMIT == "unknown" {
            "unverified"
        } else {
            "current"
        };
        assert_eq!(summary["daemon_compatibility"]["status"], expected_status);
    }

    #[test]
    fn runtime_route_summary_identifies_a_stale_daemon_build() {
        let snapshot = serde_json::json!({
            "status_schema": CONTROL_STATUS_SCHEMA,
            "daemon_commit": "old-build",
        });

        let summary = runtime_route_summary(&snapshot);

        let expected_status = if crate::build_info::COMMIT == "unknown" {
            "unverified"
        } else {
            "restart-required"
        };
        assert_eq!(summary["daemon_compatibility"]["status"], expected_status);
    }
}

#[cfg(test)]
mod commit_tests {
    use super::{commit_unknown, commits_match};

    #[test]
    fn unknown_commits_never_match() {
        assert!(commits_match(Some("abc123"), "abc123"));
        assert!(!commits_match(Some("abc123"), "def456"));
        assert!(!commits_match(None, "abc123"));
        // Two source-archive builds of different releases both say unknown.
        assert!(!commits_match(Some("unknown"), "unknown"));
        assert!(!commits_match(Some("abc123"), "unknown"));
        assert!(commit_unknown(Some("unknown"), "abc123"));
        assert!(commit_unknown(None, "abc123"));
        assert!(!commit_unknown(Some("def456"), "abc123"));
    }
}
