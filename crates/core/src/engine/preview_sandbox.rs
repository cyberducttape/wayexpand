//! Command previews under the daemon's service sandbox.
//!
//! The daemon runs command-backed snippets as children of its systemd user
//! unit, so they inherit that unit's restrictions (read-only home, private
//! /tmp, `AF_UNIX` only, no W^X memory, task and memory limits). Running a
//! preview directly in the GUI process skips all of that: a command could
//! work in the preview and then fail, or have side effects the daemon's
//! sandbox would have prevented. When a systemd user manager is reachable,
//! previews therefore run in a transient unit with the same directives.

use std::{
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use super::command_runtime::{
    configure_process_group, run_command_cancellable, run_command_unix, stop_transient_unit,
};
use super::{CommandConfig, CommandEnvironment, CommandError, MINIMAL_COMMAND_PATH};

/// Sandbox directives of the packaged capture units
/// (`systemd/wayexpand-evdev.service`, `wayexpand-input-method.service`).
/// `ReadWritePaths=` is added separately because it needs the user's paths.
/// A test keeps this list identical to the units.
const DAEMON_SANDBOX_PROPERTIES: &[&str] = &[
    "NoNewPrivileges=yes",
    "UMask=0077",
    "PrivateTmp=yes",
    "PrivateMounts=yes",
    "ProtectSystem=strict",
    "ProtectHome=read-only",
    "ProtectHostname=yes",
    "ProtectProc=invisible",
    "ProcSubset=pid",
    "SystemCallArchitectures=native",
    "ProtectKernelTunables=yes",
    "ProtectControlGroups=yes",
    "RestrictSUIDSGID=yes",
    "RestrictNamespaces=yes",
    "RestrictRealtime=yes",
    "LockPersonality=yes",
    "MemoryDenyWriteExecute=yes",
    "RestrictAddressFamilies=AF_UNIX",
    "MemoryMax=256M",
    "TasksMax=32",
    "LimitNOFILE=64",
    "LimitCORE=0",
];

/// Time allowed for systemd to start the transient unit, on top of the
/// command's own timeout.
const SANDBOX_STARTUP_ALLOWANCE_MS: u64 = 1_000;

static PREVIEW_UNIT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Whether previews will run in the service sandbox: a systemd user manager
/// is reachable. Without one the daemon also runs unsandboxed, so a direct
/// preview matches it. Checked once per process; the GUI asks every frame.
pub fn command_preview_sandboxed() -> bool {
    static SANDBOXED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SANDBOXED.get_or_init(|| {
        std::env::var_os("XDG_RUNTIME_DIR")
            .is_some_and(|dir| Path::new(&dir).join("systemd/private").exists())
    })
}

/// Run a command preview the way the daemon would run the command: in the
/// daemon's sandbox when a systemd user manager is available, otherwise
/// directly. Broker actions always go through the broker, which applies its
/// own policy.
pub fn run_command_preview(
    command: &CommandConfig,
    cancelled: &AtomicBool,
) -> Result<String, CommandError> {
    if command.action.is_some() || !command_preview_sandboxed() {
        return run_command_cancellable(command, cancelled);
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(CommandError::StaleInput);
    }
    let unit = format!(
        "wayexpand-preview-{}-{}.service",
        std::process::id(),
        PREVIEW_UNIT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let args = sandboxed_preview_args(command, &unit, |name| std::env::var(name).ok());
    let mut process = Command::new("systemd-run");
    process
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut process);
    let mut child = process.spawn().map_err(|error| CommandError::SpawnFailed {
        detail: format!("systemd-run: {error}"),
    })?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        stop_transient_unit(Some(&unit));
        return Err(CommandError::OutputChannelLost);
    };
    let result = run_command_unix(
        child,
        stdout,
        stderr,
        command
            .timeout_ms
            .saturating_add(SANDBOX_STARTUP_ALLOWANCE_MS),
        Some(cancelled),
    );
    if result.is_err() {
        // systemd-run owns a separate cgroup: killing the wrapper on timeout
        // or cancellation does not stop the command itself.
        stop_transient_unit(Some(&unit));
    }
    result
}

/// `systemd-run` arguments for one sandboxed preview. `env` reads the
/// variables a minimal-environment command receives.
fn sandboxed_preview_args(
    command: &CommandConfig,
    unit: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "--user",
        "--quiet",
        "--wait",
        "--pipe",
        "--collect",
        "--unit",
        unit,
    ]
    .map(String::from)
    .to_vec();
    for property in DAEMON_SANDBOX_PROPERTIES {
        args.push(format!("--property={property}"));
    }
    // The units grant %t and %h/.config/wayexpand. "-" tolerates a path that
    // does not exist instead of failing the unit.
    let mut writable = Vec::new();
    if let Some(runtime) = env("XDG_RUNTIME_DIR") {
        writable.push(format!("-{runtime}"));
    }
    if let Some(home) = env("HOME") {
        writable.push(format!("-{home}/.config/wayexpand"));
    }
    if !writable.is_empty() {
        args.push(format!("--property=ReadWritePaths={}", writable.join(" ")));
    }
    // Backstop only: the wrapper's own timeout normally fires first and
    // reports a timeout like the daemon would.
    args.push(format!(
        "--property=RuntimeMaxSec={}ms",
        command
            .timeout_ms
            .saturating_add(2 * SANDBOX_STARTUP_ALLOWANCE_MS)
    ));
    args.push("--".into());
    if command.environment == CommandEnvironment::Minimal {
        // Mirror configure_command_environment: the transient unit starts
        // with the user manager's environment, so clear it explicitly.
        args.extend(["env".into(), "-i".into()]);
        for name in ["HOME", "USER", "LANG"]
            .into_iter()
            .chain(command.pass_env.iter().map(String::as_str))
        {
            if let Some(value) = env(name) {
                args.push(format!("{name}={value}"));
            }
        }
        args.push(format!("PATH={MINIMAL_COMMAND_PATH}"));
    }
    args.push(command.program.clone());
    args.extend(command.args.iter().cloned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Directives in the packaged units that are not sandbox restrictions.
    const NON_SANDBOX_DIRECTIVES: &[&str] = &[
        "Type",
        "Environment",
        "ExecStartPre",
        "ExecStart",
        "Restart",
        "RestartSec",
        "SuccessExitStatus",
        "KillSignal",
        "KillMode",
        "TimeoutStopSec",
        "StandardOutput",
        "StandardError",
        "SyslogIdentifier",
        "ReadWritePaths",
    ];

    #[test]
    fn preview_sandbox_matches_the_packaged_daemon_units() {
        let expected: BTreeSet<&str> = DAEMON_SANDBOX_PROPERTIES.iter().copied().collect();
        for unit in ["wayexpand-evdev.service", "wayexpand-input-method.service"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../systemd")
                .join(unit);
            let text = std::fs::read_to_string(&path).unwrap();
            let service = text
                .split_once("[Service]")
                .and_then(|(_, rest)| rest.split_once("\n[").map(|(service, _)| service))
                .unwrap();
            let directives: BTreeSet<&str> = service
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .filter(|line| {
                    line.split_once('=')
                        .is_some_and(|(key, _)| !NON_SANDBOX_DIRECTIVES.contains(&key))
                })
                .collect();
            assert_eq!(directives, expected, "{unit}");
            assert!(
                service.contains("ReadWritePaths=%t %h/.config/wayexpand"),
                "{unit}: update the ReadWritePaths mapping in sandboxed_preview_args"
            );
        }
    }

    fn command(environment: CommandEnvironment) -> CommandConfig {
        CommandConfig {
            action: None,
            program: "date".into(),
            args: vec!["+%F".into()],
            timeout_ms: 500,
            cache_ms: 0,
            environment,
            pass_env: vec!["TZ".into()],
        }
    }

    fn test_env(name: &str) -> Option<String> {
        match name {
            "XDG_RUNTIME_DIR" => Some("/run/user/1000".into()),
            "HOME" => Some("/home/u".into()),
            "USER" => Some("u".into()),
            "TZ" => Some("UTC".into()),
            _ => None,
        }
    }

    #[test]
    fn minimal_preview_clears_the_environment_like_the_daemon() {
        let args = sandboxed_preview_args(
            &command(CommandEnvironment::Minimal),
            "unit.service",
            test_env,
        );
        let command_start = args.iter().position(|arg| arg == "--").unwrap() + 1;
        assert_eq!(
            &args[command_start..],
            [
                "env".to_string(),
                "-i".into(),
                "HOME=/home/u".into(),
                "USER=u".into(),
                "TZ=UTC".into(),
                format!("PATH={MINIMAL_COMMAND_PATH}"),
                "date".into(),
                "+%F".into(),
            ]
        );
        assert!(args.contains(
            &"--property=ReadWritePaths=-/run/user/1000 -/home/u/.config/wayexpand".to_string()
        ));
        assert!(args.contains(&"--property=MemoryDenyWriteExecute=yes".to_string()));
        assert!(args.contains(&"--property=RuntimeMaxSec=2500ms".to_string()));
    }

    /// Needs a systemd user manager: `cargo test -p wayexpand-core
    /// live_preview -- --ignored`.
    #[test]
    #[ignore = "requires a systemd user manager"]
    fn live_preview_runs_in_the_daemon_sandbox() {
        assert!(command_preview_sandboxed(), "no systemd user manager");
        let cancelled = AtomicBool::new(false);
        let shell = |script: &str, timeout_ms| CommandConfig {
            action: None,
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            timeout_ms,
            cache_ms: 0,
            environment: CommandEnvironment::Minimal,
            pass_env: Vec::new(),
        };

        assert_eq!(
            run_command_preview(&shell("printf ok", 2_000), &cancelled).as_deref(),
            Ok("ok")
        );
        // The unit's process restrictions apply. Mount-namespace directives
        // (ProtectHome=, PrivateTmp=, ...) are not checked here: some hosts
        // (e.g. Ubuntu with apparmor_restrict_unprivileged_userns=1) cannot
        // set up user-manager mount namespaces, for the daemon either.
        let status = run_command_preview(
            &shell("grep -E '^(NoNewPrivs|Seccomp):' /proc/self/status", 2_000),
            &cancelled,
        )
        .unwrap();
        assert!(status.contains("NoNewPrivs:\t1"), "{status}");
        assert!(status.contains("Seccomp:\t2"), "{status}");

        let started = std::time::Instant::now();
        assert_eq!(
            run_command_preview(&shell("sleep 30", 300), &cancelled),
            Err(CommandError::Timeout)
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // The transient unit is stopped, not left running.
        std::thread::sleep(std::time::Duration::from_millis(500));
        let units = Command::new("systemctl")
            .args(["--user", "list-units", "--no-legend", "wayexpand-preview-*"])
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&units.stdout)
                .contains(&format!("wayexpand-preview-{}-", std::process::id())),
            "{}",
            String::from_utf8_lossy(&units.stdout)
        );
    }

    #[test]
    fn inherited_preview_runs_the_program_directly() {
        let args = sandboxed_preview_args(
            &command(CommandEnvironment::Inherit),
            "unit.service",
            test_env,
        );
        let command_start = args.iter().position(|arg| arg == "--").unwrap() + 1;
        assert_eq!(&args[command_start..], ["date", "+%F"]);
    }
}
