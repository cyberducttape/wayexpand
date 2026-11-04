//! Effective runtime sandbox inspection for `wayexpand doctor --security`.

use super::{
    broker::broker_diagnostics,
    status::{read_daemon_status, status_as_json},
};
use crate::*;
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProtectionProfile {
    Workstation,
    Hardened,
    Strict,
}

impl ProtectionProfile {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "workstation" => Ok(Self::Workstation),
            "hardened" => Ok(Self::Hardened),
            "strict" => Ok(Self::Strict),
            _ => bail!(
                "unknown security profile {value:?}; expected workstation, hardened, or strict"
            ),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Workstation => "workstation",
            Self::Hardened => "hardened",
            Self::Strict => "strict",
        }
    }
}

pub(crate) fn run(config_path: &Path, json: bool, profile: ProtectionProfile) -> Result<()> {
    let config = Config::load(config_path);
    let config_permissions = config_permission_report(config_path, config.is_ok());
    let broker = broker_diagnostics(config.as_ref().ok());
    let daemon = read_daemon_status()
        .and_then(|response| status_as_json(&response))
        .unwrap_or(serde_json::Value::Null);
    let sandbox = daemon
        .get("sandbox_security")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let broker_sandbox = broker
        .get("effective_security")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let capture_source = daemon["source"].as_str().unwrap_or("unknown");
    let sensitive_field_detection = daemon["capture_sensitive_focus"].as_bool();
    let input_privacy_readiness =
        privacy_readiness(profile, capture_source, sensitive_field_detection);
    let input_privacy_required = profile == ProtectionProfile::Strict;

    let local_mount_ns = namespace_inode("mnt");
    let local_uts_ns = namespace_inode("uts");
    let local_tmp_mount = mount_id_for(Path::new("/tmp"));
    let mut checks = vec![
        check(
            "daemon_connected",
            !sandbox.is_null(),
            "The live daemon must provide its in-process security probes.",
        ),
        daemon_status_schema_check(&daemon),
        check(
            "config_permissions",
            config_permissions["safe"].as_bool(),
            "The configuration must be a valid regular file owned by this user or root and not group/world writable; Config loading also validates its ancestors.",
        ),
        check(
            "daemon_mount_namespace",
            sandbox["namespaces"]["mount"]
                .as_u64()
                .zip(local_mount_ns)
                .map(|(daemon_ns, local_ns)| daemon_ns != local_ns),
            "The running daemon must be in a mount namespace distinct from the doctor process.",
        ),
        check(
            "daemon_private_tmp",
            sandbox["tmp_mount_id"]
                .as_u64()
                .zip(local_tmp_mount)
                .map(|(daemon_mount, local_mount)| daemon_mount != local_mount),
            "The daemon and desktop session must see distinct /tmp mounts.",
        ),
        check(
            "daemon_uts_namespace",
            sandbox["namespaces"]["uts"]
                .as_u64()
                .zip(local_uts_ns)
                .map(|(daemon_ns, local_ns)| daemon_ns != local_ns),
            "ProtectHostname should place the daemon in a distinct UTS namespace.",
        ),
        check(
            "daemon_no_new_privileges",
            sandbox["no_new_privileges"].as_bool(),
            "The kernel must report NoNewPrivs=1 for the daemon process.",
        ),
        check(
            "daemon_seccomp",
            sandbox["seccomp_mode"].as_u64().map(|mode| mode == 2),
            "The kernel must report seccomp filter mode for the daemon process.",
        ),
        check(
            "daemon_af_unix_allowed",
            state(&sandbox, &["address_families", "unix"]).map(|value| value == "allowed"),
            "The daemon needs AF_UNIX for its control and portal sockets.",
        ),
        check(
            "daemon_af_inet_blocked",
            state(&sandbox, &["address_families", "inet"]).map(|value| value == "blocked"),
            "The daemon must be unable to create AF_INET sockets.",
        ),
        check(
            "daemon_af_inet6_blocked",
            state(&sandbox, &["address_families", "inet6"]).map(|value| value == "blocked"),
            "The daemon must be unable to create AF_INET6 sockets.",
        ),
        check(
            "daemon_home_read_only",
            sandbox["home_read_only"].as_bool(),
            "The daemon's home mount must be read-only outside explicit unit exceptions.",
        ),
        check(
            "daemon_system_read_only",
            sandbox["system_read_only"].as_bool(),
            "The daemon's /usr mount must be read-only under ProtectSystem=strict.",
        ),
        check(
            "daemon_kernel_tunables_read_only",
            sandbox["kernel_tunables_read_only"].as_bool(),
            "The daemon must not have a writable /proc/sys mount.",
        ),
        check(
            "daemon_proc_subset_pid",
            sandbox["proc_subset_pid"].as_bool(),
            "The daemon's proc mount should expose only process-related entries.",
        ),
        check(
            "daemon_proc_hidepid",
            sandbox["proc_hidepid_invisible"].as_bool(),
            "ProtectProc=invisible should be effective on the daemon proc mount.",
        ),
        check(
            "daemon_memory_deny_write_execute",
            state(&sandbox, &["memory_write_execute_blocked"]).map(|value| value == "blocked"),
            "A direct anonymous RWX mapping attempt must be refused.",
        ),
        check(
            "daemon_restrict_namespaces",
            state(&sandbox, &["namespace_creation"]).map(|value| value == "blocked"),
            "A child-process probe must be unable to create an unprivileged user namespace.",
        ),
        check(
            "daemon_lock_personality",
            state(&sandbox, &["personality_syscall"]).map(|value| value == "blocked"),
            "The personality syscall must be blocked by LockPersonality.",
        ),
        check(
            "daemon_restrict_suid_sgid",
            state(&sandbox, &["setuid_mode_bit"]).map(|value| value == "blocked"),
            "Setting a setuid bit on an ephemeral memfd must be refused.",
        ),
        check(
            "daemon_core_limit",
            sandbox["max_core_file_size"][1].as_u64().map(|limit| limit == 0),
            "The hard core-file limit must be zero.",
        ),
        check(
            "daemon_open_file_limit",
            sandbox["max_open_files"][1]
                .as_u64()
                .map(|limit| limit <= 64),
            "The hard open-file limit must not exceed the service's LimitNOFILE=64.",
        ),
        check(
            "daemon_memory_limit",
            sandbox["cgroup_memory_max_bytes"]
                .as_u64()
                .map(|limit| limit <= 256 * 1024 * 1024),
            "The effective cgroup memory.max must not exceed MemoryMax=256M.",
        ),
        check(
            "daemon_task_limit",
            sandbox["cgroup_pids_max"]
                .as_u64()
                .map(|limit| limit <= 32),
            "The effective cgroup pids.max must not exceed TasksMax=32.",
        ),
        check(
            "daemon_command_cgroup_containment",
            daemon["command_containment"]
                .as_str()
                .zip(daemon["command_containment_self_test"].as_str())
                .map(|(mode, result)| mode == "cgroup" && result == "passed"),
            "The daemon must require cgroup containment and pass its harmless descendant-cancellation self-test in its own service context.",
        ),
    ];
    checks.push(input_privacy_check(
        profile,
        capture_source,
        sensitive_field_detection,
    ));

    let broker_required = broker["required"].as_bool() == Some(true);
    let command_execution_configured = config.as_ref().is_ok_and(|config| {
        config
            .expansion
            .iter()
            .any(|expansion| expansion.command.is_some())
            || !config.hotkey.is_empty()
    });
    let broker_running = broker["service"]["active"].as_bool() == Some(true)
        && broker["socket"]["valid"].as_bool() == Some(true);
    checks.push(action_broker_availability_check(&broker, broker_required));
    checks.push(target_integrity_check(&daemon));
    if broker_required && broker_running {
        let broker_mount = broker_sandbox["namespaces"]["mount"].as_u64();
        let broker_uts = broker_sandbox["namespaces"]["uts"].as_u64();
        checks.extend([
            check(
                "broker_command_cgroup_containment",
                broker["command_containment_self_test"].as_str().map(|result| result == "passed"),
                "The running Action Broker must pass the descendant-cancellation self-test in its own service context.",
            ),
            check(
                "broker_seccomp",
                broker_sandbox["seccomp_mode"].as_u64().map(|mode| mode == 2),
                "The running broker must report an active seccomp filter.",
            ),
            check(
                "broker_no_new_privileges",
                broker_sandbox["no_new_privileges"].as_bool(),
                "The running broker must report NoNewPrivs=1.",
            ),
            check(
                "broker_af_unix_allowed",
                state(&broker_sandbox, &["address_families", "unix"]).map(|value| value == "allowed"),
                "The broker requires AF_UNIX for its IPC socket.",
            ),
            check(
                "broker_af_inet_blocked",
                state(&broker_sandbox, &["address_families", "inet"]).map(|value| value == "blocked"),
                "The broker must not be able to create AF_INET sockets.",
            ),
            check(
                "broker_af_inet6_blocked",
                state(&broker_sandbox, &["address_families", "inet6"]).map(|value| value == "blocked"),
                "The broker must not be able to create AF_INET6 sockets.",
            ),
            check(
                "broker_mount_namespace",
                broker_mount.zip(local_mount_ns).map(|(broker_ns, local_ns)| broker_ns != local_ns),
                "The running broker should have an isolated mount namespace.",
            ),
            check(
                "broker_uts_namespace",
                broker_uts.zip(local_uts_ns).map(|(broker_ns, local_ns)| broker_ns != local_ns),
                "The running broker should have an isolated UTS namespace.",
            ),
        ]);
    }

    // These restrictions cannot be established from /proc status alone. Keep
    // the limits explicit instead of treating configured unit text as proof.
    let unverified = unavailable_systemd_evidence();
    let required = required_checks(profile);
    for check in &mut checks {
        let name = check["name"].as_str().unwrap_or_default();
        let gate = gate_for_check(name);
        let broker_check_required = name != "action_broker_availability" || broker_required;
        check["required"] = serde_json::Value::Bool(
            (profile == ProtectionProfile::Strict || required.contains(&name))
                && broker_check_required
                && (name != "daemon_command_cgroup_containment"
                    || profile != ProtectionProfile::Workstation
                    || command_execution_configured),
        );
        check["gate"] = serde_json::Value::String(gate.to_owned());
    }
    let unavailable_checks = checks
        .iter()
        .filter_map(|check| {
            (check["status"] == "unavailable")
                .then_some(check["name"].as_str().unwrap_or("unknown").to_owned())
        })
        .collect::<Vec<_>>();
    let verified_failures = checks
        .iter()
        .filter_map(|check| {
            (check["status"] == "fail")
                .then_some(check["name"].as_str().unwrap_or("unknown").to_owned())
        })
        .collect::<Vec<_>>();
    let evdev_privacy = capture_source == "evdev";
    let execution_sandbox_checks = checks_for_gate(&checks, "execution_sandbox");
    let configuration_integrity_checks = checks_for_gate(&checks, "configuration_integrity");
    let action_broker_checks = checks_for_gate(&checks, "action_broker_availability");
    let target_integrity_checks = checks_for_gate(&checks, "target_integrity");
    let input_privacy_checks = checks_for_gate(&checks, "input_privacy");
    let execution_unverified = unverified
        .iter()
        .copied()
        .filter(|evidence| broker_required || !evidence.starts_with("broker "))
        .collect::<Vec<_>>();
    let execution_sandbox_required = gate_is_required(&execution_sandbox_checks);
    let configuration_integrity_required = gate_is_required(&configuration_integrity_checks);
    let action_broker_required = broker_required && gate_is_required(&action_broker_checks);
    let target_integrity_required = gate_is_required(&target_integrity_checks);
    let execution_sandbox_readiness =
        readiness_for(profile, &execution_sandbox_checks, &execution_unverified);
    let configuration_integrity_readiness =
        readiness_for(profile, &configuration_integrity_checks, &[]);
    let action_broker_availability_readiness = if broker_required {
        readiness_for(profile, &action_broker_checks, &[])
    } else {
        "not_required"
    };
    let target_integrity_readiness = readiness_for(profile, &target_integrity_checks, &[]);
    let readiness = combine_gate_readiness(&[
        execution_sandbox_readiness,
        input_privacy_readiness,
        configuration_integrity_readiness,
        action_broker_availability_readiness,
        target_integrity_readiness,
    ]);
    let healthy = readiness == "pass";
    let route = active_route_summary(&daemon);
    let report = serde_json::json!({
        "schema": 7,
        "profile": profile.name(),
        "readiness": readiness,
        "execution_sandbox_readiness": execution_sandbox_readiness,
        "sandbox_readiness": execution_sandbox_readiness,
        "input_privacy_readiness": input_privacy_readiness,
        "configuration_integrity_readiness": configuration_integrity_readiness,
        "action_broker_availability_readiness": action_broker_availability_readiness,
        "target_integrity_readiness": target_integrity_readiness,
        "gates": {
            "execution_sandbox": gate_report(execution_sandbox_readiness, execution_sandbox_required, &execution_sandbox_checks, &execution_unverified),
            "input_privacy": gate_report(input_privacy_readiness, input_privacy_required, &input_privacy_checks, &[]),
            "configuration_integrity": gate_report(configuration_integrity_readiness, configuration_integrity_required, &configuration_integrity_checks, &[]),
            "action_broker_availability": gate_report(action_broker_availability_readiness, action_broker_required, &action_broker_checks, &[]),
            "target_integrity": gate_report(target_integrity_readiness, target_integrity_required, &target_integrity_checks, &[]),
        },
        "healthy": healthy,
        "daemon_pid": sandbox.get("pid"),
        "daemon_runtime": {
            "status_schema": daemon.get("status_schema"),
            "expected_status_schema": CONTROL_STATUS_SCHEMA,
            "schema_compatible": daemon.get("status_schema").and_then(serde_json::Value::as_u64) == Some(u64::from(CONTROL_STATUS_SCHEMA)),
            "daemon_commit": daemon.get("daemon_commit"),
        },
        "daemon": sandbox,
        "active_route": route,
        "input_privacy": {
            "source": capture_source,
            "sensitive_field_detection": sensitive_field_detection,
            "readiness": input_privacy_readiness,
            "required": profile == ProtectionProfile::Strict,
            "reason": if input_privacy_readiness == "pass" {
                "active capture route reports sensitive-field detection"
            } else if evdev_privacy {
                "evdev captures globally and cannot identify sensitive fields"
            } else if capture_source == "unknown" {
                "active capture source is unknown, so sensitive-field protection cannot be verified"
            } else if sensitive_field_detection == Some(false) {
                "active capture route cannot identify sensitive fields"
            } else {
                "active capture route did not report sensitive-field detection"
            },
            "evdev_limitation": evdev_privacy.then_some("evdev cannot identify password or other sensitive fields; keyboard input may be observed while capture is active"),
        },
        "configuration": config_permissions,
        "command_execution_configured": command_execution_configured,
        "command_containment": {
            "mode": daemon.get("command_containment"),
            "policy": daemon.get("command_containment_mode"),
            "self_test": daemon.get("command_containment_self_test"),
            "cleanup_pending_jobs": daemon.get("command_cleanup_pending_jobs"),
            "cleanup_degraded": daemon.get("command_cleanup_degraded"),
        },
        "action_broker": {
            "required": broker_required,
            "available": broker_running,
            "readiness": action_broker_availability_readiness,
            "runtime": broker_sandbox,
            "network_policy_note": "AF_INET/AF_INET6 socket creation is probed in the live broker; IPAddressDeny BPF is listed as unverified because its attached filter cannot be queried without elevated inspection privileges."
        },
        "checks": checks,
        "unverified_mechanisms": unverified,
        "unavailable_evidence": unverified,
        "unavailable_checks": unavailable_checks,
        "verified_failures": verified_failures,
        "evidence_readiness": if unverified.is_empty() && unavailable_checks.is_empty() { "complete" } else { "incomplete" },
    });

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("WayExpand effective security report");
        println!("Protection profile: {}", profile.name());
        println!("Readiness: {}", readiness.to_ascii_uppercase());
        println!(
            "Execution sandbox readiness: {}",
            execution_sandbox_readiness.to_ascii_uppercase()
        );
        println!(
            "Input privacy readiness: {}",
            input_privacy_readiness.to_ascii_uppercase()
        );
        println!(
            "Configuration integrity readiness: {}",
            configuration_integrity_readiness.to_ascii_uppercase()
        );
        println!(
            "Action Broker availability readiness: {}",
            action_broker_availability_readiness.to_ascii_uppercase()
        );
        println!(
            "Target integrity readiness: {}",
            target_integrity_readiness.to_ascii_uppercase()
        );
        println!("Daemon PID: {}", report["daemon_pid"]);
        println!(
            "Active route: source={}, backend={}, atomic_replace={}, replacement_assurance={}, compatibility_mode={}, focus_assurance={}",
            route["source"].as_str().unwrap_or("unknown"),
            route["backend"].as_str().unwrap_or("unknown"),
            route["atomic_replace"].as_bool().unwrap_or(false),
            route["replacement_assurance"].as_str().unwrap_or("unknown"),
            route["compatibility_mode"].as_str().unwrap_or("unknown"),
            route["focus_assurance"].as_str().unwrap_or("unknown"),
        );
        println!(
            "Automatic replacement mode: {}",
            route["automatic_replacement_mode"]
                .as_str()
                .unwrap_or("unavailable")
        );
        if evdev_privacy {
            println!("Privacy: evdev cannot identify password or other sensitive fields; keyboard input may be observed while capture is active.");
        }
        println!(
            "Command containment: mode={}, policy={}, self_test={}, pending_cleanup_jobs={}, degraded={}",
            daemon["command_containment"].as_str().unwrap_or("unknown"),
            daemon["command_containment_mode"].as_str().unwrap_or("unknown"),
            daemon["command_containment_self_test"].as_str().unwrap_or("unknown"),
            daemon["command_cleanup_pending_jobs"].as_u64().unwrap_or(0),
            daemon["command_cleanup_degraded"].as_bool().unwrap_or(true),
        );
        for check in report["checks"].as_array().into_iter().flatten() {
            println!(
                "{}: {} ({}) — {}",
                check["name"].as_str().unwrap_or("unknown"),
                check["status"].as_str().unwrap_or("unknown"),
                if check["required"] == true {
                    "required"
                } else {
                    "optional"
                },
                check["detail"].as_str().unwrap_or("")
            );
        }
        println!(
            "Action Broker: required={}, active={}, socket_valid={}",
            broker_required, broker["service"]["active"], broker["socket"]["valid"]
        );
        println!(
            "Verification evidence: {}",
            if unverified.is_empty() && unavailable_checks.is_empty() {
                "COMPLETE"
            } else {
                "INCOMPLETE"
            }
        );
        println!("Unavailable evidence (not a verified failure):");
        for mechanism in unverified {
            println!("  - {mechanism}");
        }
        for name in &unavailable_checks {
            println!("  - check {name}: probe data unavailable");
        }
        println!("Overall: {}", readiness.to_ascii_uppercase());
    }
    if healthy {
        Ok(())
    } else {
        bail!(
            "security readiness is {readiness} for profile {}; inspect the security report",
            profile.name()
        )
    }
}

fn active_route_summary(daemon: &serde_json::Value) -> serde_json::Value {
    let atomic = daemon["inject_atomic_replace"].as_bool();
    let compatibility_mode = daemon["compatibility_mode"].as_str();
    let automatic_replacement_mode = match (atomic, compatibility_mode) {
        (Some(true), _) => "atomic-capability-reported",
        (Some(false), Some("off")) => "refused-by-default",
        (Some(false), Some(_)) => "experimental-compatibility-override",
        _ => "unknown",
    };
    serde_json::json!({
        "source": daemon.get("source").cloned().unwrap_or(serde_json::Value::Null),
        "backend": daemon.get("backend").cloned().unwrap_or(serde_json::Value::Null),
        "route_id": daemon.get("route_id").cloned().unwrap_or(serde_json::Value::Null),
        "atomic_replace": atomic,
        "replacement_assurance": daemon.get("replacement_assurance").cloned().unwrap_or(serde_json::Value::Null),
        "compatibility_mode": daemon.get("compatibility_mode").cloned().unwrap_or(serde_json::Value::Null),
        "focus_assurance": daemon.get("focus_assurance").cloned().unwrap_or(serde_json::Value::Null),
        "sensitive_field_detection": daemon.get("capture_sensitive_focus").cloned().unwrap_or(serde_json::Value::Null),
        "automatic_replacement_mode": automatic_replacement_mode,
    })
}

fn daemon_status_schema_check(daemon: &serde_json::Value) -> serde_json::Value {
    check(
        "daemon_status_schema",
        daemon
            .get("status_schema")
            .and_then(serde_json::Value::as_u64)
            .map(|schema| schema == u64::from(CONTROL_STATUS_SCHEMA)),
        "The running daemon must expose the current status contract so security checks cannot pass using stale or incomplete runtime evidence.",
    )
}

fn target_integrity_check(daemon: &serde_json::Value) -> serde_json::Value {
    let automatic_mode = daemon["inject_atomic_replace"]
        .as_bool()
        .zip(daemon["compatibility_mode"].as_str());
    let route_certified_atomic = daemon["route_id"]
        .as_str()
        .and_then(wayexpand_backend_selection::route_contract_by_id)
        .is_some_and(|route| route.is_certified() && route.atomic_replace);
    let result = match automatic_mode {
        // The trigger is retained and destructive automatic replacement is
        // refused by default on this route.
        Some((false, "off")) => Some(true),
        // Route certification and the live atomic capability are both
        // required before automatic replacement can be considered safe.
        Some((true, _)) if route_certified_atomic => Some(true),
        // Compatibility mode explicitly admits non-atomic replacement.
        Some((false, "tracked-focus" | "untracked-focus")) => Some(false),
        // An atomic capability without a certified desktop route is not
        // enough evidence to pass target-integrity readiness.
        Some((true, _)) => None,
        _ => None,
    };
    check(
        "automatic_replacement_target_integrity",
        result,
        if result == Some(true) {
            "Automatic replacement is either refused by default or backed by a certified atomic route."
        } else if result == Some(false) {
            "An explicit compatibility override enables a non-atomic replacement route that has not passed the real-session target-integrity gate."
        } else {
            "The active route lacks sufficient certification evidence to establish safe automatic replacement."
        },
    )
}

fn gate_for_check(name: &str) -> &'static str {
    match name {
        "config_permissions" => "configuration_integrity",
        "capture_sensitive_focus" => "input_privacy",
        "action_broker_availability" => "action_broker_availability",
        "automatic_replacement_target_integrity" => "target_integrity",
        // Broker confinement is still execution sandboxing. Keep it distinct
        // from whether the optional broker service/socket is available.
        "broker_command_cgroup_containment"
        | "broker_seccomp"
        | "broker_no_new_privileges"
        | "broker_af_unix_allowed"
        | "broker_af_inet_blocked"
        | "broker_af_inet6_blocked"
        | "broker_mount_namespace"
        | "broker_uts_namespace" => "execution_sandbox",
        _ => "execution_sandbox",
    }
}

fn input_privacy_check(
    profile: ProtectionProfile,
    source: &str,
    sensitive_field_detection: Option<bool>,
) -> serde_json::Value {
    let readiness = privacy_readiness(profile, source, sensitive_field_detection);
    let status = match readiness {
        "pass" => "pass",
        "fail" => "fail",
        "incomplete" => "unavailable",
        _ => "degraded",
    };
    let detail = if source == "evdev" {
        "The active evdev source captures globally and cannot identify sensitive fields."
    } else if source == "unknown" {
        "The active capture source is unknown; sensitive-field protection cannot be verified."
    } else if sensitive_field_detection == Some(true) {
        "The active capture route reports sensitive-field detection."
    } else if sensitive_field_detection == Some(false) {
        "The active capture route cannot identify sensitive fields."
    } else {
        "The active capture route did not report whether it identifies sensitive fields."
    };
    serde_json::json!({
        "name": "capture_sensitive_focus",
        "status": status,
        "detail": detail,
        "required": profile == ProtectionProfile::Strict,
        "gate": "input_privacy",
    })
}

fn action_broker_availability_check(
    broker: &serde_json::Value,
    required: bool,
) -> serde_json::Value {
    let available = broker["service"]["active"]
        .as_bool()
        .zip(broker["socket"]["valid"].as_bool())
        .map(|(active, socket_valid)| active && socket_valid);
    check(
        "action_broker_availability",
        available,
        if required {
            "A configured named action requires a healthy, active Action Broker."
        } else {
            "No named actions require the broker; actual availability is informational."
        },
    )
}

fn checks_for_gate(checks: &[serde_json::Value], gate: &str) -> Vec<serde_json::Value> {
    checks
        .iter()
        .filter(|check| check["gate"] == gate)
        .cloned()
        .collect()
}

fn gate_is_required(checks: &[serde_json::Value]) -> bool {
    checks.iter().any(|check| check["required"] == true)
}

fn gate_report(
    readiness: &str,
    required: bool,
    checks: &[serde_json::Value],
    unavailable_evidence: &[&str],
) -> serde_json::Value {
    let names = checks
        .iter()
        .filter_map(|check| check["name"].as_str())
        .collect::<Vec<_>>();
    let unavailable = checks
        .iter()
        .filter_map(|check| {
            (check["status"] == "unavailable")
                .then_some(check["name"].as_str().unwrap_or("unknown"))
        })
        .collect::<Vec<_>>();
    let failures = checks
        .iter()
        .filter_map(|check| {
            (check["status"] == "fail").then_some(check["name"].as_str().unwrap_or("unknown"))
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "readiness": readiness,
        "required": required,
        "applicable": readiness != "not_required",
        "checks": names,
        "unavailable_checks": unavailable,
        "unavailable_evidence": unavailable_evidence,
        "verified_failures": failures,
    })
}

fn combine_gate_readiness(gates: &[&str]) -> &'static str {
    if gates.contains(&"fail") {
        "fail"
    } else if gates.contains(&"incomplete") {
        "incomplete"
    } else if gates.contains(&"degraded") {
        "degraded"
    } else if gates
        .iter()
        .any(|gate| !matches!(*gate, "pass" | "not_required"))
    {
        "incomplete"
    } else {
        "pass"
    }
}

fn required_checks(profile: ProtectionProfile) -> &'static [&'static str] {
    const WORKSTATION: &[&str] = &[
        "daemon_connected",
        "daemon_status_schema",
        "automatic_replacement_target_integrity",
        "config_permissions",
        "daemon_no_new_privileges",
        "daemon_seccomp",
        "daemon_af_unix_allowed",
        "daemon_af_inet_blocked",
        "daemon_af_inet6_blocked",
        "daemon_memory_deny_write_execute",
        "daemon_command_cgroup_containment",
        "broker_command_cgroup_containment",
        "action_broker_availability",
    ];
    const HARDENED: &[&str] = &[
        "daemon_connected",
        "daemon_status_schema",
        "automatic_replacement_target_integrity",
        "config_permissions",
        "daemon_mount_namespace",
        "daemon_private_tmp",
        "daemon_uts_namespace",
        "daemon_no_new_privileges",
        "daemon_seccomp",
        "daemon_af_unix_allowed",
        "daemon_af_inet_blocked",
        "daemon_af_inet6_blocked",
        "daemon_home_read_only",
        "daemon_system_read_only",
        "daemon_kernel_tunables_read_only",
        "daemon_proc_subset_pid",
        "daemon_proc_hidepid",
        "daemon_memory_deny_write_execute",
        "daemon_restrict_namespaces",
        "daemon_lock_personality",
        "daemon_restrict_suid_sgid",
        "daemon_core_limit",
        "daemon_open_file_limit",
        "daemon_memory_limit",
        "daemon_task_limit",
        "daemon_command_cgroup_containment",
        "broker_command_cgroup_containment",
        "action_broker_availability",
        "broker_seccomp",
        "broker_no_new_privileges",
        "broker_af_unix_allowed",
        "broker_af_inet_blocked",
        "broker_af_inet6_blocked",
        "broker_mount_namespace",
        "broker_uts_namespace",
    ];
    match profile {
        ProtectionProfile::Workstation => WORKSTATION,
        ProtectionProfile::Hardened | ProtectionProfile::Strict => HARDENED,
    }
}

fn readiness_for(
    profile: ProtectionProfile,
    checks: &[serde_json::Value],
    unverified: &[&str],
) -> &'static str {
    let required_failure = checks
        .iter()
        .any(|check| check["required"] == true && check["status"] == "fail");
    let required_unavailable = checks
        .iter()
        .any(|check| check["required"] == true && check["status"] == "unavailable");
    let optional_failure = checks
        .iter()
        .any(|check| check["required"] != true && check["status"] != "pass");
    let incomplete =
        required_unavailable || (profile == ProtectionProfile::Strict && !unverified.is_empty());
    let degraded =
        optional_failure || (profile == ProtectionProfile::Hardened && !unverified.is_empty());
    if required_failure {
        "fail"
    } else if incomplete {
        "incomplete"
    } else if degraded {
        "degraded"
    } else {
        "pass"
    }
}

fn unavailable_systemd_evidence() -> Vec<&'static str> {
    vec![
        "SystemCallArchitectures=native (the kernel does not expose the installed rule set)",
        "RestrictRealtime=yes (the installed seccomp rule is not introspectable here)",
        "UMask=0077 (the active umask cannot be queried without changing it)",
        "broker IPAddressDeny=any (the address-family probe verifies socket creation, not cgroup BPF policy)",
    ]
}

fn privacy_readiness(
    profile: ProtectionProfile,
    source: &str,
    sensitive_field_detection: Option<bool>,
) -> &'static str {
    // Raw evdev capture cannot identify sensitive fields. Treat the known
    // source limitation as authoritative even if a future or inconsistent
    // status payload claims otherwise; strict privacy must fail closed.
    if source == "evdev" {
        return if profile == ProtectionProfile::Strict {
            "fail"
        } else {
            "degraded"
        };
    }
    match sensitive_field_detection {
        Some(true) if source != "unknown" => "pass",
        Some(false) if profile == ProtectionProfile::Strict => "fail",
        Some(false) => "degraded",
        Some(true) | None if source == "unknown" || sensitive_field_detection.is_none() => {
            if profile == ProtectionProfile::Strict {
                "incomplete"
            } else {
                "degraded"
            }
        }
        Some(true) => "degraded",
        None => "degraded",
    }
}

fn state<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(*key))
        .and_then(serde_json::Value::as_str)
}

fn check(name: &str, passed: impl Into<Option<bool>>, detail: &str) -> serde_json::Value {
    let status = match passed.into() {
        Some(true) => "pass",
        Some(false) => "fail",
        None => "unavailable",
    };
    serde_json::json!({
        "name": name,
        "status": status,
        "detail": detail,
    })
}

fn config_permission_report(path: &Path, parsed: bool) -> serde_json::Value {
    let metadata = fs::metadata(path).ok();
    let mode = metadata.as_ref().map(|metadata| metadata.mode() & 0o777);
    let uid = metadata.as_ref().map(MetadataExt::uid);
    let current_uid = rustix::process::geteuid().as_raw();
    let safe = metadata.as_ref().is_some_and(|metadata| {
        metadata.file_type().is_file()
            && (metadata.uid() == current_uid || metadata.uid() == 0)
            && metadata.mode() & 0o022 == 0
    }) && parsed;
    serde_json::json!({
        "path": path.display().to_string(),
        "regular_file": metadata.as_ref().is_some_and(|metadata| metadata.file_type().is_file()),
        "mode_octal": mode.map(|mode| format!("{:04o}", mode)),
        "owner_uid": uid,
        "current_uid": current_uid,
        "valid_config": parsed,
        "safe": safe,
    })
}

fn namespace_inode(name: &str) -> Option<u64> {
    fs::metadata(format!("/proc/self/ns/{name}"))
        .ok()
        .map(|metadata| metadata.ino())
}

fn mount_id_for(path: &Path) -> Option<u64> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").ok()?;
    let target = path.to_str()?;
    mountinfo.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let id = fields.next()?.parse().ok()?;
        fields.next()?;
        fields.next()?;
        fields.next()?;
        (fields.next()? == target).then_some(id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_profiles_are_explicit_and_validated() {
        assert_eq!(
            ProtectionProfile::parse("hardened").unwrap(),
            ProtectionProfile::Hardened
        );
        assert!(ProtectionProfile::parse("guess").is_err());
        assert!(required_checks(ProtectionProfile::Hardened).contains(&"daemon_mount_namespace"));
        assert!(required_checks(ProtectionProfile::Workstation)
            .contains(&"broker_command_cgroup_containment"));
        assert!(required_checks(ProtectionProfile::Workstation).contains(&"daemon_status_schema"));
        assert!(required_checks(ProtectionProfile::Workstation)
            .contains(&"automatic_replacement_target_integrity"));
        assert!(
            !required_checks(ProtectionProfile::Workstation).contains(&"daemon_mount_namespace")
        );
    }

    #[test]
    fn daemon_schema_gate_rejects_stale_and_missing_runtime_contracts() {
        let current = serde_json::json!({"status_schema": CONTROL_STATUS_SCHEMA});
        let stale = serde_json::json!({"status_schema": CONTROL_STATUS_SCHEMA - 1});
        let missing = serde_json::json!({});
        assert_eq!(daemon_status_schema_check(&current)["status"], "pass");
        assert_eq!(daemon_status_schema_check(&stale)["status"], "fail");
        assert_eq!(
            daemon_status_schema_check(&missing)["status"],
            "unavailable"
        );
    }

    #[test]
    fn target_integrity_fails_open_non_atomic_compatibility_overrides() {
        let refused = serde_json::json!({
            "route_id": "kde-evdev-libei",
            "inject_atomic_replace": false,
            "compatibility_mode": "off"
        });
        let override_enabled = serde_json::json!({
            "route_id": "kde-evdev-libei",
            "inject_atomic_replace": false,
            "compatibility_mode": "untracked-focus"
        });
        let uncertified_atomic = serde_json::json!({
            "route_id": "unlisted-atomic-route",
            "inject_atomic_replace": true,
            "compatibility_mode": "off"
        });
        assert_eq!(target_integrity_check(&refused)["status"], "pass");
        assert_eq!(target_integrity_check(&override_enabled)["status"], "fail");
        assert_eq!(
            target_integrity_check(&uncertified_atomic)["status"],
            "unavailable"
        );
        assert_eq!(
            gate_for_check("automatic_replacement_target_integrity"),
            "target_integrity"
        );
    }

    #[test]
    fn readiness_separates_verified_failures_from_missing_evidence() {
        let passing = [serde_json::json!({"required": true, "status": "pass"})];
        let unverified = ["UMask=0077 is not introspectable"];
        assert_eq!(
            readiness_for(ProtectionProfile::Hardened, &passing, &unverified),
            "degraded"
        );
        assert_eq!(
            readiness_for(ProtectionProfile::Strict, &passing, &unverified),
            "incomplete"
        );
        let failed = [serde_json::json!({"required": true, "status": "fail"})];
        assert_eq!(
            readiness_for(ProtectionProfile::Workstation, &failed, &[]),
            "fail"
        );
        let optional_failure = [serde_json::json!({"required": false, "status": "fail"})];
        assert_eq!(
            readiness_for(ProtectionProfile::Workstation, &optional_failure, &[]),
            "degraded"
        );
        assert_eq!(
            readiness_for(ProtectionProfile::Strict, &failed, &unverified),
            "fail",
            "an observed required-control failure remains distinct from missing evidence"
        );
        let unavailable = [serde_json::json!({"required": true, "status": "unavailable"})];
        assert_eq!(
            readiness_for(ProtectionProfile::Strict, &unavailable, &[]),
            "incomplete"
        );
        assert_eq!(
            combine_gate_readiness(&["incomplete", "pass", "not_required"]),
            "incomplete"
        );
    }

    #[test]
    fn sandbox_and_input_privacy_readiness_are_independent_and_strict_rejects_evdev() {
        let sandbox = "pass";
        let workstation_privacy =
            privacy_readiness(ProtectionProfile::Workstation, "evdev", Some(false));
        assert_eq!(sandbox, "pass");
        assert_eq!(workstation_privacy, "degraded");
        assert_eq!(
            combine_gate_readiness(&[sandbox, workstation_privacy, "pass", "not_required"]),
            "degraded"
        );
        assert_eq!(
            privacy_readiness(ProtectionProfile::Strict, "evdev", Some(false)),
            "fail"
        );
        assert_eq!(
            privacy_readiness(ProtectionProfile::Strict, "evdev", Some(true)),
            "fail",
            "strict privacy rejects evdev based on its known source limitation"
        );
        assert_eq!(
            privacy_readiness(ProtectionProfile::Workstation, "evdev", Some(true)),
            "degraded",
            "a contradictory capability report cannot make evdev privacy-ready"
        );
        assert_eq!(
            privacy_readiness(ProtectionProfile::Strict, "input-method", Some(true)),
            "pass"
        );
        assert_eq!(
            privacy_readiness(ProtectionProfile::Strict, "unknown", Some(true)),
            "incomplete"
        );
        assert_eq!(
            combine_gate_readiness(&[
                "pass",
                privacy_readiness(ProtectionProfile::Strict, "evdev", Some(false)),
                "pass",
                "not_required",
            ]),
            "fail"
        );

        let workstation_check =
            input_privacy_check(ProtectionProfile::Workstation, "evdev", Some(false));
        let strict_check = input_privacy_check(ProtectionProfile::Strict, "evdev", Some(false));
        let unknown_check = input_privacy_check(ProtectionProfile::Strict, "unknown", None);
        assert_eq!(workstation_check["status"], "degraded");
        assert_eq!(workstation_check["gate"], "input_privacy");
        assert_eq!(workstation_check["required"], false);
        assert_eq!(strict_check["status"], "fail");
        assert_eq!(unknown_check["status"], "unavailable");
        assert!(unknown_check["detail"]
            .as_str()
            .unwrap()
            .contains("source is unknown"));
    }

    #[test]
    fn security_gate_failures_do_not_leak_into_other_gate_results() {
        let checks = vec![
            serde_json::json!({"name": "daemon_seccomp", "gate": "execution_sandbox", "required": true, "status": "fail"}),
            serde_json::json!({"name": "config_permissions", "gate": "configuration_integrity", "required": true, "status": "pass"}),
            serde_json::json!({"name": "action_broker_availability", "gate": "action_broker_availability", "required": false, "status": "pass"}),
        ];
        let sandbox = checks_for_gate(&checks, "execution_sandbox");
        let configuration = checks_for_gate(&checks, "configuration_integrity");
        let broker = checks_for_gate(&checks, "action_broker_availability");
        assert_eq!(
            readiness_for(ProtectionProfile::Workstation, &sandbox, &[]),
            "fail"
        );
        assert_eq!(
            readiness_for(ProtectionProfile::Workstation, &configuration, &[]),
            "pass"
        );
        assert_eq!(
            gate_for_check("config_permissions"),
            "configuration_integrity"
        );
        assert_eq!(
            gate_for_check("action_broker_availability"),
            "action_broker_availability"
        );
        assert_eq!(gate_for_check("broker_seccomp"), "execution_sandbox");
        assert_eq!(
            gate_for_check("broker_command_cgroup_containment"),
            "execution_sandbox"
        );
        assert!(!gate_is_required(&broker));
        let report = gate_report("not_required", false, &broker, &[]);
        assert_eq!(report["readiness"], "not_required");
        assert_eq!(report["applicable"], false);
        assert_eq!(report["checks"][0], "action_broker_availability");
        let evidence = ["UMask=0077 is not introspectable"];
        let sandbox_report = gate_report("incomplete", true, &sandbox, &evidence);
        assert_eq!(sandbox_report["unavailable_evidence"][0], evidence[0]);
        let failed_broker = [serde_json::json!({
            "name": "action_broker_availability",
            "required": true,
            "status": "fail"
        })];
        assert_eq!(
            readiness_for(ProtectionProfile::Workstation, &failed_broker, &[]),
            "fail"
        );
        assert_eq!(
            combine_gate_readiness(&["fail", "pass", "pass", "not_required"]),
            "fail"
        );

        let broker_checks = vec![
            serde_json::json!({"name": "action_broker_availability", "gate": "action_broker_availability", "required": true, "status": "pass"}),
            serde_json::json!({"name": "broker_seccomp", "gate": "execution_sandbox", "required": true, "status": "fail"}),
        ];
        let availability = checks_for_gate(&broker_checks, "action_broker_availability");
        let sandbox = checks_for_gate(&broker_checks, "execution_sandbox");
        assert_eq!(availability.len(), 1);
        assert_eq!(availability[0]["name"], "action_broker_availability");
        assert_eq!(
            readiness_for(ProtectionProfile::Workstation, &availability, &[]),
            "pass"
        );
        assert_eq!(
            readiness_for(ProtectionProfile::Workstation, &sandbox, &[]),
            "fail"
        );

        let unavailable_optional_broker = action_broker_availability_check(
            &serde_json::json!({
                "service": {"active": false},
                "socket": {"valid": false}
            }),
            false,
        );
        assert_eq!(unavailable_optional_broker["status"], "fail");
        assert_eq!(
            unavailable_optional_broker["detail"],
            "No named actions require the broker; actual availability is informational."
        );
        assert_eq!(
            gate_report(
                "not_required",
                false,
                std::slice::from_ref(&unavailable_optional_broker),
                &[]
            )["readiness"],
            "not_required"
        );
    }

    #[test]
    fn route_report_identifies_non_atomic_compatibility_override() {
        let route = active_route_summary(&serde_json::json!({
            "source": "evdev",
            "backend": "libei",
            "route_id": "kde-evdev-libei",
            "inject_atomic_replace": false,
            "replacement_assurance": "focus-tracked-best-effort",
            "compatibility_mode": "untracked-focus",
            "focus_assurance": "tracker",
            "capture_sensitive_focus": false,
        }));

        assert_eq!(route["source"], "evdev");
        assert_eq!(route["backend"], "libei");
        assert_eq!(route["atomic_replace"], false);
        assert_eq!(
            route["automatic_replacement_mode"],
            "experimental-compatibility-override"
        );
        assert_eq!(route["sensitive_field_detection"], false);
    }

    #[test]
    fn route_report_defaults_to_unknown_when_no_daemon_is_connected() {
        let route = active_route_summary(&serde_json::Value::Null);
        assert_eq!(route["automatic_replacement_mode"], "unknown");
        assert!(route["source"].is_null());
    }
}
