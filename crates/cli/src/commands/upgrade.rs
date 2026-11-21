//! `upgrade` and `upgrade-status`: finish, check or roll back a staged upgrade.

use super::system::{runnable_cli_command, runnable_cli_text};
use super::Args;
use crate::*;

pub(crate) fn upgrade_status_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    if !rest.is_empty() {
        usage_bail!("usage: wayexpand upgrade-status [--json]");
    }
    let mut report = match crate::doctor::status::read_daemon_status()
        .and_then(|response| crate::doctor::status::status_as_json(&response))
    {
        Ok(snapshot) => crate::doctor::status::upgrade_status_from_snapshot(&snapshot),
        Err(_) => crate::doctor::status::upgrade_status_unavailable(),
    };
    let home = env::var_os("HOME").map(PathBuf::from);
    let current = home
        .as_ref()
        .map(|home| home.join(".local/lib/wayexpand/current"));
    let active_target = current.as_ref().and_then(|path| fs::read_link(path).ok());
    let running_cli = env::current_exe().ok();
    let install_identity = crate::doctor::status::cli_install_identity();
    let active_payload_cli = install_identity.active_user_cli;
    let cli_matches_active_install = install_identity.cli_matches_active_user_install;
    let action = upgrade_status_invoker_action(
        report["action"].as_str().unwrap_or("unknown"),
        cli_matches_active_install,
        active_payload_cli.as_deref(),
    );
    report["action"] = serde_json::Value::String(action.clone());
    report["active_install"] = serde_json::json!({
        "payload": active_target.map(|target| target.to_string_lossy().into_owned()),
        "cli": active_payload_cli.as_ref().map(|path| path.display().to_string()),
        "running_cli": running_cli.as_ref().map(|path| path.display().to_string()),
        "cli_matches": cli_matches_active_install,
    });
    if requested_json {
        println!("{report}");
    } else {
        println!(
            "CLI: {} (commit {})",
            report["cli"]["version"].as_str().unwrap_or("unknown"),
            report["cli"]["commit"].as_str().unwrap_or("unknown")
        );
        println!("Daemon: {}", report["status"].as_str().unwrap_or("unknown"));
        if let Some(matches) = cli_matches_active_install {
            println!(
                "CLI matches active user installation: {}",
                if matches { "yes" } else { "no" }
            );
        }
        if let Some(path) = active_payload_cli.as_ref() {
            println!("Active installation CLI: {}", path.display());
        }
        if let Some(commit) = report["daemon"]["commit"].as_str() {
            println!("Daemon commit: {commit}");
        }
        println!("Action: {action}");
    }
    Ok(())
}

fn upgrade_status_invoker_action(
    daemon_action: &str,
    cli_matches_active_install: Option<bool>,
    active_payload_cli: Option<&Path>,
) -> String {
    if cli_matches_active_install == Some(false) {
        if let Some(path) = active_payload_cli {
            return format!(
                "This CLI does not match the active user installation. Run `{} upgrade-status` to assess the daemon against its installed build; this report does not establish that the service needs restarting.",
                path.display()
            );
        }
    }
    daemon_action.to_owned()
}

/// Completes the runtime half of a content-addressed installer upgrade. The
/// package/download step remains the package manager or install script's job.
pub(crate) fn upgrade_command(args: Args) -> Result<()> {
    let mut action = None;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--check" | "--apply" | "--rollback" if action.is_none() => action = Some(arg),
            "--json" => json = true,
            _ => usage_bail!("usage: wayexpand upgrade --check|--apply|--rollback [--json]"),
        }
    }
    let action = action.ok_or_else(|| usage_error("choose --check, --apply, or --rollback"))?;
    if json && action != "--check" {
        usage_bail!("--json is supported with `wayexpand upgrade --check`");
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let library = home.join(".local/lib/wayexpand");
    let current = library.join("current");
    let state = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("wayexpand");
    let rollback_file = state.join("previous-install");
    let current_target = fs::read_link(&current).ok();
    let installed_cli = env::current_exe().ok();
    let active_payload_cli = current.join("bin/wayexpand");
    let cli_matches_current_install = current_target
        .as_ref()
        .and_then(|_| match_executable_paths(&active_payload_cli, installed_cli.as_deref()));
    if !upgrade_mutation_cli_identity_is_valid(
        &action,
        current_target.is_some(),
        cli_matches_current_install,
    ) {
        bail!(
            "refusing to change the active installation from a CLI whose identity does not match it; run {} upgrade --check or --apply",
            active_payload_cli.display()
        )
    }
    let config = default_config_path();
    let config_valid = Config::load(&config).is_ok();
    let status_snapshot = crate::doctor::status::read_daemon_status()
        .and_then(|response| crate::doctor::status::status_as_json(&response))
        .ok();
    let cli_commit = build_info::COMMIT;
    let cli_version = build_info::VERSION;
    let daemon_commit = status_snapshot
        .as_ref()
        .and_then(|s| s.get("daemon_commit"))
        .and_then(serde_json::Value::as_str);
    let daemon_version = status_snapshot
        .as_ref()
        .and_then(|s| s.get("daemon_version"))
        .and_then(serde_json::Value::as_str);
    let mismatch = status_snapshot.as_ref().is_some_and(|snapshot| {
        !crate::doctor::status::daemon_build_matches(daemon_commit, daemon_version)
            || !crate::doctor::status::status_schema_compatible(snapshot)
    });
    let source = status_snapshot
        .as_ref()
        .and_then(|s| s.get("source"))
        .and_then(serde_json::Value::as_str);
    let service_is_active = |unit: &str| {
        crate::setup::run_setup_command("systemctl", &["--user", "is-active", "--quiet", unit])
            .is_ok_and(|status| status.success())
    };
    // A different active WayExpand unit is not evidence that it owns this
    // daemon. Never select an unrelated service as a restart target.
    let service = active_managed_daemon_service(source, service_is_active);
    let restart_process = service.map(upgrade_restart_process);
    let broker_unit = "wayexpand-action-broker.service";
    let broker_restart_process =
        service_is_active(broker_unit).then(|| upgrade_restart_process(broker_unit));
    let runtime_action = upgrade_check_runtime_action(
        current_target.is_some(),
        cli_matches_current_install,
        status_snapshot.is_some(),
        mismatch,
        service.is_some(),
        broker_restart_process.is_some(),
    );
    let stale_ibus_processes = crate::setup_ibus::stale_engine_processes();
    let stale_ibus_processes_json: Vec<_> = stale_ibus_processes
        .iter()
        .map(|process| {
            serde_json::json!({
                "pid": process.pid,
                "name": "wayexpand-ibus",
                "executable": process.executable,
            })
        })
        .collect();
    // Keep package/file integrity beside process health in the same read-only
    // upgrade report. This catches a package upgrade that changed the daemon
    // but left a required desktop component missing.
    let installation_checks = if action == "--check" {
        crate::commands::install::checks()
    } else {
        Vec::new()
    };
    let installation_ready = installation_checks
        .iter()
        .all(|check| check.status != "fail");
    let report = serde_json::json!({
        "schema":1, "action":action, "cli_commit":cli_commit, "cli_version":cli_version,
        "current_install":current_target.as_ref().map(|v|v.to_string_lossy().into_owned()).or_else(|| installed_cli.as_ref().map(|p|p.display().to_string())),
        "running_cli":installed_cli.as_ref().map(|p|p.display().to_string()),
        "active_payload_cli":current_target.as_ref().map(|_|active_payload_cli.display().to_string()),
        "cli_matches_current_install":cli_matches_current_install,
        "daemon_commit":daemon_commit, "daemon_version":daemon_version, "daemon_running":status_snapshot.is_some(),
        "config_valid":config_valid, "version_mismatch":mismatch,
        "restart_service":service, "rollback_available":rollback_file.is_file(),
        "restart_process":restart_process.clone(),
        "broker_restart_process":broker_restart_process.clone(),
        "runtime_action":runtime_action,
        "ibus_stale_engine_processes":stale_ibus_processes_json,
        "ibus_engine_note":"IBus engine processes are owned by ibus-daemon; restarting a WayExpand systemd unit does not reload a stale engine.",
        "installation_check":{"schema":2,"level":"installed","ready":installation_ready,"checks":installation_checks}
    });
    if action == "--check" {
        if json {
            println!("{report}");
        } else {
            println!(
                "Installed payload: {}",
                report["current_install"].as_str().unwrap_or("not found")
            );
            println!(
                "Configuration: {}",
                if config_valid {
                    "valid"
                } else {
                    "invalid or missing"
                }
            );
            let component_status = if installation_ready {
                "self-check passed".to_owned()
            } else {
                format!(
                    "required component missing; run `{}`",
                    runnable_cli_command("install-check")
                )
            };
            println!("Installed components: {component_status}");
            println!(
                "Daemon: {}",
                daemon_version.or(daemon_commit).unwrap_or("not responding")
            );
            if current_target.is_some() {
                println!(
                    "CLI identity: {}",
                    match cli_matches_current_install {
                        Some(true) => "matches active installed payload",
                        Some(false) => "does not match active installed payload",
                        None => "could not be verified against active installed payload",
                    }
                );
            }
            println!(
                "Action: {}",
                upgrade_check_runtime_action_text(runtime_action)
            );
            if mismatch {
                if let Some(target) = restart_process.as_ref() {
                    println!(
                        "Process to restart: {} (PID {})",
                        target["executable"]
                            .as_str()
                            .unwrap_or("unknown executable"),
                        target["pid"]
                            .as_u64()
                            .map(|pid| pid.to_string())
                            .unwrap_or_else(|| "unknown".into())
                    );
                    println!(
                        "Service: {} (systemd --user)",
                        target["unit"].as_str().unwrap_or("unknown")
                    );
                } else {
                    println!(
                        "Process to restart: no managed WayExpand service was identified; inspect the process owner before restarting it."
                    );
                }
            }
            if let Some(target) = broker_restart_process.as_ref() {
                println!(
                    "Action Broker process to restart: {} (PID {})",
                    target["executable"]
                        .as_str()
                        .unwrap_or("executable could not be identified"),
                    target["pid"]
                        .as_u64()
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "unknown".into())
                );
                println!(
                    "Service: {} (systemd --user)",
                    target["unit"].as_str().unwrap_or("unknown")
                );
            }
            if source == Some("ibus") && stale_ibus_processes.is_empty() {
                println!("IBus: no stale WayExpand engine process was found.");
            } else if !stale_ibus_processes.is_empty() {
                print_ibus_engine_restart_guidance(&stale_ibus_processes);
            }
        }
        if !config_valid {
            bail!("configuration validation failed; upgrade was not applied")
        }
        return Ok(());
    }
    if action == "--rollback" {
        let target = restore_previous_install(&library, &current, &rollback_file, service)?;
        let check_command = runnable_cli_command("upgrade --check");
        println!("Restored payload {target}; run `{check_command}` to verify runtime health.");
        return Ok(());
    }
    if !config_valid {
        bail!("configuration is invalid; refusing to restart against the new payload")
    }
    if status_snapshot.is_none() {
        bail!("{}", runnable_cli_text("no daemon is responding; start the chosen route with `wayexpand setup --apply`, then rerun `wayexpand upgrade --check`"))
    }
    if !mismatch {
        let broker = "wayexpand-action-broker.service";
        if crate::setup::run_setup_command("systemctl", &["--user", "is-active", "--quiet", broker])
            .is_ok_and(|status| status.success())
        {
            let status =
                crate::setup::run_setup_command("systemctl", &["--user", "restart", broker])?;
            if !status.success()
                || !crate::setup::run_setup_command(
                    "systemctl",
                    &["--user", "is-active", "--quiet", broker],
                )
                .is_ok_and(|status| status.success())
            {
                bail!("daemon build is current, but the active Action Broker did not pass its restart check")
            }
            println!("Daemon build is current; restarted active {broker}.");
        } else {
            println!("No running daemon version mismatch; nothing to apply.");
        }
        let failures = installation_check_failures();
        if !failures.is_empty() {
            bail!("{}", runnable_cli_text(&format!(
                "post-upgrade installation self-check failed for {}. Run `wayexpand install-check --json` for details",
                failures.join(", ")
            )))
        }
        if !stale_ibus_processes.is_empty() {
            print_ibus_engine_restart_guidance(&stale_ibus_processes);
        }
        return Ok(());
    }
    let Some(unit) = service else {
        bail!("{}", runnable_cli_text("daemon route is not mapped to a managed user service; restart it using its owner, then run `wayexpand upgrade --check`"))
    };
    let status = crate::setup::run_setup_command("systemctl", &["--user", "restart", unit])?;
    if !status.success() {
        if rollback_file.is_file() {
            let restored = restore_previous_install(&library, &current, &rollback_file, Some(unit));
            bail!(
                "{}",
                runnable_cli_text(&format!(
                    "{unit} restart failed; automatic rollback {}",
                    if restored.is_ok() {
                        "restored the previous payload"
                    } else {
                        "also failed; inspect the service and run `wayexpand upgrade --rollback`"
                    }
                ))
            )
        }
        bail!("{unit} restart failed; no saved payload is available for automatic rollback")
    }
    // A successful systemd restart only means the process was launched. The
    // daemon may still be initializing its control socket/status publisher, so
    // give it a short bounded window before treating an unavailable snapshot
    // as a failed upgrade.
    let (health, restarted_build) = wait_for_daemon_build();
    let restarted_build = restarted_build.as_deref().unwrap_or("unknown");
    let compatible = health.as_ref().is_ok_and(|verified| {
        crate::doctor::status::upgrade_status_from_snapshot(verified)["compatible"] == true
    });
    let build_matches = health.as_ref().is_ok_and(|verified| {
        crate::doctor::status::daemon_build_matches(
            verified
                .get("daemon_commit")
                .and_then(serde_json::Value::as_str),
            verified
                .get("daemon_version")
                .and_then(serde_json::Value::as_str),
        )
    });
    if !build_matches || !compatible {
        if rollback_file.is_file() {
            let restored = restore_previous_install(&library, &current, &rollback_file, Some(unit));
            bail!(
                "{}",
                runnable_cli_text(&upgrade_health_failure(
                    cli_version,
                    restarted_build,
                    health.is_ok(),
                    restored.is_ok()
                ))
            )
        }
        bail!(
            "{}",
            runnable_cli_text(&upgrade_health_failure(
                cli_version,
                restarted_build,
                health.is_ok(),
                false
            ))
        )
    }
    let broker = "wayexpand-action-broker.service";
    if crate::setup::run_setup_command("systemctl", &["--user", "is-active", "--quiet", broker])
        .is_ok_and(|status| status.success())
    {
        let restarted =
            crate::setup::run_setup_command("systemctl", &["--user", "restart", broker])
                .is_ok_and(|status| status.success());
        let healthy = restarted
            && crate::setup::run_setup_command(
                "systemctl",
                &["--user", "is-active", "--quiet", broker],
            )
            .is_ok_and(|status| status.success());
        if !healthy {
            if rollback_file.is_file() {
                let restored =
                    restore_previous_install(&library, &current, &rollback_file, Some(unit));
                bail!(
                    "{}",
                    runnable_cli_text(&format!(
                        "action broker health check failed; automatic rollback {}",
                        if restored.is_ok() {
                            "restored the previous payload"
                        } else {
                            "also failed; inspect services and run `wayexpand upgrade --rollback`"
                        }
                    ))
                )
            }
            bail!("action broker health check failed; no saved payload is available for automatic rollback")
        }
    }
    let installation_failures = installation_check_failures();
    if !installation_failures.is_empty() {
        let restored = rollback_file.is_file()
            && restore_previous_install(&library, &current, &rollback_file, Some(unit)).is_ok();
        bail!("{}", runnable_cli_text(&format!(
            "post-upgrade installation self-check failed for {}; automatic rollback {}. Run `wayexpand install-check --json` for details",
            installation_failures.join(", "),
            if restored {
                "restored the previous payload"
            } else {
                "was unavailable or failed; inspect the installation before using it"
            }
        )))
    }
    println!("Restarted {unit}; daemon build {restarted_build} now matches this CLI.");
    let stale_ibus_processes = crate::setup_ibus::stale_engine_processes();
    if !stale_ibus_processes.is_empty() {
        print_ibus_engine_restart_guidance(&stale_ibus_processes);
    }
    Ok(())
}

fn installation_check_failures() -> Vec<&'static str> {
    crate::commands::install::checks()
        .into_iter()
        .filter(|check| check.status == "fail")
        .map(|check| check.component)
        .collect()
}

fn upgrade_restart_process(unit: &str) -> serde_json::Value {
    let pid = crate::setup_ibus::command_output(
        "systemctl",
        &["--user", "show", unit, "--property=MainPID", "--value"],
    )
    .and_then(|value| value.trim().parse::<u32>().ok())
    .filter(|pid| *pid > 0);
    let executable = pid
        .and_then(|pid| fs::read_link(format!("/proc/{pid}/exe")).ok())
        .map(|path| path.to_string_lossy().into_owned());
    serde_json::json!({
        "unit": unit,
        "pid": pid,
        "executable": executable,
    })
}

fn active_managed_daemon_service(
    source: Option<&str>,
    mut is_active: impl FnMut(&str) -> bool,
) -> Option<&'static str> {
    let unit = match source {
        Some("evdev") => "wayexpand-evdev.service",
        Some("input-method") => "wayexpand-input-method.service",
        Some("stdin") => "wayexpand-control.service",
        _ => return None,
    };
    is_active(unit).then_some(unit)
}

fn upgrade_check_runtime_action(
    content_addressed_install: bool,
    cli_matches_current_install: Option<bool>,
    daemon_running: bool,
    daemon_mismatch: bool,
    managed_daemon_service_active: bool,
    broker_active: bool,
) -> &'static str {
    if content_addressed_install && cli_matches_current_install == Some(false) {
        "run_current_install_cli"
    } else if content_addressed_install && cli_matches_current_install.is_none() {
        "verify_current_install_cli"
    } else if !daemon_running {
        "start_selected_route"
    } else if daemon_mismatch {
        if managed_daemon_service_active {
            "restart_daemon_service"
        } else {
            "restart_daemon_manually"
        }
    } else if broker_active {
        "restart_action_broker"
    } else {
        "none"
    }
}

fn upgrade_check_runtime_action_text(action: &str) -> &'static str {
    match action {
        "run_current_install_cli" => {
            "this CLI is not from the active payload; run the active installation's CLI before applying changes"
        }
        "verify_current_install_cli" => {
            "could not verify this CLI against the active payload; do not apply changes until its identity is verified"
        }
        "start_selected_route" => {
            "no daemon responded; start the selected route before applying an upgrade"
        }
        "restart_daemon_service" => "restart the affected service after validating the new payload",
        "restart_daemon_manually" => {
            "daemon build mismatch; no active service matching its reported route was identified, so do not restart another WayExpand unit"
        }
        "restart_action_broker" => {
            "restart the active Action Broker service to load its updated binary"
        }
        _ => "no runtime version mismatch detected",
    }
}

fn match_executable_paths(
    expected: &std::path::Path,
    actual: Option<&std::path::Path>,
) -> Option<bool> {
    crate::doctor::status::executable_paths_match(expected, actual)
}

fn upgrade_mutation_cli_identity_is_valid(
    action: &str,
    content_addressed_install: bool,
    cli_matches_current_install: Option<bool>,
) -> bool {
    action != "--apply" || !content_addressed_install || cli_matches_current_install == Some(true)
}

fn print_ibus_engine_restart_guidance(processes: &[crate::setup_ibus::StaleEngineProcess]) {
    println!("IBus still has WayExpand engine process(es) running the previous binary:");
    for process in processes {
        println!(
            "  wayexpand-ibus (PID {}) — {}",
            process.pid,
            process.executable.display()
        );
    }
    println!("{}", runnable_cli_text(
        "Restart IBus to load the updated engine: `wayexpand ibus restart` (input pauses briefly)."
    ));
}

fn wait_for_daemon_build() -> (Result<serde_json::Value>, Option<String>) {
    const ATTEMPTS: usize = 20;
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

    let mut last_health = Err(anyhow::anyhow!("daemon has not published status yet"));
    let mut last_build = None;
    for attempt in 0..ATTEMPTS {
        last_health = crate::doctor::status::read_daemon_status()
            .and_then(|response| crate::doctor::status::status_as_json(&response));
        last_build = last_health.as_ref().ok().and_then(|snapshot| {
            let commit = snapshot
                .get("daemon_commit")
                .and_then(serde_json::Value::as_str);
            let version = snapshot
                .get("daemon_version")
                .and_then(serde_json::Value::as_str);
            crate::doctor::status::daemon_build_matches(commit, version)
                .then(|| version.or(commit).unwrap_or("unknown").to_owned())
        });
        if last_build.is_some() {
            break;
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(RETRY_DELAY);
        }
    }
    let reported_build = last_build.or_else(|| {
        last_health.as_ref().ok().and_then(|snapshot| {
            snapshot
                .get("daemon_version")
                .or_else(|| snapshot.get("daemon_commit"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
    });
    (last_health, reported_build)
}

fn upgrade_health_failure(
    cli_commit: &str,
    daemon_commit: &str,
    status_available: bool,
    rolled_back: bool,
) -> String {
    let diagnosis = if !status_available {
        "the restarted daemon did not return a readable status; inspect `systemctl --user status` and `journalctl --user` for the selected service"
    } else if daemon_commit == "unknown" {
        "the running daemon has no embedded source revision; reinstall/rebuild the daemon with build identity metadata, then retry"
    } else {
        "the running daemon revision does not match this CLI; verify both binaries came from the same installation payload"
    };
    format!(
        "post-restart health check failed (CLI build {cli_commit}, daemon build {daemon_commit}); {diagnosis}; automatic rollback {}",
        if rolled_back {
            "restored the previous payload"
        } else {
            "was unavailable or failed; run `wayexpand upgrade --rollback` after inspecting the service"
        }
    )
}

fn restore_previous_install(
    library: &Path,
    current: &Path,
    rollback_file: &Path,
    service: Option<&str>,
) -> Result<String> {
    let target = fs::read_to_string(rollback_file).map_err(|_| {
        anyhow::anyhow!(
            "no saved previous install target; reinstall the prior package or release to roll back"
        )
    })?;
    let target = target.trim();
    if target.is_empty()
        || target.starts_with('/')
        || target.contains("..")
        || !library.join(target).is_dir()
    {
        bail!("saved rollback target is invalid or unavailable; refusing to change current installation")
    }
    let temp = library.join(format!(".current-rollback-{}", std::process::id()));
    let _ = fs::remove_file(&temp);
    std::os::unix::fs::symlink(target, &temp)?;
    fs::rename(&temp, current)?;
    if let Some(unit) = service {
        let status = crate::setup::run_setup_command("systemctl", &["--user", "restart", unit])?;
        if !status.success() {
            bail!("restored payload, but {unit} failed to restart")
        }
    }
    let broker = "wayexpand-action-broker.service";
    if crate::setup::run_setup_command("systemctl", &["--user", "is-active", "--quiet", broker])
        .is_ok_and(|status| status.success())
    {
        let status = crate::setup::run_setup_command("systemctl", &["--user", "restart", broker])?;
        if !status.success() {
            bail!("restored payload, but {broker} failed to restart")
        }
    }
    Ok(target.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        active_managed_daemon_service, match_executable_paths, upgrade_check_runtime_action,
        upgrade_check_runtime_action_text, upgrade_health_failure,
        upgrade_mutation_cli_identity_is_valid, upgrade_status_invoker_action,
    };

    #[test]
    fn upgrade_health_failure_explains_unknown_daemon_identity() {
        let message = upgrade_health_failure("cli-sha", "unknown", true, true);
        assert!(message.contains("no embedded source revision"));
        assert!(message.contains("automatic rollback restored the previous payload"));
    }

    #[test]
    fn upgrade_health_failure_distinguishes_unavailable_status() {
        let message = upgrade_health_failure("cli-sha", "unknown", false, false);
        assert!(message.contains("did not return a readable status"));
        assert!(message.contains("run `wayexpand upgrade --rollback`"));
    }

    #[test]
    fn upgrade_check_calls_out_an_active_broker_even_when_daemon_matches() {
        let action = upgrade_check_runtime_action(false, None, true, false, true, true);
        assert_eq!(action, "restart_action_broker");
        assert!(upgrade_check_runtime_action_text(action).contains("Action Broker"));
    }

    #[test]
    fn missing_daemon_takes_precedence_over_service_restart_advice() {
        let action = upgrade_check_runtime_action(false, None, false, false, false, true);
        assert_eq!(action, "start_selected_route");
        assert!(upgrade_check_runtime_action_text(action).contains("no daemon responded"));
    }

    #[test]
    fn daemon_mismatch_does_not_guess_an_unrelated_active_unit() {
        let action = upgrade_check_runtime_action(false, None, true, true, false, true);
        assert_eq!(action, "restart_daemon_manually");
        assert!(upgrade_check_runtime_action_text(action).contains("do not restart another"));
    }

    #[test]
    fn development_cli_mismatch_takes_precedence_over_daemon_restart_advice() {
        let action = upgrade_check_runtime_action(true, Some(false), true, true, true, true);
        assert_eq!(action, "run_current_install_cli");
        assert!(upgrade_check_runtime_action_text(action).contains("active installation's CLI"));
    }

    #[test]
    fn upgrade_status_does_not_call_a_daemon_stale_when_the_cli_is_not_installed_payload() {
        let action = upgrade_status_invoker_action(
            "The running daemon does not speak this CLI's status protocol; restart it.",
            Some(false),
            Some(std::path::Path::new(
                "/home/user/.local/lib/wayexpand/current/bin/wayexpand",
            )),
        );
        assert!(action.contains("does not match the active user installation"));
        assert!(
            action.contains("/home/user/.local/lib/wayexpand/current/bin/wayexpand upgrade-status")
        );
        assert!(action.contains("does not establish that the service needs restarting"));

        let current_action = upgrade_status_invoker_action(
            "Restart the daemon because its build is stale.",
            Some(true),
            Some(std::path::Path::new(
                "/home/user/.local/lib/wayexpand/current/bin/wayexpand",
            )),
        );
        assert_eq!(
            current_action,
            "Restart the daemon because its build is stale."
        );
    }

    #[test]
    fn daemon_service_resolution_checks_only_the_reported_route() {
        let mut checked = Vec::new();
        let service = active_managed_daemon_service(Some("evdev"), |unit| {
            checked.push(unit.to_owned());
            unit == "wayexpand-input-method.service"
        });
        assert_eq!(service, None);
        assert_eq!(checked, ["wayexpand-evdev.service"]);
    }

    #[test]
    fn executable_identity_follows_the_current_payload_symlink() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wayexpand-upgrade-path-{}-{nonce}",
            std::process::id()
        ));
        let payload = root.join("payload");
        let current = root.join("current");
        std::fs::create_dir_all(payload.join("bin")).unwrap();
        std::fs::write(payload.join("bin/wayexpand"), b"test cli").unwrap();
        symlink(&payload, &current).unwrap();

        assert_eq!(
            match_executable_paths(
                &current.join("bin/wayexpand"),
                Some(&payload.join("bin/wayexpand"))
            ),
            Some(true)
        );
        assert_eq!(
            match_executable_paths(
                &current.join("bin/wayexpand"),
                Some(&root.join("other-cli"))
            ),
            None
        );
        assert_eq!(
            upgrade_check_runtime_action(true, Some(false), true, true, true, false),
            "run_current_install_cli"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn upgrade_mutations_require_verified_identity_for_content_addressed_installs() {
        assert!(upgrade_mutation_cli_identity_is_valid(
            "--apply", false, None
        ));
        assert!(upgrade_mutation_cli_identity_is_valid(
            "--apply",
            true,
            Some(true)
        ));
        assert!(!upgrade_mutation_cli_identity_is_valid(
            "--apply",
            true,
            Some(false)
        ));
        assert!(!upgrade_mutation_cli_identity_is_valid(
            "--apply", true, None
        ));
        assert!(upgrade_mutation_cli_identity_is_valid(
            "--rollback",
            true,
            Some(false)
        ));
    }
}
