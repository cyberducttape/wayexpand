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

    let local_mount_ns = namespace_inode("mnt");
    let local_uts_ns = namespace_inode("uts");
    let local_tmp_mount = mount_id_for(Path::new("/tmp"));
    let mut checks = vec![
        check(
            "daemon_connected",
            !sandbox.is_null(),
            "The live daemon must provide its in-process security probes.",
        ),
        check(
            "config_permissions",
            config_permissions["safe"].as_bool() == Some(true),
            "The configuration must be a valid regular file owned by this user or root and not group/world writable; Config loading also validates its ancestors.",
        ),
        check(
            "daemon_mount_namespace",
            sandbox["namespaces"]["mount"].as_u64().is_some()
                && local_mount_ns.is_some()
                && sandbox["namespaces"]["mount"].as_u64() != local_mount_ns,
            "The running daemon must be in a mount namespace distinct from the doctor process.",
        ),
        check(
            "daemon_private_tmp",
            sandbox["tmp_mount_id"].as_u64().is_some()
                && local_tmp_mount.is_some()
                && sandbox["tmp_mount_id"].as_u64() != local_tmp_mount,
            "The daemon and desktop session must see distinct /tmp mounts.",
        ),
        check(
            "daemon_uts_namespace",
            sandbox["namespaces"]["uts"].as_u64().is_some()
                && local_uts_ns.is_some()
                && sandbox["namespaces"]["uts"].as_u64() != local_uts_ns,
            "ProtectHostname should place the daemon in a distinct UTS namespace.",
        ),
        check(
            "daemon_no_new_privileges",
            sandbox["no_new_privileges"].as_bool() == Some(true),
            "The kernel must report NoNewPrivs=1 for the daemon process.",
        ),
        check(
            "daemon_seccomp",
            sandbox["seccomp_mode"].as_u64() == Some(2),
            "The kernel must report seccomp filter mode for the daemon process.",
        ),
        check(
            "daemon_af_unix_allowed",
            state(&sandbox, &["address_families", "unix"]) == Some("allowed"),
            "The daemon needs AF_UNIX for its control and portal sockets.",
        ),
        check(
            "daemon_af_inet_blocked",
            state(&sandbox, &["address_families", "inet"]) == Some("blocked"),
            "The daemon must be unable to create AF_INET sockets.",
        ),
        check(
            "daemon_af_inet6_blocked",
            state(&sandbox, &["address_families", "inet6"]) == Some("blocked"),
            "The daemon must be unable to create AF_INET6 sockets.",
        ),
        check(
            "daemon_home_read_only",
            sandbox["home_read_only"].as_bool() == Some(true),
            "The daemon's home mount must be read-only outside explicit unit exceptions.",
        ),
        check(
            "daemon_system_read_only",
            sandbox["system_read_only"].as_bool() == Some(true),
            "The daemon's /usr mount must be read-only under ProtectSystem=strict.",
        ),
        check(
            "daemon_kernel_tunables_read_only",
            sandbox["kernel_tunables_read_only"].as_bool() == Some(true),
            "The daemon must not have a writable /proc/sys mount.",
        ),
        check(
            "daemon_proc_subset_pid",
            sandbox["proc_subset_pid"].as_bool() == Some(true),
            "The daemon's proc mount should expose only process-related entries.",
        ),
        check(
            "daemon_proc_hidepid",
            sandbox["proc_hidepid_invisible"].as_bool() == Some(true),
            "ProtectProc=invisible should be effective on the daemon proc mount.",
        ),
        check(
            "daemon_memory_deny_write_execute",
            state(&sandbox, &["memory_write_execute_blocked"]) == Some("blocked"),
            "A direct anonymous RWX mapping attempt must be refused.",
        ),
        check(
            "daemon_restrict_namespaces",
            state(&sandbox, &["namespace_creation"]) == Some("blocked"),
            "A child-process probe must be unable to create an unprivileged user namespace.",
        ),
        check(
            "daemon_lock_personality",
            state(&sandbox, &["personality_syscall"]) == Some("blocked"),
            "The personality syscall must be blocked by LockPersonality.",
        ),
        check(
            "daemon_restrict_suid_sgid",
            state(&sandbox, &["setuid_mode_bit"]) == Some("blocked"),
            "Setting a setuid bit on an ephemeral memfd must be refused.",
        ),
        check(
            "daemon_core_limit",
            sandbox["max_core_file_size"][1].as_u64() == Some(0),
            "The hard core-file limit must be zero.",
        ),
        check(
            "daemon_open_file_limit",
            sandbox["max_open_files"][1].as_u64().is_some_and(|limit| limit <= 64),
            "The hard open-file limit must not exceed the service's LimitNOFILE=64.",
        ),
        check(
            "daemon_memory_limit",
            sandbox["cgroup_memory_max_bytes"]
                .as_u64()
                .is_some_and(|limit| limit <= 256 * 1024 * 1024),
            "The effective cgroup memory.max must not exceed MemoryMax=256M.",
        ),
        check(
            "daemon_task_limit",
            sandbox["cgroup_pids_max"]
                .as_u64()
                .is_some_and(|limit| limit <= 32),
            "The effective cgroup pids.max must not exceed TasksMax=32.",
        ),
        check(
            "daemon_command_cgroup_containment",
            daemon["command_containment"].as_str() == Some("cgroup")
                && daemon["command_containment_self_test"].as_str() == Some("passed"),
            "The daemon must require cgroup containment and pass its harmless descendant-cancellation self-test in its own service context.",
        ),
    ];

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
    checks.push(check(
        "action_broker_availability",
        !broker_required || broker_running,
        if broker_required {
            "A configured named action requires a healthy, active Action Broker."
        } else {
            "No named actions require the broker; availability is reported for visibility."
        },
    ));
    if broker_required && broker_running {
        let broker_mount = broker_sandbox["namespaces"]["mount"].as_u64();
        let broker_uts = broker_sandbox["namespaces"]["uts"].as_u64();
        checks.extend([
            check(
                "broker_command_cgroup_containment",
                broker["command_containment_self_test"].as_str() == Some("passed"),
                "The running Action Broker must pass the descendant-cancellation self-test in its own service context.",
            ),
            check(
                "broker_seccomp",
                broker_sandbox["seccomp_mode"].as_u64() == Some(2),
                "The running broker must report an active seccomp filter.",
            ),
            check(
                "broker_no_new_privileges",
                broker_sandbox["no_new_privileges"].as_bool() == Some(true),
                "The running broker must report NoNewPrivs=1.",
            ),
            check(
                "broker_af_unix_allowed",
                state(&broker_sandbox, &["address_families", "unix"]) == Some("allowed"),
                "The broker requires AF_UNIX for its IPC socket.",
            ),
            check(
                "broker_af_inet_blocked",
                state(&broker_sandbox, &["address_families", "inet"]) == Some("blocked"),
                "The broker must not be able to create AF_INET sockets.",
            ),
            check(
                "broker_af_inet6_blocked",
                state(&broker_sandbox, &["address_families", "inet6"]) == Some("blocked"),
                "The broker must not be able to create AF_INET6 sockets.",
            ),
            check(
                "broker_mount_namespace",
                broker_mount.is_some() && broker_mount != local_mount_ns,
                "The running broker should have an isolated mount namespace.",
            ),
            check(
                "broker_uts_namespace",
                broker_uts.is_some() && broker_uts != local_uts_ns,
                "The running broker should have an isolated UTS namespace.",
            ),
        ]);
    }

    // These restrictions cannot be established from /proc status alone. Keep
    // the limits explicit instead of treating configured unit text as proof.
    let unverified = [
        "SystemCallArchitectures=native (the kernel does not expose the installed rule set)",
        "RestrictRealtime=yes (the installed seccomp rule is not introspectable here)",
        "UMask=0077 (the active umask cannot be queried without changing it)",
        "broker IPAddressDeny=any (the address-family probe verifies socket creation, not cgroup BPF policy)",
    ];
    let required = required_checks(profile);
    for check in &mut checks {
        let name = check["name"].as_str().unwrap_or_default();
        check["required"] = serde_json::Value::Bool(
            (profile == ProtectionProfile::Strict || required.contains(&name))
                && (name != "daemon_command_cgroup_containment"
                    || profile != ProtectionProfile::Workstation
                    || command_execution_configured),
        );
    }
    let readiness = readiness_for(profile, &checks, &unverified);
    let healthy = readiness == "pass";
    let capture_source = daemon["source"].as_str().unwrap_or("unknown");
    let sensitive_field_detection = daemon["capture_sensitive_focus"].as_bool();
    let evdev_privacy = capture_source == "evdev";
    let route = active_route_summary(&daemon);
    let report = serde_json::json!({
        "schema": 2,
        "profile": profile.name(),
        "readiness": readiness,
        "healthy": healthy,
        "daemon_pid": sandbox.get("pid"),
        "daemon": sandbox,
        "active_route": route,
        "input_privacy": {
            "source": capture_source,
            "sensitive_field_detection": sensitive_field_detection,
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
            "runtime": broker_sandbox,
            "network_policy_note": "AF_INET/AF_INET6 socket creation is probed in the live broker; IPAddressDeny BPF is listed as unverified because its attached filter cannot be queried without elevated inspection privileges."
        },
        "checks": checks,
        "unverified_mechanisms": unverified,
    });

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("WayExpand effective security report");
        println!("Protection profile: {}", profile.name());
        println!("Readiness: {}", readiness.to_ascii_uppercase());
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
        println!("Unverified systemd restrictions:");
        for mechanism in unverified {
            println!("  - {mechanism}");
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

fn required_checks(profile: ProtectionProfile) -> &'static [&'static str] {
    const WORKSTATION: &[&str] = &[
        "daemon_connected",
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
        .any(|check| check["required"] == true && check["status"] != "pass");
    let optional_failure = checks
        .iter()
        .any(|check| check["required"] != true && check["status"] != "pass");
    if required_failure || (profile == ProtectionProfile::Strict && !unverified.is_empty()) {
        "fail"
    } else if optional_failure {
        "degraded"
    } else {
        "pass"
    }
}

fn state<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(*key))
        .and_then(serde_json::Value::as_str)
}

fn check(name: &str, passed: bool, detail: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "status": if passed { "pass" } else { "fail" },
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
        assert!(
            !required_checks(ProtectionProfile::Workstation).contains(&"daemon_mount_namespace")
        );
    }

    #[test]
    fn strict_readiness_fails_closed_on_unverified_controls() {
        let passing = [serde_json::json!({"required": true, "status": "pass"})];
        let unverified = ["UMask=0077 is not introspectable"];
        assert_eq!(
            readiness_for(ProtectionProfile::Hardened, &passing, &unverified),
            "pass"
        );
        assert_eq!(
            readiness_for(ProtectionProfile::Strict, &passing, &unverified),
            "fail"
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
