use super::*;
use crate::doctor::backends::{
    automatic_selection_is_ready, capture_path_available, capture_readiness, display_session_flags,
};
use crate::doctor::certification::certification_selection_status;
use crate::doctor::files::existing_control_socket_is_healthy;
use crate::doctor::json::{production_readiness, protocol_probe_report};
use crate::doctor::policy::{
    absolute_command_policy_diagnostic, composition_policy_diagnostic,
    exclusive_capture_policy_diagnostic, print_policy_diagnostics_json,
};
use crate::doctor::status::{runtime_capabilities_from_status, status_schema_compatible};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn live_explain_requires_matching_daemon_build_and_status_schema() {
    let cli_commit = "tested-cli-build";
    let current = format!(
        "running\nstatus_schema={}\ndaemon_commit={}\n",
        CONTROL_STATUS_SCHEMA, cli_commit
    );
    assert_eq!(
        crate::commands::system::live_explain_build_gate(&current, cli_commit),
        None
    );

    let stale = format!(
        "running\nstatus_schema={}\ndaemon_commit=older-build\n",
        CONTROL_STATUS_SCHEMA
    );
    assert!(
        crate::commands::system::live_explain_build_gate(&stale, cli_commit)
            .unwrap()
            .contains("different build")
    );

    let unknown = format!("running\nstatus_schema={}\n", CONTROL_STATUS_SCHEMA);
    assert!(
        crate::commands::system::live_explain_build_gate(&unknown, cli_commit)
            .unwrap()
            .contains("cannot be verified")
    );

    let source_archive = format!(
        "running\nstatus_schema={}\ndaemon_commit=unknown\n",
        CONTROL_STATUS_SCHEMA
    );
    assert!(
        crate::commands::system::live_explain_build_gate(&source_archive, "unknown")
            .unwrap()
            .contains("cannot be verified")
    );

    let incompatible = format!(
        "running\nstatus_schema={}\ndaemon_commit={}\n",
        CONTROL_STATUS_SCHEMA - 1,
        cli_commit
    );
    assert!(
        crate::commands::system::live_explain_build_gate(&incompatible, cli_commit)
            .unwrap()
            .contains("incompatible status protocol")
    );
}

#[test]
fn doctor_protocol_probe_report_distinguishes_available_and_missing_globals() {
    let capabilities = wayexpand_backend_selection::Capabilities {
        has_input_method_v2: false,
        has_virtual_keyboard: true,
        has_window_tracker: false,
        compositor: wayexpand_backend_selection::Compositor::KdePlasma,
        ..Default::default()
    };
    let probes = protocol_probe_report(&capabilities, true);
    assert_eq!(probes["input_method_v2"]["status"], "unavailable");
    assert_eq!(probes["wlroots_virtual_keyboard"]["status"], "available");
    assert_eq!(probes["kwin_window_tracker"]["status"], "unavailable");

    let kde_without_wayland = wayexpand_backend_selection::Capabilities {
        compositor: wayexpand_backend_selection::Compositor::KdePlasma,
        ..Default::default()
    };
    let probes = protocol_probe_report(&kde_without_wayland, false);
    assert_eq!(probes["input_method_v2"]["status"], "not_applicable");
    assert_eq!(
        probes["wlroots_virtual_keyboard"]["status"],
        "not_applicable"
    );
    assert_eq!(probes["kwin_window_tracker"]["status"], "not_applicable");
}

#[test]
fn doctor_policy_json_reports_absolute_command_requirement() {
    let policy = OrganizationPolicy {
        safe_mode: false,
        require_absolute_commands: true,
        ..OrganizationPolicy::default()
    };
    let diagnostics = print_policy_diagnostics_json(&Ok(policy));

    assert_eq!(
        diagnostics["policy"]["require_absolute_commands"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(
        diagnostics["policy"]["require_exclusive_capture"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(
        absolute_command_policy_diagnostic(&OrganizationPolicy {
            safe_mode: false,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        }),
        Some("Require absolute command paths: audit only")
    );
}

#[test]
fn doctor_policy_json_reports_exclusive_capture_requirement() {
    let policy = OrganizationPolicy {
        safe_mode: true,
        require_exclusive_capture: true,
        ..OrganizationPolicy::default()
    };
    let diagnostics = print_policy_diagnostics_json(&Ok(policy.clone()));
    assert_eq!(
        diagnostics["policy"]["require_exclusive_capture"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(
        exclusive_capture_policy_diagnostic(&policy),
        Some("Require exclusive keyboard capture: enforced")
    );
    assert_eq!(
        exclusive_capture_policy_diagnostic(&OrganizationPolicy {
            safe_mode: false,
            ..policy
        }),
        Some("Require exclusive keyboard capture: audit only")
    );
}

#[test]
fn doctor_policy_reports_external_composition_requirement() {
    let policy = OrganizationPolicy {
        safe_mode: true,
        require_composition_aware: true,
        ..OrganizationPolicy::default()
    };
    let diagnostics = print_policy_diagnostics_json(&Ok(policy.clone()));
    assert_eq!(
        diagnostics["policy"]["require_composition_aware"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(
        composition_policy_diagnostic(&policy),
        Some("Require external IME/preedit awareness: enforced")
    );
    assert_eq!(
        composition_policy_diagnostic(&OrganizationPolicy {
            safe_mode: false,
            ..policy
        }),
        Some("Require external IME/preedit awareness: audit only")
    );
}

#[test]
fn production_readiness_never_promotes_capabilities_to_certification() {
    let report = production_readiness(&serde_json::json!({
        "available": true,
        "compatible": true,
        "commit_matches": true,
        "state": "connected",
        "capabilities": {
            "exclusive_capture": true,
            "sensitive_field_detection": true,
            "composition_awareness": true,
            "layout_awareness": true,
            "key_passthrough": true,
            "exact_window_identity": true,
            "atomic_replacement": true,
            "full_unicode": true
        }
    }));
    assert_eq!(report["status"], "uncertified");
    assert_eq!(report["certified"], false);
    assert_eq!(report["live_route"], true);
    assert_eq!(
        report["blockers"],
        serde_json::json!(["reviewed compositor/client certification evidence"])
    );
}

#[test]
fn production_readiness_reports_limited_live_capabilities() {
    let report = production_readiness(&serde_json::json!({
        "available": true,
        "compatible": true,
        "commit_matches": true,
        "state": "connected",
        "capabilities": {
            "exclusive_capture": false,
            "sensitive_field_detection": false,
            "composition_awareness": false,
            "layout_awareness": false,
            "key_passthrough": false,
            "exact_window_identity": true,
            "atomic_replacement": false,
            "full_unicode": false
        }
    }));
    assert_eq!(report["status"], "limited");
    assert_eq!(report["certified"], false);
    assert_eq!(report["capabilities"]["exclusive_capture"], false);
    let blockers = report["blockers"].as_array().unwrap();
    assert!(blockers.len() >= 5);
    assert!(blockers.contains(&serde_json::json!("external IME/preedit awareness")));
    assert!(blockers.contains(&serde_json::json!("exclusive keyboard capture")));
}

#[test]
fn backup_rejects_a_fifo_source_without_blocking_and_copies_files_exactly() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("wayexpand-backup-fifo-{suffix}"));
    fs::create_dir(&directory).unwrap();
    let fifo = directory.join("config.toml");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: `path` is a valid NUL-terminated path for this call.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let error = create_backup(&fifo, &directory.join("fifo.bak")).unwrap_err();
    assert!(error.to_string().contains("not a regular file"));
    assert!(!directory.join("fifo.bak").exists());

    let source = directory.join("source.toml");
    fs::write(&source, "[[expansion]]\n").unwrap();
    create_backup(&source, &directory.join("source.bak")).unwrap();
    assert_eq!(
        fs::read(directory.join("source.bak")).unwrap(),
        fs::read(&source).unwrap()
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn backup_refuses_existing_destination_atomically() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = std::env::temp_dir().join(format!("wayexpand-backup-source-{suffix}"));
    let destination = std::env::temp_dir().join(format!("wayexpand-backup-dest-{suffix}"));
    fs::write(&source, "source").unwrap();
    fs::write(&destination, "existing").unwrap();

    let error = create_backup(&source, &destination).unwrap_err();
    assert!(error.to_string().contains("creating configuration backup"));
    assert_eq!(fs::read_to_string(&destination).unwrap(), "existing");

    let _ = fs::remove_file(source);
    let _ = fs::remove_file(destination);
}

#[test]
fn status_json_preserves_types_and_ignores_banner() {
    let value = status_as_json(
        "running\nsource=stdin\nroute_id=stdin-test\nstatus_schema=11\ndaemon_commit=test-commit\npaused=true\ncommand_queue_depth=3\nconfig_state=ok\ncapture_sensitive_focus=false\nwindow_tracker_connected=true\nwindow_identity_exact=true\ninject_full_unicode=true\nclipboard_reads_total=6\nclipboard_read_max_duration_us=125000\n",
    )
    .unwrap();
    assert_eq!(value["response"], "running");
    assert_eq!(value["source"], "stdin");
    assert_eq!(value["route_id"], "stdin-test");
    assert_eq!(value["status_schema"], 11);
    assert_eq!(value["daemon_commit"], "test-commit");
    assert_eq!(value["paused"], true);
    assert_eq!(value["command_queue_depth"], 3);
    assert_eq!(value["config_state"], "ok");
    assert_eq!(value["capture_sensitive_focus"], false);
    assert_eq!(value["window_tracker_connected"], true);
    assert_eq!(value["inject_full_unicode"], true);
    assert_eq!(value["clipboard_reads_total"], 6);
    assert_eq!(value["clipboard_read_max_duration_us"], 125_000);
}

#[test]
fn status_json_omits_ambiguous_duplicate_fields() {
    let value = status_as_json(
        "running\nstatus_schema=13\ndaemon_commit=real-build\ndaemon_commit=spoofed-build\npaused=false\npaused=true\n",
    )
    .unwrap();

    assert_eq!(value["status_schema"], 13);
    assert!(value.get("daemon_commit").is_none());
    assert!(value.get("paused").is_none());
}

#[test]
fn doctor_explains_verified_compatibility_and_blocked_insertion() {
    let insertion =
        |status: serde_json::Value| crate::doctor::status::insertion_from_status(&status);
    let verified = insertion(serde_json::json!({
        "insertion_assurance": "verified",
        "compatibility_mode": "off",
        "focus_assurance": "protocol",
        "output_refused_total": 0,
        "output_last_refusal": "none",
    }));
    assert!(verified["explanation"]
        .as_str()
        .unwrap()
        .starts_with("target-text verified:"));
    assert_eq!(verified["replacement_assurance"], "verified");
    assert!(verified["last_refusal"].is_null());

    let tracked = insertion(serde_json::json!({
        "replacement_assurance": "focus-tracked-best-effort",
        "compatibility_mode": "tracked-focus",
        "focus_assurance": "tracker",
        "output_refused_total": 0,
        "output_last_refusal": "none",
    }));
    assert!(tracked["explanation"]
        .as_str()
        .unwrap()
        .contains("cannot verify that the text at the cursor"));

    let unverified = insertion(serde_json::json!({
        "replacement_assurance": "unverified",
        "compatibility_mode": "untracked-focus",
        "focus_assurance": "unconfirmed",
    }));
    assert!(unverified["explanation"]
        .as_str()
        .unwrap()
        .starts_with("unverified replacement:"));

    let compatibility = insertion(serde_json::json!({
        "insertion_assurance": "compatibility",
        "compatibility_mode": "untracked-focus",
        "focus_assurance": "unconfirmed",
        "output_refused_total": 2,
        "output_last_refusal": "focus-changed",
    }));
    let explanation = compatibility["explanation"].as_str().unwrap();
    assert!(explanation.contains("without seeing the field's text"));
    assert!(explanation.contains("focus cannot be confirmed"));
    assert_eq!(compatibility["refused_total"], 2);
    assert_eq!(compatibility["last_refusal"], "focus-changed");

    let blocked = insertion(serde_json::json!({
        "insertion_assurance": "blocked",
        "compatibility_mode": "off",
    }));
    assert!(blocked["explanation"]
        .as_str()
        .unwrap()
        .contains("compatibility_mode"));
    let readiness = crate::doctor::json::production_readiness(&serde_json::json!({
        "insertion": blocked,
    }));
    assert!(readiness["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|blocker| blocker
            .as_str()
            .unwrap()
            .contains("typed expansions are refused")));
}

#[test]
fn doctor_reports_current_session_capabilities_separately_with_recovery_steps() {
    let report = crate::doctor::status::session_capabilities_from_status(&serde_json::json!({
        "state": "connected",
        "source": "evdev",
        "backend": "libei",
        "capture_exclusive": false,
        "capture_reliable_key_state": true,
        "capture_layout_aware": false,
        "capture_sensitive_focus": false,
        "window_tracker_connected": true,
        "window_identity_exact": true,
        "focus_assurance": "tracker",
        "inject_insertion_mode": "ei_text",
        "inject_full_unicode": true,
        "inject_atomic_replace": false,
        "replacement_assurance": "focus-tracked-best-effort",
        "compatibility_mode": "tracked-focus",
    }));

    assert_eq!(report["capture_source"], "evdev");
    assert_eq!(report["injection_backend"], "libei");
    assert_eq!(report["capture"]["status"], "active-shared");
    assert_eq!(report["injection"]["status"], "active");
    assert_eq!(report["focus_tracking"]["status"], "exact-window-tracked");
    assert_eq!(report["password_field_protection"]["status"], "unavailable");
    assert_eq!(report["replacement_assurance"]["status"], "best-effort");
    assert_eq!(report["end_to_end_certified"], false);
    for key in [
        "capture",
        "injection",
        "focus_tracking",
        "password_field_protection",
        "replacement_assurance",
    ] {
        assert!(
            !report[key]["next_steps"].as_array().unwrap().is_empty(),
            "{key}"
        );
    }
}

#[test]
fn doctor_does_not_claim_capabilities_without_a_live_route() {
    let report = crate::doctor::status::session_capabilities_from_status(&serde_json::Value::Null);
    assert_eq!(report["capture"]["status"], "unavailable");
    assert_eq!(report["injection"]["status"], "unavailable");
    assert_eq!(report["focus_tracking"]["status"], "unavailable");
    assert_eq!(report["password_field_protection"]["status"], "unknown");
    assert_eq!(report["replacement_assurance"]["status"], "unknown");
}

#[test]
fn doctor_reports_stalled_output_with_route_specific_recovery() {
    let stalled = crate::doctor::status::output_health_from_status(&serde_json::json!({
        "source": "evdev",
        "output_health": "stalled",
        "output_uncertain_total": 2,
        "output_retiring_workers": 1,
        "output_stalled_ms": 9000,
    }));
    assert_eq!(stalled["health"], "stalled");
    assert_eq!(stalled["uncertain_transactions_total"], 2);
    assert!(stalled["recovery"]
        .as_str()
        .unwrap()
        .contains("systemctl --user restart wayexpand-evdev.service"));
    let readiness = crate::doctor::json::production_readiness(&serde_json::json!({
        "available": true,
        "output": stalled,
    }));
    assert!(readiness["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|blocker| blocker.as_str().unwrap().contains("stalled output worker")));

    let recovering = crate::doctor::status::output_health_from_status(&serde_json::json!({
        "output_health": "recovering",
    }));
    assert!(recovering["recovery"].is_null());
}

#[test]
fn status_schema_rejects_missing_or_newer_incompatible_daemons() {
    assert!(status_schema_compatible(
        &serde_json::json!({ "status_schema": CONTROL_STATUS_SCHEMA })
    ));
    assert!(!status_schema_compatible(&serde_json::json!({})));
    assert!(!status_schema_compatible(
        &serde_json::json!({ "status_schema": 7 })
    ));
    assert!(!status_schema_compatible(
        &serde_json::json!({ "status_schema": CONTROL_STATUS_SCHEMA + 1 })
    ));
    assert!(!status_schema_compatible(
        &serde_json::json!({ "status_schema": "1" })
    ));
}

#[test]
fn upgrade_status_distinguishes_current_stale_and_incompatible_daemons() {
    let current = crate::doctor::status::upgrade_status_from_snapshot(&serde_json::json!({
        "daemon_commit": crate::build_info::COMMIT,
        "status_schema": CONTROL_STATUS_SCHEMA,
        "state": "connected",
        "source": "input-method",
        "backend": "input-method-v2",
    }));
    if crate::build_info::COMMIT == "unknown" {
        assert_eq!(current["status"], "unverified");
        assert_eq!(current["commit_matches"], false);
    } else {
        assert_eq!(current["status"], "current");
        assert_eq!(current["commit_matches"], true);
    }
    assert_eq!(current["compatible"], true);

    let stale = crate::doctor::status::upgrade_status_from_snapshot(&serde_json::json!({
        "daemon_commit": "older-build",
        "status_schema": CONTROL_STATUS_SCHEMA,
    }));
    let stale_status = if crate::build_info::COMMIT == "unknown" {
        "unverified"
    } else {
        "restart-required"
    };
    assert_eq!(stale["status"], stale_status);
    assert_eq!(stale["commit_matches"], false);
    if crate::build_info::COMMIT == "unknown" {
        assert!(stale["action"]
            .as_str()
            .unwrap()
            .contains("cannot tell whether"));
        assert!(stale["action"]
            .as_str()
            .unwrap()
            .contains("same launch method"));
    } else {
        let action = stale["action"].as_str().unwrap();
        assert!(action.contains("same launch method"));
        assert!(action.contains("updated binary"));
        assert!(action.contains("systemd"));
    }

    let incompatible = crate::doctor::status::upgrade_status_from_snapshot(
        &serde_json::json!({ "daemon_commit": crate::build_info::COMMIT }),
    );
    assert_eq!(incompatible["status"], "incompatible");
    assert_eq!(incompatible["compatible"], false);

    let unavailable = crate::doctor::status::upgrade_status_unavailable();
    assert_eq!(unavailable["status"], "unavailable");
    assert_eq!(unavailable["daemon"]["available"], false);
}

#[test]
fn certification_separates_capture_and_injection_capabilities() {
    let status = status_as_json(
        "running\nsource=evdev\nbackend=libei\ncapture_sensitive_focus=false\ncapture_exclusive=false\ninject_full_unicode=true\ninject_atomic_replace=false\n",
    )
    .unwrap();
    let capabilities = runtime_capabilities_from_status(&status);
    assert_eq!(capabilities["capture"]["sensitive_focus"], false);
    assert_eq!(capabilities["capture"]["exclusive"], false);
    assert_eq!(
        capabilities["capture"]["composition_aware"],
        serde_json::Value::Null
    );
    assert_eq!(capabilities["injection"]["full_unicode"], true);
    assert_eq!(capabilities["injection"]["atomic_replace"], false);
}

/// Contract test for docs/COMPATIBILITY.md's `wayexpand status --json`
/// section: the exact daemon status line documented there as the
/// "Stable" example must still produce exactly the documented field
/// set (no more, no less) and types. If this fails, either the
/// implementation changed in a way that needs a compatibility note, or
/// the documentation needs to be updated to match -- either way it
/// should not be silently discovered by a user's integration breaking.
#[test]
fn status_json_matches_documented_stable_contract() {
    let daemon_response = "running\n\
         source=input-method\n\
         backend=input-method-v2\n\
         route_id=input-method-v2\n\
         backend_mode=unknown\n\
         status_schema=15\n\
         daemon_commit=test-commit\n\
         state=connected\n\
         paused=false\n\
         config=/home/user/.config/wayexpand/expansions.toml\n\
         config_state=ok\n\
         capture_sensitive_focus=true\n\
         capture_exclusive=true\n\
         capture_reliable_key_state=true\n\
         capture_key_passthrough=false\n\
         capture_composition_aware=false\n\
         capture_local_compose_aware=true\n\
         capture_layout_aware=true\n\
         window_tracker_connected=false\n\
         window_identity_exact=false\n\
         inject_atomic_replace=true\n\
         inject_full_unicode=true\n\
         inject_cursor_reposition=false\n\
         inject_key_passthrough=false\n\
         inject_insertion_mode=ei_text\n\
         inject_max_text_chars=0\n\
         inject_expected_throughput_chars_per_sec=0\n\
         command_queue_depth=0\n\
         command_in_flight=0\n\
         expansion_command_queue_depth=0\n\
         expansion_command_in_flight=0\n\
         hotkey_queue_depth=0\n\
         hotkey_in_flight=0\n\
         command_queue_rejected_total=0\n\
         command_timeout_total=0\n\
         command_failure_total=0\n\
         injection_latency_sample_count=0\n\
         injection_latency_window_count=0\n\
         injection_latency_mean_us=0\n\
         injection_latency_p50_us=0\n\
         injection_latency_p95_us=0\n\
         injection_latency_p99_us=0\n\
         injection_latency_dropped_total=0\n\
         injection_latency_profile_dropped_total=0\n\
         injection_latency_profiles={}\n\
         matcher_latency_sample_count=0\n\
         matcher_latency_window_count=0\n\
         matcher_latency_dropped_total=0\n\
         matcher_latency_p50_us=0\n\
         matcher_latency_p95_us=0\n\
         matcher_latency_p99_us=0\n\
         usage_flush_max_duration_us=25\n\
         usage_flush_failures_total=1\n\
         usage_queue_rejected_total=2\n\
         usage_worker_failures_total=0\n\
         usage_events_recorded_total=20\n\
         usage_events_persisted_total=18\n\
         usage_events_dropped_total=2\n\
         clipboard_worker_threads=1\n\
         clipboard_reads_in_flight=0\n\
         clipboard_reads_total=4\n\
         clipboard_read_failures_total=1\n\
         clipboard_deadline_misses_total=2\n\
         clipboard_read_max_duration_us=150000\n\
         output_health=ok\n\
         output_uncertain_total=1\n\
         output_retiring_workers=0\n\
         output_stalled_ms=0\n\
         insertion_assurance=verified\n\
         replacement_assurance=verified\n\
         compatibility_mode=off\n\
         focus_assurance=protocol\n\
         output_refused_total=3\n\
         output_last_refusal=focus-changed\n\
         output_unsupported_total=7\n\
         output_last_unsupported=unicode\n\
         recent_expansion_failures=[]\n\
         output_reconnect_total=1\n\
         input_reconnect_total=2\n\
         input_overflow_total=3\n\
         evdev_quarantined_events_total=7\n\
         control_rejected_total=4\n\
         memory_rss_kib=5\n\
         memory_high_water_kib=6";
    let value = status_as_json(daemon_response).unwrap();
    let object = value.as_object().expect("status --json returns an object");
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/contracts/status-json.json"))
            .expect("status contract fixture must be valid JSON");
    assert_eq!(contract["schema_version"], 10);
    assert_eq!(contract["daemon_control_schema"], CONTROL_STATUS_SCHEMA);
    assert_ne!(
        contract["schema_version"],
        contract["daemon_control_schema"]
    );
    let documented_fields = contract["fields"]
        .as_object()
        .expect("status contract fields must be an object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        object
            .keys()
            .map(|key| key.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        documented_fields.into_iter().collect(),
        "status --json fields no longer match docs/COMPATIBILITY.md's documented Stable contract"
    );
    assert_eq!(value["response"], "running");
    assert_eq!(value["source"], "input-method");
    assert_eq!(value["backend"], "input-method-v2");
    assert_eq!(value["backend_mode"], "unknown");
    assert_eq!(value["state"], "connected");
    assert_eq!(value["paused"], false);
    assert_eq!(
        value["config"],
        "/home/user/.config/wayexpand/expansions.toml"
    );
    assert_eq!(value["config_state"], "ok");
    assert_eq!(value["command_queue_depth"], 0);
    assert_eq!(value["command_in_flight"], 0);
    assert_eq!(value["expansion_command_queue_depth"], 0);
    assert_eq!(value["expansion_command_in_flight"], 0);
    assert_eq!(value["hotkey_queue_depth"], 0);
    assert_eq!(value["hotkey_in_flight"], 0);
    assert_eq!(value["command_queue_rejected_total"], 0);
    assert_eq!(value["command_timeout_total"], 0);
    assert_eq!(value["command_failure_total"], 0);
    assert_eq!(value["usage_flush_max_duration_us"], 25);
    assert_eq!(value["usage_flush_failures_total"], 1);
    assert_eq!(value["usage_queue_rejected_total"], 2);
    assert_eq!(value["clipboard_worker_threads"], 1);
    assert_eq!(value["clipboard_reads_in_flight"], 0);
    assert_eq!(value["clipboard_reads_total"], 4);
    assert_eq!(value["clipboard_read_failures_total"], 1);
    assert_eq!(value["clipboard_deadline_misses_total"], 2);
    assert_eq!(value["clipboard_read_max_duration_us"], 150_000);
    assert_eq!(value["output_uncertain_total"], 1);
    assert_eq!(value["insertion_assurance"], "verified");
    assert_eq!(value["replacement_assurance"], "verified");
    assert_eq!(value["compatibility_mode"], "off");
    assert_eq!(value["focus_assurance"], "protocol");
    assert_eq!(value["output_refused_total"], 3);
    assert_eq!(value["output_last_refusal"], "focus-changed");
    assert_eq!(value["output_unsupported_total"], 7);
    assert_eq!(value["output_last_unsupported"], "unicode");
    assert_eq!(value["output_reconnect_total"], 1);
    assert_eq!(value["input_reconnect_total"], 2);
    assert_eq!(value["input_overflow_total"], 3);
    assert_eq!(value["evdev_quarantined_events_total"], 7);
    assert_eq!(value["control_rejected_total"], 4);
    assert_eq!(value["memory_rss_kib"], 5);
    assert_eq!(value["memory_high_water_kib"], 6);
}

#[test]
fn stable_cli_shape_fixture_is_valid_and_includes_status_contract() {
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/contracts/cli-json-shapes.json"
    ))
    .expect("CLI contract fixture must be valid JSON");
    let doctor_fields = contract["doctor"]
        .as_array()
        .expect("doctor contract must be an array")
        .iter()
        .map(|field| field.as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        doctor_fields,
        [
            "healthy",
            "wayland",
            "desktop",
            "config",
            "control_socket",
            "daemon",
            "production_readiness",
            "action_broker",
            "policy",
            "backends",
            "protocol_probes",
            "capabilities",
            "automatic_selection",
            "setup_recommendation",
            "capture_readiness",
        ]
        .into_iter()
        .collect()
    );
    let status_fields = contract["status"]
        .as_array()
        .expect("status contract must be an array")
        .iter()
        .map(|field| field.as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        status_fields,
        [
            "response",
            "source",
            "backend",
            "backend_mode",
            "state",
            "paused",
            "config",
            "config_state",
            "capture_sensitive_focus",
            "capture_exclusive",
            "capture_reliable_key_state",
            "capture_key_passthrough",
            "capture_composition_aware",
            "capture_local_compose_aware",
            "capture_layout_aware",
            "window_tracker_connected",
            "window_identity_exact",
            "inject_atomic_replace",
            "inject_full_unicode",
            "inject_cursor_reposition",
            "inject_key_passthrough",
            "inject_insertion_mode",
            "inject_max_text_chars",
            "inject_expected_throughput_chars_per_sec",
            "command_queue_depth",
            "command_in_flight",
            "expansion_command_queue_depth",
            "expansion_command_in_flight",
            "hotkey_queue_depth",
            "hotkey_in_flight",
            "command_queue_rejected_total",
            "command_timeout_total",
            "command_failure_total",
            "injection_latency_sample_count",
            "injection_latency_window_count",
            "injection_latency_mean_us",
            "injection_latency_p50_us",
            "injection_latency_p95_us",
            "injection_latency_p99_us",
            "injection_latency_dropped_total",
            "injection_latency_profile_dropped_total",
            "injection_latency_profiles",
            "matcher_latency_sample_count",
            "matcher_latency_window_count",
            "matcher_latency_dropped_total",
            "matcher_latency_p50_us",
            "matcher_latency_p95_us",
            "matcher_latency_p99_us",
            "usage_flush_max_duration_us",
            "usage_flush_failures_total",
            "usage_queue_rejected_total",
            "usage_worker_failures_total",
            "usage_events_recorded_total",
            "usage_events_persisted_total",
            "usage_events_dropped_total",
            "clipboard_worker_threads",
            "clipboard_reads_in_flight",
            "clipboard_reads_total",
            "clipboard_read_failures_total",
            "clipboard_deadline_misses_total",
            "clipboard_read_max_duration_us",
            "output_health",
            "output_uncertain_total",
            "output_retiring_workers",
            "output_stalled_ms",
            "insertion_assurance",
            "compatibility_mode",
            "focus_assurance",
            "output_refused_total",
            "output_last_refusal",
            "output_unsupported_total",
            "output_last_unsupported",
            "recent_expansion_failures",
            "replacement_assurance",
            "output_reconnect_total",
            "input_reconnect_total",
            "input_overflow_total",
            "evdev_quarantined_events_total",
            "control_rejected_total",
            "memory_rss_kib",
            "memory_high_water_kib",
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn json_flag_is_recognized_in_any_position() {
    let mut args = vec!["expansions.toml".to_string(), "--json".to_string()];
    assert!(take_json_flag(&mut args));
    assert_eq!(args, vec!["expansions.toml".to_string()]);

    let mut args = vec!["--json".to_string(), "expansions.toml".to_string()];
    assert!(take_json_flag(&mut args));
    assert_eq!(args, vec!["expansions.toml".to_string()]);

    let mut args = vec!["expansions.toml".to_string()];
    assert!(!take_json_flag(&mut args));
    assert_eq!(args, vec!["expansions.toml".to_string()]);
}

#[test]
fn setup_modes_do_not_promote_unavailable_paths() {
    let no_devices = wayexpand_backend_selection::Capabilities {
        has_input_method_v2: false,
        has_virtual_keyboard: true,
        has_direct_libei_socket: false,
        has_dev_input: false,
        has_window_tracker: false,
        compositor: wayexpand_backend_selection::Compositor::Gnome,
    };
    let policy = OrganizationPolicy::default();
    assert!(setup_backend_for_mode("maximum", &no_devices, &policy).is_err());
    assert!(setup_backend_for_mode("experimental", &no_devices, &policy).is_err());

    let experimental = wayexpand_backend_selection::Capabilities {
        has_input_method_v2: true,
        ..no_devices
    };
    assert_eq!(
        setup_backend_for_mode("experimental", &experimental, &policy).unwrap(),
        "input-method"
    );

    let unsupported_compositor = wayexpand_backend_selection::Capabilities {
        compositor: wayexpand_backend_selection::Compositor::Sway,
        ..experimental
    };
    assert!(setup_backend_for_mode("experimental", &unsupported_compositor, &policy).is_err());
}

#[test]
fn setup_modes_respect_runtime_backend_policy_names() {
    let capabilities = wayexpand_backend_selection::Capabilities {
        has_input_method_v2: true,
        has_direct_libei_socket: true,
        has_dev_input: true,
        ..Default::default()
    };
    let input_method_only = OrganizationPolicy {
        safe_mode: true,
        allowed_backends: vec!["input-method-v2".into()],
        ..Default::default()
    };
    assert_eq!(
        setup_backend_for_mode("experimental", &capabilities, &input_method_only).unwrap(),
        "input-method"
    );
    assert!(setup_backend_for_mode("maximum", &capabilities, &input_method_only).is_err());
    let exclusive_only = OrganizationPolicy {
        safe_mode: true,
        require_exclusive_capture: true,
        ..Default::default()
    };
    assert!(setup_backend_for_mode("maximum", &capabilities, &exclusive_only).is_err());
    assert_eq!(
        setup_backend_for_mode("experimental", &capabilities, &exclusive_only).unwrap(),
        "input-method"
    );

    // Allowing input-method-v2 does not allow IBus; it must be named.
    assert!(!setup_backend_allowed(&input_method_only, "ibus"));
    let with_ibus = OrganizationPolicy {
        allowed_backends: vec!["input-method-v2".into(), "ibus".into()],
        ..Default::default()
    };
    assert!(setup_backend_allowed(&with_ibus, "ibus"));
    // IBus replacement is not atomic, so a safe-mode atomic requirement
    // rules it out; audit mode only reports the gap.
    assert!(!setup_backend_allowed(
        &OrganizationPolicy {
            safe_mode: true,
            require_atomic_replace: true,
            ..Default::default()
        },
        "ibus"
    ));
    assert!(setup_backend_allowed(
        &OrganizationPolicy {
            require_atomic_replace: true,
            ..Default::default()
        },
        "ibus"
    ));

    let raw_only = OrganizationPolicy {
        safe_mode: true,
        allowed_backends: vec!["libei".into()],
        ..Default::default()
    };
    assert!(setup_backend_for_mode("experimental", &capabilities, &raw_only).is_err());
    assert_eq!(
        setup_backend_for_mode("maximum", &capabilities, &raw_only).unwrap(),
        "evdev"
    );

    let wlroots_only = OrganizationPolicy {
        safe_mode: true,
        allowed_backends: vec!["wlroots".into()],
        ..Default::default()
    };
    assert!(setup_backend_for_mode("maximum", &capabilities, &wlroots_only).is_err());
}

#[test]
fn recommended_mode_never_selects_evdev_even_when_available() {
    let capabilities = wayexpand_backend_selection::Capabilities {
        has_direct_libei_socket: true,
        has_dev_input: true,
        ..Default::default()
    };
    let recommendation = recommended_setup_backend(&capabilities, &OrganizationPolicy::default());
    assert_ne!(recommendation.backend, "evdev");
}

#[test]
fn capture_readiness_never_promotes_a_probe_to_verified() {
    let capabilities = wayexpand_backend_selection::Capabilities {
        has_input_method_v2: true,
        ..Default::default()
    };
    assert_eq!(
        capture_readiness(&capabilities, false, &OrganizationPolicy::default()),
        (
            "available-to-try",
            "a protocol or IBus probe succeeded; live client typing is not verified"
        )
    );
}

#[test]
fn certification_does_not_call_stdin_only_selection_available() {
    assert_eq!(certification_selection_status("stdin"), "unsupported");
    assert_eq!(certification_selection_status("ibus"), "available");
    assert_eq!(certification_selection_status("none"), "failed");
}

#[test]
fn human_capture_readiness_accepts_ibus_as_the_only_path() {
    assert!(capture_path_available(true, 0, 0));
    assert!(!capture_path_available(false, 0, 0));
}

#[test]
fn doctor_health_requires_a_graphical_session() {
    assert!(!display_session_flags(false, false, false));
    assert!(display_session_flags(true, false, false));
    assert!(display_session_flags(false, true, false));
    assert!(display_session_flags(false, false, true));
}

#[test]
fn control_socket_health_rejects_missing_regular_and_insecure_paths() {
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos()
    );
    let root = std::env::temp_dir().join(format!("wx-sock-{unique}"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let missing = root.join("missing.sock");
    assert!(!existing_control_socket_is_healthy(&missing));

    let regular = root.join("regular.sock");
    std::fs::write(&regular, b"not a socket").unwrap();
    assert!(!existing_control_socket_is_healthy(&regular));

    let socket = root.join("wayexpand.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    assert!(existing_control_socket_is_healthy(&socket));
    drop(listener);

    let insecure_parent = root.join("insecure");
    std::fs::create_dir(&insecure_parent).unwrap();
    std::fs::set_permissions(
        &insecure_parent,
        std::os::unix::fs::PermissionsExt::from_mode(0o777),
    )
    .unwrap();
    let insecure_socket = insecure_parent.join("wayexpand.sock");
    let insecure_listener = std::os::unix::net::UnixListener::bind(&insecure_socket).unwrap();
    std::fs::set_permissions(
        &insecure_socket,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    assert!(!existing_control_socket_is_healthy(&insecure_socket));
    drop(insecure_listener);
    std::fs::set_permissions(
        &insecure_parent,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn installed_ibus_is_available_to_try_not_certified() {
    let (state, detail) =
        capture_readiness(&Default::default(), true, &OrganizationPolicy::default());
    assert_eq!(state, "available-to-try");
    assert!(detail.contains("live client typing is not verified"));
}

#[test]
fn capture_readiness_hides_policy_disallowed_paths() {
    let capabilities = wayexpand_backend_selection::Capabilities {
        has_input_method_v2: true,
        has_virtual_keyboard: true,
        has_direct_libei_socket: true,
        has_dev_input: true,
        ..Default::default()
    };
    let policy = OrganizationPolicy {
        allowed_backends: vec!["none".into()],
        ..Default::default()
    };
    let (state, _) = capture_readiness(&capabilities, true, &policy);
    assert!(matches!(state, "unavailable" | "not-probed"));
}

#[test]
fn automatic_selection_health_requires_policy_permission() {
    let policy = OrganizationPolicy {
        allowed_backends: vec!["wlroots".into()],
        ..Default::default()
    };
    assert!(!automatic_selection_is_ready("evdev", "libei", &policy));
    assert!(automatic_selection_is_ready("evdev", "wlroots", &policy));
    assert!(!automatic_selection_is_ready("stdin", "wlroots", &policy));
}

#[test]
fn output_probe_without_a_capture_source_is_not_a_ready_path() {
    let capabilities = wayexpand_backend_selection::Capabilities {
        has_virtual_keyboard: true,
        ..Default::default()
    };
    let (state, detail) = capture_readiness(&capabilities, false, &OrganizationPolicy::default());
    assert!(matches!(state, "unavailable" | "not-probed"));
    assert!(
        detail.contains("no non-invasive source and output path")
            || detail.contains("no active Wayland session was detected")
    );
}

#[test]
fn preview_app_option_accepts_equals_and_separate_values() {
    let mut equals = vec![
        "--preview-app=thunderbird".to_string(),
        "config".to_string(),
    ];
    assert_eq!(
        take_option(&mut equals, "--preview-app").unwrap(),
        Some("thunderbird".into())
    );
    assert_eq!(equals, vec!["config"]);

    let mut separate = vec!["--preview-app".to_string(), "konsole".to_string()];
    assert_eq!(
        take_option(&mut separate, "--preview-app").unwrap(),
        Some("konsole".into())
    );
    assert!(separate.is_empty());
}

#[test]
fn exit_codes_follow_typed_categories_not_error_wording() {
    assert_eq!(
        exit_code_for(&usage_error("daemon socket unavailable")),
        EXIT_USAGE
    );
    assert_eq!(
        exit_code_for(&config_error("daemon socket unavailable")),
        EXIT_CONFIG
    );
    assert_eq!(
        exit_code_for(&daemon_error("configuration invalid: parse error")),
        EXIT_DAEMON
    );
    assert_eq!(
        exit_code_for(&anyhow::anyhow!("usage: this is unclassified")),
        1
    );
}

#[test]
fn help_rows_share_one_summary_column() {
    let help = help_text();
    let columns: Vec<usize> = help
        .lines()
        .filter(|line| line.starts_with("  "))
        .map(|line| {
            let usage_end = line[2..].find("  ").expect("summary separator") + 2;
            usage_end + line[usage_end..].len() - line[usage_end..].trim_start().len()
        })
        .collect();
    assert!(columns.len() > 20, "{help}");
    assert!(columns.windows(2).all(|pair| pair[0] == pair[1]), "{help}");
}

#[test]
fn help_lists_current_pack_and_profile_management_commands() {
    let help = help_text();
    for command in [
        "pack search|inspect|diff",
        "rollback|import|verify|sign",
        "profile list|create|switch|remove|assign-app|unassign-app",
    ] {
        assert!(
            help.contains(command),
            "help does not list {command:?}:\n{help}"
        );
    }
}

#[test]
fn missing_configuration_names_the_path_and_keeps_the_config_exit_code() {
    let path = Path::new("/nonexistent/wayexpand/expansions.toml");
    let error = config_load_error(path, Config::load(path).unwrap_err());
    assert_eq!(exit_code_for(&error), EXIT_CONFIG);
    assert!(error
        .to_string()
        .starts_with(&format!("no configuration file at {}", path.display())));
}

#[test]
fn repeated_default_backups_get_distinct_names() {
    let directory =
        std::env::temp_dir().join(format!("wayexpand-backup-names-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("expansions.toml");
    fs::write(&source, "").unwrap();

    let first = default_backup_destination(&source);
    assert_eq!(first, directory.join("expansions.toml.bak"));
    create_backup(&source, &first).unwrap();
    let second = default_backup_destination(&source);
    assert_eq!(second, directory.join("expansions.toml.bak.2"));
    create_backup(&source, &second).unwrap();
    assert_eq!(
        default_backup_destination(&source),
        directory.join("expansions.toml.bak.3")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unclassified_errors_keep_their_cause() {
    let error = normalize_error(
        Err::<(), _>(io::Error::new(io::ErrorKind::AlreadyExists, "File exists"))
            .context("creating configuration backup /x")
            .unwrap_err(),
    );
    assert_eq!(
        error.to_string(),
        "creating configuration backup /x: File exists"
    );
    assert_eq!(exit_code_for(&error), 1);
}

fn metric_lines_are_well_formed(text: &str) {
    for line in text.lines() {
        if line.starts_with("# HELP ") || line.starts_with("# TYPE ") {
            continue;
        }
        let (name_and_labels, value) = line.rsplit_once(' ').expect("sample has a value");
        assert!(value.parse::<f64>().is_ok(), "bad value in {line:?}");
        let name = name_and_labels.split('{').next().unwrap();
        assert!(
            name.starts_with("wayexpand_")
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "bad metric name in {line:?}"
        );
    }
}

#[test]
fn metrics_export_status_counters_without_content() {
    let snapshot = serde_json::json!({
        "response": "running",
        "route_id": "kde-evdev-libei",
        "source": "evdev",
        "backend": "libei",
        "state": "connected",
        "config_state": "ok",
        "output_health": "ok",
        "paused": false,
        "window_tracker_connected": true,
        "command_queue_depth": 2,
        "output_reconnect_total": 3,
        "future_counter_total": "7",
        "config": "/home/user/secret-project/expansions.toml",
        "recent_expansion_outcomes": "[{\"trigger\":\";pw\"}]",
    });
    let upgrade = serde_json::json!({"commit_matches": true});
    let text = commands::metrics::render_metrics(
        Some(&snapshot),
        &upgrade,
        &commands::metrics::ConfigMetrics::Valid {
            files_loaded: 2,
            expansions: 10,
            hotkeys: 1,
            policy_violations: 0,
        },
    );
    metric_lines_are_well_formed(&text);
    for expected in [
        "wayexpand_daemon_up 1",
        "wayexpand_daemon_build_matches 1",
        "wayexpand_daemon_route_info{route_id=\"kde-evdev-libei\",source=\"evdev\",backend=\"libei\",state=\"connected\"} 1",
        "wayexpand_daemon_command_queue_depth 2",
        "# TYPE wayexpand_daemon_output_reconnect_total counter",
        "wayexpand_daemon_output_reconnect_total 3",
        "wayexpand_daemon_future_counter_total 7",
        "wayexpand_config_valid 1",
        "wayexpand_config_expansions 10",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
    assert!(!text.contains("secret-project"));
    assert!(!text.contains(";pw"));
}

#[test]
fn metrics_escape_labels_and_report_a_stopped_daemon() {
    let snapshot = serde_json::json!({
        "response": "running",
        "route_id": "odd\"route\\\nid",
    });
    let text = commands::metrics::render_metrics(
        Some(&snapshot),
        &serde_json::json!({}),
        &commands::metrics::ConfigMetrics::Invalid,
    );
    metric_lines_are_well_formed(&text);
    assert!(text.contains("route_id=\"odd\\\"route\\\\\\nid\""));
    assert!(text.contains("wayexpand_config_valid 0"));
    assert!(!text.contains("wayexpand_config_expansions"));

    let stopped = commands::metrics::render_metrics(
        None,
        &crate::doctor::status::upgrade_status_unavailable(),
        &commands::metrics::ConfigMetrics::Invalid,
    );
    metric_lines_are_well_formed(&stopped);
    assert!(stopped.contains("wayexpand_daemon_up 0"));
    assert!(stopped.contains("wayexpand_daemon_build_matches 0"));
    assert!(!stopped.contains("wayexpand_daemon_route_info"));
}
