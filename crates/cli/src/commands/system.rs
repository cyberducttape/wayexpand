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
    let automatic = recommended_setup_backend(&capabilities, &policy);
    println!();
    println!("Compatibility modes");
    println!("  Recommended         safest detected path; run doctor/certify for verification");
    println!("  Maximum compatibility broad application coverage; may observe global input");
    println!("  Experimental         protocol paths whose key pass-through is not certified");
    println!();
    println!("Automatic recommendation: {}", automatic.label);
    println!("  {}", automatic.detail);
    let backend = if let Some(backend) = requested_backend {
        backend
    } else if let Some(mode) = requested_mode {
        setup_backend_for_mode(&mode, &capabilities, &policy)?
    } else if assume_yes {
        automatic.backend.to_owned()
    } else {
        let prompt = format!("Configure Recommended mode ({})? [Y/n] ", automatic.label);
        if prompt_yes_no(&prompt, true)? {
            automatic.backend.to_owned()
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
            bail!("doctor found configuration, policy, or control-socket problems");
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
    let policy_ok = print_policy_diagnostics();
    let broker_ok = print_broker_diagnostics(Config::load(&config_path).ok().as_ref());
    let capture_ready = print_backend_diagnostics(true);
    print_capabilities_diagnostics();
    if !config_ok || !control_socket_ok || !policy_ok || !broker_ok || !capture_ready {
        bail!("doctor found configuration, runtime, or backend problems");
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
