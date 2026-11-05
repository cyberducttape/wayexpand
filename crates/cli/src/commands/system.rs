//! Setup and diagnostics commands: setup, doctor, certify, backend selection, and portal tokens.

use super::Args;
use crate::args::take_flag;
use crate::*;

pub(crate) fn setup_command(mut args: Args) -> Result<()> {
    let mut requested_backend: Option<String> = None;
    let mut requested_mode: Option<String> = None;
    let mut assume_yes = false;
    let mut terminals = false;
    let mut check_only = false;
    let mut apply_requested = false;
    let mut json = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--check" => check_only = true,
            "--apply" => apply_requested = true,
            "--json" => json = true,
            "--terminals" => terminals = true,
            "--experimental-input-method-v2" => {
                requested_backend = Some("input-method".into());
            }
            "--yes" | "-y" => assume_yes = true,
            "--backend" => {
                requested_backend = Some(args.next().ok_or_else(|| {
                    usage_error("--backend requires ibus, input-method, or evdev")
                })?);
            }
            "--mode" => {
                requested_mode = Some(args.next().ok_or_else(|| {
                    usage_error("--mode requires recommended, maximum, or experimental")
                })?);
            }
            value if value.starts_with("--backend=") => {
                requested_backend = Some(value.trim_start_matches("--backend=").to_owned());
            }
            value if value.starts_with("--mode=") => {
                requested_mode = Some(value.trim_start_matches("--mode=").to_owned());
            }
            _ => {
                usage_bail!(
                    "usage: wayexpand setup [--mode recommended|maximum|experimental | --backend ibus|input-method|evdev] [--terminals] [--yes]"
                )
            }
        }
    }
    if check_only && apply_requested {
        usage_bail!("choose either `wayexpand setup --check` or `wayexpand setup --apply`");
    }
    if check_only {
        if requested_backend.is_some() || requested_mode.is_some() || terminals || assume_yes {
            usage_bail!("usage: wayexpand setup --check [--json]");
        }
        return crate::commands::install::setup_check(json);
    }
    if requested_backend.is_some() && requested_mode.is_some() {
        usage_bail!("setup accepts either --mode or expert --backend, not both");
    }
    if json {
        usage_bail!("--json is supported with `wayexpand setup --check`");
    }
    println!(
        "WayExpand setup{}",
        if apply_requested {
            " — applying selected setup"
        } else {
            ""
        }
    );
    println!(
        "WayExpand {} (commit {})",
        build_info::VERSION,
        build_info::COMMIT
    );
    println!("Session: {}", session_description());
    println!();
    Config::ensure_user_config(default_config_path()).map_err(|error| {
        config_error(format!(
            "could not initialize configuration: {}",
            error.safe_summary()
        ))
    })?;
    let capabilities = probe_capabilities();
    let policy = load_policy()?;
    let ibus_available = ibus_engine_available();
    let automatic = recommended_setup_backend_with_ibus(&capabilities, ibus_available, &policy);
    let route_plan =
        wayexpand_backend_selection::plan_routes(&capabilities, ibus_available, |route| {
            route_allowed_by_policy(&policy, route)
        });
    println!();
    println!("Compatibility modes");
    println!("  Recommended         safest detected path; run doctor/certify for verification");
    println!("  Maximum compatibility explicit evdev fallback; wider application coverage with global-input and privacy tradeoffs");
    println!("  Experimental         protocol paths whose key pass-through is not certified");
    println!();
    println!("Automatic recommendation: {}", automatic.label);
    println!("  {}", automatic.detail);
    println!("\nRanked routes (current session and policy):");
    for planned in &route_plan.routes {
        println!(
            "  {:<24} {:<18} {}",
            planned.contract.label,
            planned.standing.as_str(),
            planned.reason
        );
        let assessment = &planned.assessment;
        println!(
            "    Installed: {} · Supported: {} · Authorized: {} · Connected: {} · Policy allowed: {} · Certified: {}",
            assessment.installed.as_str(),
            assessment.supported.as_str(),
            assessment.authorized.as_str(),
            assessment.connected.as_str(),
            assessment.policy_allowed,
            assessment.certified,
        );
        for limitation in &assessment.limitations {
            println!("    Limitation: {}", limitation.description());
        }
        if let Some(portal) = &assessment.portal {
            if portal.provider_candidate || portal.direct_eis_socket_configured {
                println!(
                    "    Portal/EIS: candidate={} · implementation={} · interface={} · authorization={} · direct socket={} · EIS={} · injection={} · verified={}",
                    portal.provider_candidate,
                    portal.implementation_discovered.as_str(),
                    portal.interface_available.as_str(),
                    portal.authorization.as_str(),
                    portal.direct_eis_socket_configured,
                    portal.eis_connected.as_str(),
                    portal.injection_negotiated.as_str(),
                    portal.injection_verified.as_str(),
                );
            }
        }
    }
    let backend = if let Some(backend) = requested_backend {
        backend
    } else if let Some(mode) = requested_mode {
        setup_backend_for_mode(&mode, &capabilities, &policy)?
    } else if assume_yes {
        automatic.backend
    } else {
        let prompt = format!("Configure Recommended mode ({})? [Y/n] ", automatic.label);
        if prompt_yes_no(&prompt, true)? {
            automatic.backend
        } else {
            prompt_mode_choice(&capabilities, &policy)?
        }
    };
    if backend == "unavailable" {
        bail!(
                "Recommended mode found no safe automatic path; use --mode experimental or configure permissions and rerun setup"
            );
    }
    if !setup_backend_allowed(&policy, &backend) {
        bail!("setup backend '{backend}' is disallowed by organization policy")
    }
    configure_setup_backend(
        &backend,
        crate::setup_ibus::IbusSetupOptions {
            assume_yes,
            terminals,
        },
    )?;
    Ok(())
}

pub(crate) fn doctor_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let requested_security = rest
        .iter()
        .position(|argument| argument == "--security")
        .map(|index| rest.remove(index))
        .is_some();
    if requested_security {
        let mut profile = crate::doctor::security::ProtectionProfile::Workstation;
        let mut positional = Vec::new();
        let mut index = 0;
        while index < rest.len() {
            let argument = rest[index].as_str();
            if argument == "--certify" {
                usage_bail!("usage: wayexpand doctor --security [--profile workstation|hardened|strict] [--json] [config]");
            }
            if argument == "--profile" {
                let value = rest.get(index + 1).ok_or_else(|| {
                    usage_error("--profile requires workstation, hardened, or strict")
                })?;
                profile = crate::doctor::security::ProtectionProfile::parse(value)?;
                index += 2;
            } else if let Some(value) = argument.strip_prefix("--profile=") {
                profile = crate::doctor::security::ProtectionProfile::parse(value)?;
                index += 1;
            } else if argument.starts_with('-') {
                usage_bail!("usage: wayexpand doctor --security [--profile workstation|hardened|strict] [--json] [config]");
            } else {
                positional.push(argument.to_owned());
                index += 1;
            }
        }
        if positional.len() > 1 {
            usage_bail!("usage: wayexpand doctor --security [--profile workstation|hardened|strict] [--json] [config]");
        }
        let config_path = positional
            .into_iter()
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(default_config_path);
        return crate::doctor::security::run(&config_path, requested_json, profile);
    }
    if rest.iter().any(|argument| argument == "--certify") {
        if rest.len() != 1 {
            usage_bail!("usage: wayexpand doctor --certify [--json]");
        }
        return crate::doctor::live_certification::run(requested_json);
    }
    if rest.len() > 1 {
        usage_bail!("usage: wayexpand doctor [--json] [config]");
    }
    let config_path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    if requested_json {
        let healthy = print_json_diagnostics(&config_path)?;
        if !healthy {
            bail!("doctor reported unhealthy host readiness or runtime health; inspect the JSON checks for details");
        }
        return Ok(());
    }
    println!(
        "WayExpand {} (commit {})",
        build_info::VERSION,
        build_info::COMMIT
    );
    println!("Session: {}", session_description());
    println!("Feature limits: IME preedit/composition is unsupported; app_filter window tracking is KWin-only (fails closed elsewhere).");
    let config_ok = print_config_diagnostics(&config_path);
    let control_socket_ok = print_control_socket_diagnostics();
    let output_ok = crate::doctor::status::print_output_health_diagnostics();
    let policy_ok = print_policy_diagnostics();
    let broker_ok = print_broker_diagnostics(Config::load(&config_path).ok().as_ref());
    let evdev_consent_ok = crate::doctor::backends::print_evdev_consent_diagnostics();
    let capture_ready = print_backend_diagnostics(true);
    print_capabilities_diagnostics();
    if !config_ok
        || !control_socket_ok
        || !output_ok
        || !policy_ok
        || !broker_ok
        || !evdev_consent_ok
        || !capture_ready
    {
        bail!("doctor found configuration, runtime, or backend problems");
    }
    Ok(())
}

pub(crate) fn policy_command(mut args: Args) -> Result<()> {
    let action = args.next();
    match action.as_deref() {
        Some("inspect") => policy_inspect(args),
        Some("check") => policy_check(args),
        _ => usage_bail!("usage: wayexpand policy inspect|check [--file PATH] [--json]"),
    }
}

fn policy_inspect(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let policy_path = match rest.iter().position(|argument| argument == "--file") {
        Some(index) => {
            rest.remove(index);
            if index >= rest.len() {
                usage_bail!("--file requires a policy path");
            }
            Some(PathBuf::from(rest.remove(index)))
        }
        None => None,
    };
    if !rest.is_empty() {
        usage_bail!("usage: wayexpand policy inspect [--file PATH] [--json]");
    }
    let (policy, policy_path, trusted_system_policy) = match policy_path {
        Some(path) => {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("could not read policy file {}", path.display()))?;
            let policy = wayexpand_core::parse_organization_policy(&content).map_err(|error| {
                anyhow::anyhow!("invalid policy file {}: {error}", path.display())
            })?;
            (policy, path, false)
        }
        None => (
            crate::doctor::policy::load_policy()?,
            PathBuf::from(wayexpand_core::ORGANIZATION_POLICY_PATH),
            true,
        ),
    };
    let report = crate::doctor::policy::effective_policy_report(&policy);
    if requested_json {
        println!(
            "{}",
            serde_json::json!({
                "path": policy_path,
                "trusted_system_policy": trusted_system_policy,
                "policy": report,
            })
        );
    } else {
        println!("Organization policy: {}", policy_path.display());
        if !trusted_system_policy {
            println!("Source file is syntax-inspected only; it is not loaded as the trusted system policy.");
        }
        println!("Mode: {}", report["mode"].as_str().unwrap_or("unknown"));
        let audit_only = report["audit_only_controls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        if audit_only.is_empty() {
            println!("Audit-only controls: none");
        } else {
            println!("Audit-only controls: {}", audit_only.join(", "));
        }
        let unsupported = report["unsupported_controls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        if unsupported.is_empty() {
            println!("Known unsupported controls: none");
        } else {
            println!("Known unsupported controls: {}", unsupported.join(", "));
        }
        println!("Capability assessment: {}", report["capability_assessment"]);
    }
    Ok(())
}

fn policy_check(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let allow_audit_only = take_flag(&mut rest, "--allow-audit-only");
    let policy_file = take_option(&mut rest, "--file")
        .map_err(|error| usage_error(error.to_string()))?
        .map(PathBuf::from);
    if !rest.is_empty() {
        usage_bail!("usage: wayexpand policy check [--file PATH] [--allow-audit-only] [--json]");
    }
    let trusted_system_policy = policy_file.is_none();
    let report_path = policy_file
        .clone()
        .unwrap_or_else(|| PathBuf::from(wayexpand_core::ORGANIZATION_POLICY_PATH));
    let loaded_policy = match policy_file {
        Some(path) => fs::read_to_string(&path)
            .map_err(|error| {
                config_error(format!(
                    "could not read policy file {}: {error}",
                    path.display()
                ))
            })
            .and_then(|source| {
                wayexpand_core::parse_organization_policy(&source).map_err(|error| {
                    config_error(format!("invalid policy file {}: {error}", path.display()))
                })
            }),
        None => crate::doctor::policy::load_policy()
            .map_err(|error| config_error(format!("organization policy is invalid: {error}"))),
    };
    let policy = match loaded_policy {
        Ok(policy) => policy,
        Err(error) => {
            if requested_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schema_version": 1,
                        "command": "policy check",
                        "ok": false,
                        "exit_code": 3,
                        "trusted_system_policy": trusted_system_policy,
                        "path": report_path,
                        "findings": [{"control":"policy_file", "status":"fail", "reason": error.to_string()}],
                    })
                );
            }
            return Err(error);
        }
    };
    let report = crate::doctor::policy::effective_policy_report(&policy);
    let findings = policy_check_findings(&policy, &report, allow_audit_only);
    let failed = findings.iter().any(|finding| finding["status"] == "fail");
    let output = serde_json::json!({
        "schema_version": 1,
        "command": "policy check",
        "ok": !failed,
        "exit_code": if failed { 1 } else { 0 },
        "trusted_system_policy": trusted_system_policy,
        "path": report_path,
        "policy": report,
        "findings": findings,
    });
    if requested_json {
        println!("{output}");
    } else {
        println!(
            "Policy check: {} ({})",
            if failed { "FAIL" } else { "PASS" },
            output["policy"]["mode"].as_str().unwrap_or("unknown")
        );
        for finding in output["findings"].as_array().into_iter().flatten() {
            println!(
                "{}  {} — {}",
                finding["status"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_uppercase(),
                finding["control"].as_str().unwrap_or("policy"),
                finding["reason"].as_str().unwrap_or("")
            );
        }
    }
    if failed {
        bail!("policy check failed; inspect the control-level findings above");
    }
    Ok(())
}

fn policy_check_findings(
    policy: &OrganizationPolicy,
    report: &serde_json::Value,
    allow_audit_only: bool,
) -> Vec<serde_json::Value> {
    let mut findings = Vec::new();
    if !policy.safe_mode {
        findings.push(serde_json::json!({
            "control": "safe_mode",
            "status": if allow_audit_only {"warn"} else {"fail"},
            "reason": "safe_mode is false, so restriction controls are audit-only and are not enforced",
        }));
    }
    if let Some(controls) = report["controls"].as_object() {
        for (name, control) in controls {
            if control["status"] == "audit_only" {
                findings.push(serde_json::json!({
                    "control": name,
                    "status": if allow_audit_only {"warn"} else {"fail"},
                    "reason": format!("{name} is configured but cleared from the effective policy because safe_mode is false"),
                }));
            }
            if control["support"] == "unsupported" && control["status"] != "not_configured" {
                findings.push(serde_json::json!({
                    "control": name,
                    "status": "fail",
                    "reason": format!("{name} is configured but this build cannot provide the required capability"),
                }));
            }
        }
    }
    if findings.is_empty() {
        findings.push(serde_json::json!({"control":"policy","status":"pass","reason":"configured restrictions are enforced; session-dependent controls still require runtime verification"}));
    }
    findings
}

pub(crate) fn upgrade_status_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    if !rest.is_empty() {
        usage_bail!("usage: wayexpand upgrade-status [--json]");
    }
    let report = match crate::doctor::status::read_daemon_status()
        .and_then(|response| crate::doctor::status::status_as_json(&response))
    {
        Ok(snapshot) => crate::doctor::status::upgrade_status_from_snapshot(&snapshot),
        Err(_) => crate::doctor::status::upgrade_status_unavailable(),
    };
    if requested_json {
        println!("{report}");
    } else {
        println!(
            "CLI: {} (commit {})",
            report["cli"]["version"].as_str().unwrap_or("unknown"),
            report["cli"]["commit"].as_str().unwrap_or("unknown")
        );
        println!("Daemon: {}", report["status"].as_str().unwrap_or("unknown"));
        if let Some(commit) = report["daemon"]["commit"].as_str() {
            println!("Daemon commit: {commit}");
        }
        println!("Action: {}", report["action"].as_str().unwrap_or("unknown"));
    }
    Ok(())
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
    let config = default_config_path();
    let config_valid = Config::load(&config).is_ok();
    let status_snapshot = crate::doctor::status::read_daemon_status()
        .and_then(|response| crate::doctor::status::status_as_json(&response))
        .ok();
    let cli_commit = build_info::COMMIT;
    let daemon_commit = status_snapshot
        .as_ref()
        .and_then(|s| s.get("daemon_commit"))
        .and_then(serde_json::Value::as_str);
    let mismatch = status_snapshot
        .as_ref()
        .is_some_and(|_| daemon_commit != Some(cli_commit));
    let source = status_snapshot
        .as_ref()
        .and_then(|s| s.get("source"))
        .and_then(serde_json::Value::as_str);
    let proposed_service = match source {
        Some("evdev") => Some("wayexpand-evdev.service"),
        Some("input-method") => Some("wayexpand-input-method.service"),
        Some("stdin") => Some("wayexpand-control.service"),
        _ => None,
    };
    let service_is_active = |unit: &str| {
        crate::setup::run_setup_command("systemctl", &["--user", "is-active", "--quiet", unit])
            .is_ok_and(|status| status.success())
    };
    let service = proposed_service
        .filter(|unit| service_is_active(unit))
        .or_else(|| {
            crate::services::UserService::ALL
                .into_iter()
                .map(crate::services::UserService::unit)
                .find(|unit| service_is_active(unit))
        });
    let report = serde_json::json!({
        "schema":1, "action":action, "cli_commit":cli_commit,
        "current_install":current_target.as_ref().map(|v|v.to_string_lossy().into_owned()).or_else(|| installed_cli.as_ref().map(|p|p.display().to_string())),
        "daemon_commit":daemon_commit, "daemon_running":status_snapshot.is_some(),
        "config_valid":config_valid, "version_mismatch":mismatch,
        "restart_service":service, "rollback_available":rollback_file.is_file(),
        "ibus_engine_note":"An IBus engine is owned by ibus-daemon; restarting a WayExpand systemd unit does not reload that engine. Restart IBus explicitly after reviewing the impact."
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
            println!("Daemon: {}", daemon_commit.unwrap_or("not responding"));
            println!(
                "Action: {}",
                if status_snapshot.is_none() {
                    "no daemon responded; start the selected route before applying an upgrade"
                } else if mismatch {
                    "restart the affected service after validating the new payload"
                } else {
                    "no runtime version mismatch detected"
                }
            );
            if source == Some("ibus") {
                println!(
                    "IBus: {}",
                    report["ibus_engine_note"].as_str().unwrap_or("")
                );
            }
        }
        if !config_valid {
            bail!("configuration validation failed; upgrade was not applied")
        }
        return Ok(());
    }
    if action == "--rollback" {
        let target = restore_previous_install(&library, &current, &rollback_file, service)?;
        println!(
            "Restored payload {target}; run `wayexpand upgrade --check` to verify runtime health."
        );
        return Ok(());
    }
    if !config_valid {
        bail!("configuration is invalid; refusing to restart against the new payload")
    }
    if status_snapshot.is_none() {
        bail!("no daemon is responding; start the chosen route with `wayexpand setup --apply`, then rerun `wayexpand upgrade --check`")
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
        return Ok(());
    }
    let Some(unit) = service else {
        bail!("daemon route is not mapped to a managed user service; restart it using its owner, then run `wayexpand upgrade --check`")
    };
    let status = crate::setup::run_setup_command("systemctl", &["--user", "restart", unit])?;
    if !status.success() {
        if rollback_file.is_file() {
            let restored = restore_previous_install(&library, &current, &rollback_file, Some(unit));
            bail!(
                "{unit} restart failed; automatic rollback {}",
                if restored.is_ok() {
                    "restored the previous payload"
                } else {
                    "also failed; inspect the service and run `wayexpand upgrade --rollback`"
                }
            )
        }
        bail!("{unit} restart failed; no saved payload is available for automatic rollback")
    }
    let health = crate::doctor::status::read_daemon_status()
        .and_then(|response| crate::doctor::status::status_as_json(&response));
    let restarted_commit = health
        .as_ref()
        .ok()
        .and_then(|verified| verified.get("daemon_commit"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let compatible = health.as_ref().is_ok_and(|verified| {
        crate::doctor::status::upgrade_status_from_snapshot(verified)["compatible"] == true
    });
    if restarted_commit != cli_commit || !compatible {
        if rollback_file.is_file() {
            let restored = restore_previous_install(&library, &current, &rollback_file, Some(unit));
            bail!("post-restart health check failed (daemon build {restarted_commit}); automatic rollback {}", if restored.is_ok() {"restored the previous payload"} else {"also failed; inspect the service and run `wayexpand upgrade --rollback`"})
        }
        bail!("post-restart health check failed (daemon build {restarted_commit}); no saved payload is available for automatic rollback")
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
                    "action broker health check failed; automatic rollback {}",
                    if restored.is_ok() {
                        "restored the previous payload"
                    } else {
                        "also failed; inspect services and run `wayexpand upgrade --rollback`"
                    }
                )
            }
            bail!("action broker health check failed; no saved payload is available for automatic rollback")
        }
    }
    println!("Restarted {unit}; daemon build {restarted_commit} now matches this CLI. IBus-owned engine processes require a separate IBus restart.");
    Ok(())
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

pub(crate) fn certify_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    if !rest.is_empty() {
        usage_bail!("usage: wayexpand certify [--json]");
    }
    let certified = print_certification(requested_json)?;
    if !certified && !requested_json {
        bail!("certification is incomplete; see the reported unsupported or untested checks");
    }
    Ok(())
}

pub(crate) fn backend_command(mut args: Args) -> Result<()> {
    match args.next().as_deref() {
        None => {
            print_backend_diagnostics(true);
        }
        Some("select") => {
            if args.next().as_deref() != Some("--explain") || args.next().is_some() {
                usage_bail!("usage: wayexpand backend select --explain");
            }
            print_backend_selection_explain();
        }
        Some("profile") => backend_profile_command(args.collect())?,
        Some("test") => match args.next().as_deref() {
            Some("--capture") if args.next().is_none() => backend_capture_test()?,
            Some("--injection") if args.next().is_none() => backend_injection_test()?,
            _ => usage_bail!("usage: wayexpand backend test --capture|--injection"),
        },
        _ => {
            usage_bail!("usage: wayexpand backend [select --explain | profile [ID|--current] [--json] | test --capture|--injection]")
        }
    };
    Ok(())
}

fn backend_profile_command(arguments: Vec<String>) -> Result<()> {
    let mut profile_id = None;
    let mut current = false;
    let mut json = false;
    for argument in arguments {
        match argument.as_str() {
            "--json" => json = true,
            "--current" if profile_id.is_none() && !current => current = true,
            id if !id.starts_with('-') && profile_id.is_none() && !current => {
                profile_id = Some(id.to_owned())
            }
            _ => usage_bail!("usage: wayexpand backend profile [ID|--current] [--json]"),
        }
    }
    let selected = if current {
        let id = match wayexpand_backend_selection::probe_capabilities().compositor {
            wayexpand_backend_selection::Compositor::KdePlasma => Some("kde-wayland"),
            wayexpand_backend_selection::Compositor::Gnome => Some("gnome-wayland"),
            wayexpand_backend_selection::Compositor::Sway => Some("sway"),
            wayexpand_backend_selection::Compositor::Hyprland => Some("hyprland"),
            _ => None,
        };
        id.and_then(wayexpand_backend_selection::desktop_integration_profile)
    } else if let Some(id) = profile_id.as_deref() {
        Some(
            wayexpand_backend_selection::desktop_integration_profile(id)
                .ok_or_else(|| usage_error(format!("unknown desktop profile {id:?}")))?,
        )
    } else {
        None
    };
    if let Some(profile) = selected {
        let live_recommendation = if current {
            let capabilities = wayexpand_backend_selection::probe_capabilities();
            let policy = load_policy()?;
            Some(crate::setup::recommended_setup_backend(
                &capabilities,
                &policy,
            ))
        } else {
            None
        };
        if json {
            if let Some(live) = live_recommendation.as_ref() {
                println!(
                    "{}",
                    serde_json::json!({
                        "profile": profile,
                        "live_assessment": {
                            "recommended_backend": live.backend,
                            "label": live.label,
                            "reason": live.detail,
                        }
                    })
                );
            } else {
                println!("{}", serde_json::to_string(profile)?);
            }
        } else {
            print_desktop_profile(profile);
            if let Some(live) = live_recommendation {
                println!(
                    "Live capability/policy recommendation: {} — {}",
                    live.label, live.detail
                );
            }
        }
        return Ok(());
    }
    if json {
        println!(
            "{}",
            serde_json::to_string(wayexpand_backend_selection::desktop_integration_profiles())?
        );
    } else {
        for profile in wayexpand_backend_selection::desktop_integration_profiles() {
            println!(
                "{} — {} [{}]",
                profile.id, profile.desktop, profile.recommended_route_id
            );
        }
        println!(
            "Use `wayexpand backend profile --current` to inspect the detected desktop profile."
        );
    }
    Ok(())
}

fn print_desktop_profile(profile: &wayexpand_backend_selection::DesktopIntegrationProfile) {
    println!("{} — {}", profile.id, profile.desktop);
    println!("Recommended route: {}", profile.recommended_route_id);
    println!(
        "Injection backends: {}",
        profile.supported_injection_backends.join(", ")
    );
    println!("Required permissions:");
    for permission in &profile.required_permissions {
        println!("  - {permission}");
    }
    println!("Setup instructions:");
    for instruction in &profile.setup_instructions {
        println!("  - {instruction}");
    }
    println!("Known limitations:");
    for limitation in &profile.known_limitations {
        println!("  - {limitation}");
    }
    let verified_applications = if profile.verified_applications.is_empty() {
        "none".to_owned()
    } else {
        profile.verified_applications.join(", ")
    };
    println!("Verified applications: {verified_applications}");
    println!(
        "Last certification result: {}",
        profile.last_certification_result
    );
}

/// One-shot evdev capture check. The source is non-exclusive, so keystrokes
/// still reach the focused application normally; only activity counts are
/// retained and no key values are printed or written to a log.
fn backend_capture_test() -> Result<()> {
    let policy = load_policy()?;
    if !setup_backend_allowed(&policy, "evdev") {
        bail!("organization policy does not permit evdev capture");
    }
    println!("Evdev capture test — one-shot, 15 second limit");
    println!("Privacy: this process can observe global keyboard events during the test. Key values are neither printed nor stored, but the focused application still receives your typing.");
    print!("Use a disposable text field and explicitly allow this capture test? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Capture test cancelled; no keyboard device was opened.");
        return Ok(());
    }

    let mut source = wayexpand_backend_evdev::EvdevSource::connect()
        .context("evdev capture test could not open a readable keyboard")?;
    println!("Capture device: connected");
    if !wayexpand_core::InputSource::capabilities(&source).layout_aware {
        println!("Matching: unavailable — no trustworthy active keyboard layout is attached to this test source.");
    }
    println!("Type a few harmless characters in the disposable field now; the test will report activity only.");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match source.next_event_timeout(remaining.min(std::time::Duration::from_millis(250))) {
            Ok(Some(_event)) => {
                println!("Capture: PASS — sanitized physical-input activity observed; event contents discarded.");
                return Ok(());
            }
            Ok(None) => {}
            Err(error) => return Err(anyhow::anyhow!("capture test stopped: {error}")),
        }
    }
    bail!("capture test timed out without observing input activity; no key values were recorded")
}

/// Injects a fixed harmless marker only after the user focuses a controlled
/// target and explicitly confirms. The route acknowledgment is not target
/// verification; the operator must inspect the target before the test passes.
fn backend_injection_test() -> Result<()> {
    let policy = load_policy()?;
    if !policy
        .effective_enforcement_policy()
        .backend_allowed("libei")
    {
        bail!("organization policy does not permit libei injection");
    }
    println!("libei injection test — no deletion and no Enter key");
    println!("Connecting to the explicitly selected EIS path. A desktop portal may request authorization.");
    let options = wayexpand_backend_libei::LibeiOptions {
        persist_portal_token: false,
        ..wayexpand_backend_libei::LibeiOptions::default()
    };
    let mut injector = wayexpand_backend_libei::LibeiInjector::connect(options)
        .context("libei injection test could not establish and negotiate an EIS session")?;
    println!(
        "EIS session: connected; negotiated insertion mode: {}",
        wayexpand_core::TextInjector::status_detail(&injector)
    );
    println!("After confirmation, you will have five seconds to focus a controlled, disposable text field. The test inserts only `WayExpand injection test` at the current caret.");
    print!("Proceed with this insertion? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        wayexpand_core::TextInjector::shutdown(Box::new(injector));
        println!("Injection test cancelled; no text was inserted.");
        return Ok(());
    }
    println!("Focus the selected text field now; injection starts in five seconds. Return to this terminal afterward to verify the marker.");
    for seconds_left in (1..=5).rev() {
        println!("{seconds_left}…");
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    const MARKER: &str = "WayExpand injection test";
    if let Err(error) = wayexpand_core::TextInjector::preflight_text(&mut injector, MARKER) {
        wayexpand_core::TextInjector::shutdown(Box::new(injector));
        return Err(anyhow::anyhow!(
            "test marker was rejected before insertion: {error}"
        ));
    }
    if let Err(error) = wayexpand_core::TextInjector::insert(&mut injector, MARKER) {
        wayexpand_core::TextInjector::shutdown(Box::new(injector));
        bail!("injection outcome may be partial or uncertain ({error}); inspect the target and do not retry automatically")
    }
    wayexpand_core::TextInjector::shutdown(Box::new(injector));
    println!("Injection request: acknowledged by the backend; target application result is not yet verified.");
    print!("Did the exact marker appear once in the selected field? [y/N] ");
    io::stdout().flush()?;
    answer.clear();
    io::stdin().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Injection: operator-confirmed in the selected target. This does not certify other applications or routes.");
        Ok(())
    } else {
        bail!("injection was not confirmed; inspect the target manually and do not repeat until its contents are understood")
    }
}

pub(crate) fn explain_backend_command(mut args: Args) -> Result<()> {
    if args.next().is_some() {
        usage_bail!("usage: wayexpand explain-backend");
    }
    print_backend_selection_explain();
    Ok(())
}

pub(crate) fn portal_command(mut args: Args) -> Result<()> {
    match args.next().as_deref() {
        Some("status") => {
            if args.next().is_some() {
                usage_bail!("usage: wayexpand portal status|reset");
            }
            let path = portal_token_path().context("cannot determine config directory")?;
            println!(
                "portal restoration token: {} ({})",
                if path.is_file() {
                    "present"
                } else {
                    "not present"
                },
                path.display()
            );
        }
        Some("reset") => {
            if args.next().is_some() {
                usage_bail!("usage: wayexpand portal status|reset");
            }
            let removed =
                reset_portal_token().context("removing the libei portal restoration token")?;
            println!(
                "{}",
                if removed {
                    "removed the stored portal restoration token"
                } else {
                    "no stored portal restoration token was present"
                }
            );
            println!("the next libei connection may ask for portal access again");
        }
        _ => usage_bail!("usage: wayexpand portal status|reset"),
    };
    Ok(())
}

/// `wayexpand explain`: ask the running daemon, which knows the focused
/// application, field sensitivity, composition, pause state, and output
/// route; without a daemon (or with `--offline`/`--app`), explain from the
/// configuration and organization policy alone.
pub(crate) fn explain_command(mut args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand explain <text> [--json] [--offline] [--app ID] [config]";
    let text = args.next().ok_or_else(|| usage_error(USAGE))?;
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let offline_flag = rest.iter().position(|arg| arg == "--offline");
    let offline = offline_flag.is_some();
    if let Some(index) = offline_flag {
        rest.remove(index);
    }
    let app = take_option(&mut rest, "--app")?;
    if rest.len() > 1 {
        usage_bail!("{USAGE}");
    }
    if text.is_empty() || text.chars().any(char::is_control) {
        usage_bail!("the text to explain cannot be empty or contain control characters");
    }
    let config_path = rest.into_iter().next().map(PathBuf::from);

    let mut daemon_unreachable = None;
    if !offline && app.is_none() && config_path.is_none() {
        match control_operation(wayexpand_core::DaemonOperation::Status) {
            Ok(status) => match live_explain_build_gate(&status, crate::build_info::COMMIT) {
                None => match control_operation(wayexpand_core::DaemonOperation::Explain {
                    text: text.clone(),
                    json: requested_json,
                }) {
                    Ok(response) if !response.starts_with("unknown command") => {
                        print!("{response}");
                        return Ok(());
                    }
                    Ok(_) => {
                        daemon_unreachable =
                            Some("the running daemon is too old to explain matches".to_owned())
                    }
                    Err(error) => daemon_unreachable = Some(format!("not reachable ({error})")),
                },
                Some(reason) => daemon_unreachable = Some(reason.to_owned()),
            },
            Err(error) => daemon_unreachable = Some(format!("not reachable ({error})")),
        }
    }

    let path = config_path.unwrap_or_else(default_config_path);
    let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
    let policy = load_policy()
        .map_err(|error| config_error(format!("organization policy is invalid: {error}")))?;
    let mut engine = ExpansionEngine::new(config).map_err(|error| {
        config_error(format!("configuration invalid: {}", error.safe_summary()))
    })?;
    engine
        .apply_administrator_policy(&policy)
        .map_err(|error| {
            config_error(format!("configuration invalid: {}", error.safe_summary()))
        })?;
    if let Some(app) = &app {
        engine.process(InputEvent::WindowChanged(Some(
            wayexpand_core::WindowContext {
                app_id: Some(app.clone()),
                title: None,
                instance_id: None,
            },
        )));
    }
    let selection = wayexpand_backend_selection::auto_select(None, None).ok();
    let backend = selection
        .as_ref()
        .map(|selection| {
            wayexpand_core::policy_backend_name(selection.pair.source(), selection.pair.backend())
        })
        .unwrap_or("none");
    let mut explanation = engine.explain(&text, backend);
    match daemon_unreachable {
        Some(reason) => explanation.push(
            "daemon",
            wayexpand_core::CheckStatus::Fail,
            format!("{reason}; live session state is unavailable for verification"),
        ),
        None => explanation.push(
            "daemon",
            wayexpand_core::CheckStatus::Info,
            "offline explanation from the configuration; live focus, field, and output state \
             are not included",
        ),
    }
    if requested_json {
        let suppressed_by = explanation.suppressed_by().map(|check| check.name);
        println!(
            "{}",
            serde_json::json!({
                "would_expand": suppressed_by.is_none(),
                "suppressed_by": suppressed_by,
                "typed": explanation.typed,
                "snippet": explanation.snippet,
                "checks": explanation.checks,
                "offline": true,
            })
        );
    } else {
        print!("{}", explanation.render_text());
    }
    Ok(())
}

/// Live explanations depend on safety decisions made by the daemon. Never
/// present an older or unidentifiable daemon's answer as authoritative: its
/// route capabilities or form safeguards may differ from this CLI build.
pub(crate) fn live_explain_build_gate(
    status_response: &str,
    cli_commit: &str,
) -> Option<&'static str> {
    let status = wayexpand_core::DaemonStatus::parse(status_response);
    if status.status_schema() != Some(CONTROL_STATUS_SCHEMA) {
        return Some(
            "the running daemon uses an incompatible status protocol; restart it before relying on live explanations",
        );
    }
    let daemon_commit = status.field("daemon_commit");
    if cli_commit != "unknown"
        && daemon_commit.is_some_and(|commit| commit != "unknown" && commit == cli_commit)
    {
        None
    } else if cli_commit == "unknown" || daemon_commit.is_none_or(|commit| commit == "unknown") {
        Some(
            "the running daemon build cannot be verified; restart it before relying on live explanations",
        )
    } else {
        Some(
            "the running daemon is from a different build; restart it before relying on live explanations",
        )
    }
}

#[cfg(test)]
mod policy_check_tests {
    use super::policy_check_findings;
    use wayexpand_core::OrganizationPolicy;

    #[test]
    fn policy_check_explains_audit_only_controls_and_can_allow_them_explicitly() {
        let policy = OrganizationPolicy {
            safe_mode: false,
            disable_commands: true,
            ..OrganizationPolicy::default()
        };
        let report = crate::doctor::policy::effective_policy_report(&policy);
        let findings = policy_check_findings(&policy, &report, false);
        assert!(findings.iter().any(|finding| {
            finding["control"] == "disable_commands"
                && finding["status"] == "fail"
                && finding["reason"]
                    .as_str()
                    .unwrap()
                    .contains("safe_mode is false")
        }));
        let allowed = policy_check_findings(&policy, &report, true);
        assert!(allowed.iter().all(|finding| finding["status"] != "fail"));
    }

    #[test]
    fn policy_check_fails_known_unsupported_controls_even_in_audit_mode() {
        let policy = OrganizationPolicy {
            safe_mode: true,
            require_composition_aware: true,
            ..OrganizationPolicy::default()
        };
        let report = crate::doctor::policy::effective_policy_report(&policy);
        let findings = policy_check_findings(&policy, &report, true);
        assert!(findings.iter().any(|finding| {
            finding["control"] == "require_composition_aware" && finding["status"] == "fail"
        }));
    }
}
