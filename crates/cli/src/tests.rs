use super::*;
use crate::doctor::backends::{
    automatic_selection_is_ready, capture_path_available, capture_readiness, display_session_flags,
};
use crate::doctor::certification::certification_selection_status;
use crate::doctor::files::existing_control_socket_is_healthy;
use crate::doctor::policy::{absolute_command_policy_diagnostic, print_policy_diagnostics_json};
use crate::doctor::status::{runtime_capabilities_from_status, status_schema_compatible};
use std::time::{SystemTime, UNIX_EPOCH};

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
        absolute_command_policy_diagnostic(&OrganizationPolicy {
            safe_mode: false,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        }),
        Some("Require absolute command paths: audit only")
    );
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
        "running\nsource=stdin\nstatus_schema=2\npaused=true\ncommand_queue_depth=3\nconfig_state=ok\ncapture_sensitive_focus=false\nwindow_tracker_connected=true\ninject_full_unicode=true\n",
    )
    .unwrap();
    assert_eq!(value["response"], "running");
    assert_eq!(value["source"], "stdin");
    assert_eq!(value["status_schema"], 2);
    assert_eq!(value["paused"], true);
    assert_eq!(value["command_queue_depth"], 3);
    assert_eq!(value["config_state"], "ok");
    assert_eq!(value["capture_sensitive_focus"], false);
    assert_eq!(value["window_tracker_connected"], true);
    assert_eq!(value["inject_full_unicode"], true);
}

#[test]
fn status_schema_rejects_missing_or_newer_incompatible_daemons() {
    assert!(status_schema_compatible(
        &serde_json::json!({ "status_schema": 2 })
    ));
    assert!(!status_schema_compatible(&serde_json::json!({})));
    assert!(!status_schema_compatible(
        &serde_json::json!({ "status_schema": 3 })
    ));
    assert!(!status_schema_compatible(
        &serde_json::json!({ "status_schema": "1" })
    ));
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
         backend_mode=unknown\n\
         status_schema=2\n\
         state=connected\n\
         paused=false\n\
         config=/home/user/.config/wayexpand/expansions.toml\n\
         config_state=ok\n\
         capture_sensitive_focus=true\n\
         capture_exclusive=true\n\
         capture_reliable_key_state=true\n\
         capture_key_passthrough=false\n\
         capture_composition_aware=false\n\
         window_tracker_connected=false\n\
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
         injection_latency_p50_us=0\n\
         injection_latency_p95_us=0\n\
         injection_latency_p99_us=0";
    let value = status_as_json(daemon_response).unwrap();
    let object = value.as_object().expect("status --json returns an object");
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/contracts/status-json.json"))
            .expect("status contract fixture must be valid JSON");
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
            "action_broker",
            "policy",
            "backends",
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
            "window_tracker_connected",
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
            "injection_latency_p50_us",
            "injection_latency_p95_us",
            "injection_latency_p99_us",
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
        allowed_backends: vec!["input-method-v2".into()],
        ..Default::default()
    };
    assert_eq!(
        setup_backend_for_mode("experimental", &capabilities, &input_method_only).unwrap(),
        "input-method"
    );
    assert!(setup_backend_for_mode("maximum", &capabilities, &input_method_only).is_err());

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
        allowed_backends: vec!["libei".into()],
        ..Default::default()
    };
    assert!(setup_backend_for_mode("experimental", &capabilities, &raw_only).is_err());
    assert_eq!(
        setup_backend_for_mode("maximum", &capabilities, &raw_only).unwrap(),
        "evdev"
    );

    let wlroots_only = OrganizationPolicy {
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
