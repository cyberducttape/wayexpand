//! Setup and diagnostics commands: setup, doctor, certify, backend selection, and portal tokens.

use super::Args;
use crate::args::take_flag;
use crate::*;

pub(crate) fn setup_command(mut args: Args) -> Result<()> {
    let mut requested_backend: Option<String> = None;
    let mut requested_mode: Option<String> = None;
    let mut assume_yes = false;
    let mut accept_non_atomic_replacement = false;
    let mut install_system_integration = false;
    let mut terminals = false;
    let mut check_only = false;
    let mut apply_requested = false;
    let mut json = false;
    let mut cancel_resume = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--check" => check_only = true,
            "--apply" => apply_requested = true,
            "--json" => json = true,
            "--cancel-resume" => cancel_resume = true,
            "--terminals" => terminals = true,
            "--experimental-input-method-v2" => {
                requested_backend = Some("input-method".into());
            }
            "--yes" | "-y" => assume_yes = true,
            "--accept-non-atomic-replacement" => accept_non_atomic_replacement = true,
            "--install-system-integration" => install_system_integration = true,
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
                    "usage: wayexpand setup [--mode recommended|maximum|experimental | --backend ibus|input-method|evdev] [--terminals] [--yes] [--accept-non-atomic-replacement] [--install-system-integration]"
                )
            }
        }
    }
    if cancel_resume {
        if check_only
            || apply_requested
            || requested_backend.is_some()
            || requested_mode.is_some()
            || assume_yes
            || json
            || terminals
            || accept_non_atomic_replacement
            || install_system_integration
        {
            usage_bail!("`wayexpand setup --cancel-resume` cannot be combined with other options");
        }
        wayexpand_core::clear_gnome_ibus_setup_resume(&default_config_path())
            .map_err(|error| anyhow::anyhow!("could not cancel setup continuation: {error}"))?;
        println!("GNOME setup continuation cancelled. No backend or configuration was changed.");
        return Ok(());
    }
    if check_only && apply_requested {
        usage_bail!("choose either `wayexpand setup --check` or `wayexpand setup --apply`");
    }
    if check_only {
        if requested_backend.is_some()
            || requested_mode.is_some()
            || terminals
            || assume_yes
            || accept_non_atomic_replacement
            || install_system_integration
        {
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
    let is_gnome = wayexpand_backend_selection::Compositor::detect()
        == wayexpand_backend_selection::Compositor::Gnome;
    let is_gnome_wayland = is_gnome
        && std::env::var("XDG_SESSION_TYPE")
            .is_ok_and(|session| session.eq_ignore_ascii_case("wayland"));
    if is_gnome_wayland {
        println!("GNOME Wayland detected. Existing keyboard sources and layout will be preserved.");
        println!("WayExpand will be added as an IBus source; its engine uses the system keyboard layout.");
    }
    println!();
    let config_path = default_config_path();
    let resume_pending = wayexpand_core::gnome_ibus_setup_resume_pending_for_session(
        &config_path,
        is_gnome,
        is_gnome_wayland,
    );
    let resume_marker_exists = wayexpand_core::gnome_ibus_setup_resume_marker_present(&config_path);
    if resume_pending {
        println!("Resuming the GNOME IBus setup prepared before the previous logout.");
    } else if resume_marker_exists {
        println!("A GNOME IBus setup continuation is stale, unsupported, or belongs to another session; it will not affect backend selection. Use `wayexpand setup --cancel-resume` to discard it.");
    }
    let capabilities = probe_capabilities();
    let policy = load_policy()?;
    // A packaged IBus component can be installed but absent from the running
    // registry until setup refreshes IBus. Treat that as an available setup
    // route so the one-command first-run flow can register/activate it.
    let ibus_setup_available = crate::setup_ibus::setup_route_available()
        || ibus_engine_available()
        || ibus_engine_installed();
    let ibus_route_ready = ibus_engine_available() || ibus_engine_installed();
    let automatic =
        recommended_setup_backend_with_ibus(&capabilities, ibus_setup_available, &policy);
    let route_plan =
        wayexpand_backend_selection::plan_routes(&capabilities, ibus_route_ready, |route| {
            route_allowed_by_policy(&policy, route)
        });
    if automatic.backend == "ibus" && !ibus_route_ready && ibus_setup_available {
        println!("IBus is ready to configure, but its component is not registered in the running session yet. Setup will register it and refresh IBus; no IBus shell commands are needed.");
    }
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
    } else if resume_pending {
        "ibus".to_owned()
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
        eprintln!("SETUP RESULT: ACTION REQUIRED");
        bail!(
                "Recommended mode found no safe automatic path; use --mode experimental or configure permissions and rerun setup"
            );
    }
    if !setup_backend_allowed(&policy, &backend) {
        eprintln!("SETUP RESULT: ACTION REQUIRED");
        bail!("setup backend '{backend}' is disallowed by organization policy")
    }
    if accept_non_atomic_replacement && backend != "ibus" {
        usage_bail!("--accept-non-atomic-replacement applies only to the IBus backend");
    }
    if install_system_integration && backend != "ibus" {
        usage_bail!("--install-system-integration applies only to the IBus backend");
    }
    Config::ensure_user_config(default_config_path()).map_err(|error| {
        config_error(format!(
            "could not initialize configuration: {}",
            error.safe_summary()
        ))
    })?;
    let terminals_requested = terminals;
    let setup_result = configure_setup_backend(
        &backend,
        crate::setup_ibus::IbusSetupOptions {
            assume_yes,
            accept_non_atomic_replacement,
            // Terminal capture is a separately authorized compatibility
            // route. Set it up only after the core IBus path passes a real
            // insertion check below.
            terminals: false,
            real_insertion_verified: false,
            install_system_integration,
        },
    );
    if let Err(error) = setup_result {
        eprintln!("SETUP RESULT: ACTION REQUIRED");
        return Err(error);
    }
    if backend == "ibus" && resume_pending {
        wayexpand_core::clear_gnome_ibus_setup_resume(&config_path).map_err(|error| {
            anyhow::anyhow!(
                "IBus setup completed, but its continuation marker could not be cleared: {error}"
            )
        })?;
    }
    let test_config = Config::load(default_config_path())?;
    let test_trigger = setup_test_trigger(&test_config);
    let mut desktop_test_result = crate::commands::debug::DesktopTestResult::NotTested;
    if let Some(test_trigger) = test_trigger.as_deref() {
        if assume_yes {
            println!("Activation completed. Real application insertion: NOT TESTED.");
            println!("Next: run `wayexpand debug --test {test_trigger:?}` in a disposable text field and confirm the replacement.");
        } else if crate::prompt_yes_no(
            &format!("Activation checks completed; real insertion is not yet verified. Run the optional test with {test_trigger:?} now? [Y/n] "),
            true,
        )? {
            desktop_test_result = match crate::commands::debug::desktop_test(test_trigger) {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("SETUP RESULT: ACTION REQUIRED");
                    return Err(error);
                }
            };
        } else {
            println!("Activation completed. Real application insertion: NOT TESTED.");
            println!("Next: run `wayexpand debug --test {test_trigger:?}` in a disposable text field and confirm the replacement.");
        }
    } else {
        println!("Activation completed. No enabled, safe-to-test expansion is configured; real application insertion remains NOT TESTED.");
        println!("Setup is configured, but insertion is unverified. Add a harmless static expansion and run `wayexpand debug --test TRIGGER` in a disposable text field when ready.");
    }
    if desktop_test_result == crate::commands::debug::DesktopTestResult::Failed {
        println!("SETUP RESULT: ACTION REQUIRED");
        println!("The real application test did not verify an exact insertion. Follow the reported stage and do not retry blindly.");
        bail!("WX-SETUP-TEST-002: the real application insertion test failed or was not confirmed");
    }
    let real_insertion_verified =
        desktop_test_result == crate::commands::debug::DesktopTestResult::Verified;
    if real_insertion_verified {
        println!("SETUP RESULT: WORKING");
        println!("A real expansion was user-confirmed in the tested application. Other applications and persistence across logout remain unverified.");
    } else {
        println!("SETUP RESULT: CONFIGURED, AWAITING TEST");
        if test_trigger.is_some() {
            println!("Next: run `wayexpand debug --test {}` in a disposable text field and confirm the replacement.", test_trigger.as_deref().unwrap_or_default());
        }
    }
    if backend == "ibus" {
        let enable_terminals = if real_insertion_verified && !terminals_requested && !assume_yes {
            println!("\nOptional terminal coverage uses evdev and can observe raw keyboard input, including password fields. It requires separate permission and compatible output injection.");
            crate::prompt_yes_no(
                "Configure terminal coverage as a separate evdev service? [y/N] ",
                false,
            )?
        } else {
            terminals_requested
        };
        if enable_terminals && real_insertion_verified {
            if let Err(error) = configure_setup_backend(
                "ibus",
                crate::setup_ibus::IbusSetupOptions {
                    assume_yes,
                    accept_non_atomic_replacement,
                    terminals: true,
                    real_insertion_verified: true,
                    install_system_integration,
                },
            ) {
                eprintln!("SETUP RESULT: WORKING (core IBus); optional terminal coverage: ACTION REQUIRED.");
                return Err(error);
            }
        } else if terminals_requested {
            println!("Optional terminal coverage was not configured because a real IBus insertion was not verified.");
            println!("After verifying a harmless expansion in a disposable text field, rerun `wayexpand setup --backend ibus --terminals`.");
        }
    }
    Ok(())
}

/// Pick an enabled, static expansion for the optional real-client test. The
/// conventional sample remains preferred when it is testable, but custom
/// configurations need not keep it. Command-backed and templated entries are
/// excluded so setup never executes actions or starts an incomplete form.
pub(crate) fn setup_test_trigger(config: &Config) -> Option<String> {
    let is_testable = |expansion: &&wayexpand_core::ExpansionConfig| {
        expansion.enabled && expansion.command.is_none() && !expansion.replacement.contains("{{")
    };
    config
        .expansion
        .iter()
        .find(|expansion| is_testable(expansion) && expansion.answers_to(";;hello"))
        .or_else(|| config.expansion.iter().find(is_testable))
        .map(|expansion| expansion.trigger.clone())
}

pub(crate) fn doctor_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let requested_security = rest
        .iter()
        .position(|argument| argument == "--security")
        .map(|index| rest.remove(index))
        .is_some();
    let requested_gnome = rest
        .iter()
        .position(|argument| argument == "--gnome")
        .map(|index| rest.remove(index))
        .is_some();
    if requested_gnome {
        if requested_security
            || rest.iter().any(|argument| argument == "--certify")
            || !rest.is_empty()
        {
            usage_bail!("usage: wayexpand doctor --gnome [--json]");
        }
        return crate::doctor::gnome::run(requested_json);
    }
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
    println!("Feature limits: IME preedit/composition is unsupported; app_filter window tracking needs KWin (KDE Plasma) or the WayExpand GNOME Shell extension (GNOME 45+); it fails closed elsewhere.");
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
    let report = serde_json::json!({
        "schema":1, "action":action, "cli_commit":cli_commit, "cli_version":cli_version,
        "current_install":current_target.as_ref().map(|v|v.to_string_lossy().into_owned()).or_else(|| installed_cli.as_ref().map(|p|p.display().to_string())),
        "daemon_commit":daemon_commit, "daemon_version":daemon_version, "daemon_running":status_snapshot.is_some(),
        "config_valid":config_valid, "version_mismatch":mismatch,
        "restart_service":service, "rollback_available":rollback_file.is_file(),
        "ibus_stale_engine_processes":stale_ibus_processes_json,
        "ibus_engine_note":"IBus engine processes are owned by ibus-daemon; restarting a WayExpand systemd unit does not reload a stale engine."
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
            println!(
                "Daemon: {}",
                daemon_version.or(daemon_commit).unwrap_or("not responding")
            );
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
        if !stale_ibus_processes.is_empty() {
            print_ibus_engine_restart_guidance(&stale_ibus_processes);
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
            bail!(upgrade_health_failure(
                cli_version,
                restarted_build,
                health.is_ok(),
                restored.is_ok()
            ))
        }
        bail!(upgrade_health_failure(
            cli_version,
            restarted_build,
            health.is_ok(),
            false
        ))
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
    println!("Restarted {unit}; daemon build {restarted_build} now matches this CLI.");
    let stale_ibus_processes = crate::setup_ibus::stale_engine_processes();
    if !stale_ibus_processes.is_empty() {
        print_ibus_engine_restart_guidance(&stale_ibus_processes);
    }
    Ok(())
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
    println!("Restart IBus to load the updated engine: `ibus restart` (input pauses briefly).");
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
        Some("list") => backend_list_command(args.collect())?,
        Some("test") => match args.next().as_deref() {
            Some("--capture") if args.next().is_none() => backend_capture_test()?,
            Some("--injection") if args.next().is_none() => backend_injection_test()?,
            _ => usage_bail!("usage: wayexpand backend test --capture|--injection"),
        },
        _ => {
            usage_bail!("usage: wayexpand backend [list [--json] | select --explain | profile [ID|--current] [--json] | test --capture|--injection]")
        }
    };
    Ok(())
}

fn backend_list_command(arguments: Vec<String>) -> Result<()> {
    let json = match arguments.as_slice() {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => usage_bail!("usage: wayexpand backend list [--json]"),
    };
    let capabilities = probe_capabilities();
    let policy = load_policy()?;
    let plan =
        wayexpand_backend_selection::plan_routes(&capabilities, ibus_engine_available(), |route| {
            route_allowed_by_policy(&policy, route)
        });
    let routes = plan
        .routes
        .iter()
        .map(|planned| {
            let contract = planned.contract;
            let assessment = &planned.assessment;
            serde_json::json!({
                "id": contract.id,
                "label": contract.label,
                "capture": contract.capture,
                "injection": contract.injection,
                "standing": planned.standing.as_str(),
                "reason": planned.reason,
                "assessment": {
                    "installed": assessment.installed.as_str(),
                    "supported": assessment.supported.as_str(),
                    "authorized": assessment.authorized.as_str(),
                    "connected": assessment.connected.as_str(),
                    "policy_allowed": assessment.policy_allowed,
                    "certified": assessment.certified,
                    "requires_user_action": assessment.requires_user_action,
                    "limitations": assessment.limitations.iter().map(|item| item.description()).collect::<Vec<_>>(),
                    "portal": assessment.portal,
                },
                "replacement_assurance": contract.replacement_assurance,
                "atomic_replace": contract.atomic_replace,
                "sensitive_field_detection": contract.sensitive_fields,
            })
        })
        .collect::<Vec<_>>();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "session": capabilities.compositor.name(),
                "routes": routes,
                "production_certification_claimed": false,
            }))?
        );
    } else {
        println!("Backend routes for {}:", capabilities.compositor.name());
        for route in routes {
            println!(
                "{} · {} · capture={} · injection={}",
                route["id"].as_str().unwrap_or("unknown"),
                route["standing"].as_str().unwrap_or("unknown"),
                route["capture"].as_str().unwrap_or("unknown"),
                route["injection"].as_str().unwrap_or("unknown"),
            );
            let assessment = &route["assessment"];
            println!(
                "  installed={} supported={} authorized={} connected={} policy_allowed={} certified={}",
                assessment["installed"].as_str().unwrap_or("unknown"),
                assessment["supported"].as_str().unwrap_or("unknown"),
                assessment["authorized"].as_str().unwrap_or("unknown"),
                assessment["connected"].as_str().unwrap_or("unknown"),
                assessment["policy_allowed"],
                assessment["certified"],
            );
            for limitation in assessment["limitations"].as_array().into_iter().flatten() {
                if let Some(limitation) = limitation.as_str() {
                    println!("  limitation: {limitation}");
                }
            }
        }
        println!("Routes are not production-certified unless their certification evidence explicitly says so.");
    }
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
            Ok(status) => match live_explain_build_gate(
                &status,
                crate::build_info::COMMIT,
                crate::build_info::VERSION,
            ) {
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
    cli_version: &str,
) -> Option<&'static str> {
    let status = wayexpand_core::DaemonStatus::parse(status_response);
    if status.status_schema() != Some(CONTROL_STATUS_SCHEMA) {
        return Some(
            "the running daemon uses an incompatible status protocol; restart it before relying on live explanations",
        );
    }
    let daemon_commit = status.field("daemon_commit");
    let daemon_version = status.field("daemon_version");
    let commit_matches = cli_commit != "unknown"
        && daemon_commit.is_some_and(|commit| commit != "unknown" && commit == cli_commit);
    let build_identity_unverified = cli_version.contains(".dirty-unknown")
        || daemon_version.is_some_and(|version| version.contains(".dirty-unknown"));
    let build_matches = !build_identity_unverified
        && daemon_version.map_or_else(
            || !cli_version.contains(".dirty.") && commit_matches,
            |version| commit_matches && version == cli_version,
        );
    if build_matches {
        None
    } else if cli_commit == "unknown"
        || cli_version.contains(".dirty-unknown")
        || daemon_version.map_or_else(
            || {
                cli_commit == "unknown"
                    || daemon_commit.is_none_or(|commit| commit == "unknown")
                    || (cli_version.contains(".dirty.") && daemon_commit == Some(cli_commit))
            },
            |version| {
                version == "unknown"
                    || version.contains(".dirty-unknown")
                    || daemon_commit.is_none_or(|c| c == "unknown")
            },
        )
    {
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
    use super::{policy_check_findings, setup_test_trigger, upgrade_health_failure};
    use wayexpand_core::{Config, OrganizationPolicy};

    #[test]
    fn setup_uses_hello_when_present_but_does_not_require_it() {
        let config = Config::parse(
            r#"[[expansion]]
trigger = ";custom"
replacement = "custom"

[[expansion]]
trigger = ";;hello"
replacement = "hello"
"#,
        )
        .unwrap();
        assert_eq!(setup_test_trigger(&config).as_deref(), Some(";;hello"));
    }

    #[test]
    fn setup_skips_templated_hello_and_uses_a_static_custom_snippet() {
        let config = Config::parse(
            r#"[[expansion]]
trigger = ";;hello"
replacement = "Hello {{field:name}}"

[[expansion]]
trigger = ";custom"
replacement = "safe static test"
"#,
        )
        .unwrap();
        assert_eq!(setup_test_trigger(&config).as_deref(), Some(";custom"));
    }

    #[test]
    fn setup_can_offer_an_existing_custom_trigger_without_hello() {
        let config = Config::parse(
            r#"[[expansion]]
trigger = ";custom"
replacement = "custom"
"#,
        )
        .unwrap();
        assert_eq!(setup_test_trigger(&config).as_deref(), Some(";custom"));
    }

    #[test]
    fn setup_has_no_test_trigger_for_empty_disabled_or_command_only_configs() {
        let empty = Config::parse("").unwrap();
        assert_eq!(setup_test_trigger(&empty), None);

        let disabled = Config::parse(
            r#"[[expansion]]
trigger = ";disabled"
replacement = "value"
enabled = false
"#,
        )
        .unwrap();
        assert_eq!(setup_test_trigger(&disabled), None);

        let command_only = Config::parse(
            r#"[[expansion]]
trigger = ";action"
replacement = "value"
[expansion.command]
program = "/usr/bin/true"
"#,
        )
        .unwrap();
        assert_eq!(setup_test_trigger(&command_only), None);

        let template_only = Config::parse(
            r#"[[expansion]]
trigger = ";template"
replacement = "value: {{field:value}}"
"#,
        )
        .unwrap();
        assert_eq!(setup_test_trigger(&template_only), None);
    }

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
}
