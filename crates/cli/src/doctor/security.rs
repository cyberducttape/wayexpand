//! Effective runtime sandbox inspection for `wayexpand doctor --security`.

use super::{
    broker::broker_diagnostics,
    status::{read_daemon_status, status_as_json},
};
use crate::*;
use std::os::unix::fs::MetadataExt;

pub(crate) fn run(config_path: &Path, json: bool) -> Result<()> {
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
            daemon["command_containment"].as_str() == Some("cgroup"),
            "Command-backed expansions must use cgroup descendant cleanup.",
        ),
    ];

    let broker_required = broker["required"].as_bool() == Some(true);
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
    let healthy = checks.iter().all(|check| check["status"] == "pass");
    let report = serde_json::json!({
        "schema": 1,
        "healthy": healthy,
        "daemon_pid": sandbox.get("pid"),
        "daemon": sandbox,
        "configuration": config_permissions,
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
        println!("Daemon PID: {}", report["daemon_pid"]);
        for check in report["checks"].as_array().into_iter().flatten() {
            println!(
                "{}: {} — {}",
                check["name"].as_str().unwrap_or("unknown"),
                check["status"].as_str().unwrap_or("unknown"),
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
        println!("Overall: {}", if healthy { "PASS" } else { "FAIL" });
    }
    if healthy {
        Ok(())
    } else {
        bail!("effective sandbox checks failed; inspect the security report")
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
