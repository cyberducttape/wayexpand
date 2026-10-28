//! Setup and diagnostics commands: setup, doctor, certify, backend selection, and portal tokens.

use super::Args;
use crate::*;

pub(crate) fn setup_command(mut args: Args) -> Result<()> {
    let mut requested_backend: Option<String> = None;
    let mut requested_mode: Option<String> = None;
    let mut assume_yes = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
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
                    "usage: wayexpand setup [--mode recommended|maximum|experimental] [--yes]"
                )
            }
        }
    }
    if requested_backend.is_some() && requested_mode.is_some() {
        usage_bail!("setup accepts either --mode or expert --backend, not both");
    }
    println!("WayExpand setup");
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
    println!("  Maximum compatibility explicit evdev fallback; observes global input");
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
    configure_setup_backend(&backend)?;
    Ok(())
}

pub(crate) fn doctor_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
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
        _ => usage_bail!("usage: wayexpand backend [select --explain]"),
    };
    Ok(())
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
            Ok(status) => match live_explain_build_gate(&status) {
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
pub(crate) fn live_explain_build_gate(status_response: &str) -> Option<&'static str> {
    let status = wayexpand_core::DaemonStatus::parse(status_response);
    if status.status_schema() != Some(CONTROL_STATUS_SCHEMA) {
        return Some(
            "the running daemon uses an incompatible status protocol; restart it before relying on live explanations",
        );
    }
    let daemon_commit = status.field("daemon_commit");
    if crate::doctor::status::daemon_commit_matches(daemon_commit) {
        None
    } else if crate::doctor::status::daemon_commit_unknown(daemon_commit) {
        Some(
            "the running daemon build cannot be verified; restart it before relying on live explanations",
        )
    } else {
        Some(
            "the running daemon is from a different build; restart it before relying on live explanations",
        )
    }
}
