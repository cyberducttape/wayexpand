//! Setup and diagnostics commands: setup, doctor, policy, certify, explain and portal tokens.

use super::Args;
use crate::args::take_flag;
use crate::*;
use std::io::IsTerminal;

pub(super) fn runnable_cli_command(subcommand: &str) -> String {
    crate::commands::install::make_cli_command_runnable(format!("wayexpand {subcommand}"))
}

pub(super) fn runnable_cli_text(text: &str) -> String {
    let cli = crate::commands::install::make_cli_command_runnable("wayexpand".into());
    text.replace("`wayexpand ", &format!("`{cli} "))
}

fn setup_plan_actions(
    backend: &str,
    is_gnome_wayland: bool,
    accept_non_atomic_replacement: bool,
    install_system_integration: bool,
    terminals_requested: bool,
    evdev_apps_requested: bool,
) -> Vec<&'static str> {
    let mut actions = vec!["Create the default configuration only if it is missing"];
    match backend {
        "ibus" => {
            actions.push("Check IBus prerequisites, shared-manifest safety, and replacement policy before applying integration changes");
            if is_gnome_wayland {
                actions.push("After IBus preflight and explicit replacement consent, prepare GNOME's window tracker and IBus source");
                actions.push("Keep GNOME extension files and enablement settings in the IBus rollback transaction until verification");
            }
            actions.push("Register and activate the IBus engine, repairing a missing or user-scoped shared registration with administrator authorization if needed, then verify the session route");
        }
        "input-method" => actions.push("Enable the input-method-v2 user service; portal authorization may be requested when it connects"),
        "evdev" => {
            actions.push("Configure global keyboard capture only with its separate privacy acknowledgment and device permission");
            if evdev_apps_requested {
                actions.push("Restrict automatic matching to configured terminal and browser application IDs; raw key observation still occurs globally, including password fields");
            }
        }
        _ => {}
    }
    if backend == "ibus" && !accept_non_atomic_replacement {
        actions.push("Stop before changing GNOME or IBus if replacement authorization is not already configured");
    }
    if install_system_integration {
        actions.push("Use the explicitly authorized system-integration repair if the shared IBus registration needs replacement");
    }
    if evdev_apps_requested && backend == "ibus" {
        actions.push(
            "Offer evdev fallback only after the core insertion test; if no app scope exists, add known browser app IDs, while raw capture sees every key and IBus yields in those apps",
        );
    } else if terminals_requested && backend == "ibus" {
        actions.push(
            "Offer optional terminal coverage only after the core insertion test; raw capture sees every key, matching is limited to terminal app IDs",
        );
    }
    actions
}

fn setup_can_apply_without_prompt(
    assume_yes: bool,
    apply_requested: bool,
    stdin_is_tty: bool,
) -> bool {
    assume_yes || apply_requested || stdin_is_tty
}

/// Preserve a route already selected by a compatible running daemon. Setup
/// is also the repair command, so replacing an existing route merely because
/// another backend ranks higher would be surprising and can reduce coverage.
/// Control-only (stdin) daemons are paired with IBus and are handled by the
/// ordinary recommendation path.
fn existing_setup_backend(policy: &OrganizationPolicy) -> Option<&'static str> {
    let response = control_operation(wayexpand_core::DaemonOperation::Status).ok()?;
    existing_setup_backend_from_status(&response, policy)
}

pub(crate) fn existing_setup_backend_from_status(
    response: &str,
    policy: &OrganizationPolicy,
) -> Option<&'static str> {
    let status = wayexpand_core::DaemonStatus::parse(response);
    if status.status_schema() != Some(CONTROL_STATUS_SCHEMA) || !status.field_is_unique("source") {
        return None;
    }
    let backend = match status.field("source")? {
        "evdev" => "evdev",
        "input-method" | "input-method-v2" => "input-method",
        // The IBus engine is the typing route when the daemon only provides
        // the control socket. Let current IBus readiness and policy determine
        // whether it is recommended.
        "stdin" => return None,
        _ => return None,
    };
    setup_backend_allowed(policy, backend).then_some(backend)
}

pub(crate) fn setup_command(mut args: Args) -> Result<()> {
    let mut requested_backend: Option<String> = None;
    let mut requested_mode: Option<String> = None;
    let mut assume_yes = false;
    let mut accept_non_atomic_replacement = false;
    let mut install_system_integration = false;
    let mut terminals = false;
    let mut evdev_apps = false;
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
            "--evdev-apps" => evdev_apps = true,
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
        "usage: wayexpand setup [--mode recommended|maximum|experimental | --backend ibus|input-method|evdev] [--apply] [--terminals | --evdev-apps] [--yes] [--accept-non-atomic-replacement] [--install-system-integration]"
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
            || evdev_apps
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
            || evdev_apps
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
        println!("{}", runnable_cli_text("A GNOME IBus setup continuation is stale, unsupported, or belongs to another session; it will not affect backend selection. Use `wayexpand setup --cancel-resume` to discard it."));
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
    let existing_backend = existing_setup_backend(&policy);
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
    if let Some(existing) = existing_backend {
        println!("Existing configured typing route: {existing}.");
        if existing != automatic.backend {
            println!("Setup keeps this route by default. To switch to the automatic recommendation, pass `--mode recommended`.");
        }
    }
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
    } else if let Some(existing) = existing_backend {
        if assume_yes
            || apply_requested
            || crate::prompt_yes_no(
                &format!("Keep the existing {existing} typing route? [Y/n] "),
                true,
            )?
        {
            existing.to_owned()
        } else {
            prompt_mode_choice(&capabilities, &policy)?
        }
    } else if assume_yes || apply_requested {
        automatic.backend
    } else {
        let prompt = format!("Use Recommended mode ({})? [Y/n] ", automatic.label);
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
    if terminals && backend != "ibus" {
        usage_bail!("--terminals applies only beside the IBus backend");
    }
    if evdev_apps && !matches!(backend.as_str(), "ibus" | "evdev") {
        usage_bail!("--evdev-apps requires an IBus or evdev backend");
    }
    if install_system_integration && backend != "ibus" {
        usage_bail!("--install-system-integration applies only to the IBus backend");
    }
    println!("\nSelected setup plan (read-only):");
    println!("  Backend: {backend}");
    println!(
        "  Configuration: {}",
        if default_config_path().exists() {
            "validate existing configuration"
        } else {
            "create default configuration"
        }
    );
    for action in setup_plan_actions(
        &backend,
        is_gnome_wayland && backend == "ibus",
        accept_non_atomic_replacement,
        install_system_integration,
        terminals,
        evdev_apps,
    ) {
        println!("  - {action}");
    }
    if !setup_can_apply_without_prompt(assume_yes, apply_requested, std::io::stdin().is_terminal())
    {
        println!("Setup plan only; no changes were made. Review the plan, then rerun with `--apply` to continue.");
        return Ok(());
    }
    if !assume_yes && !apply_requested && !prompt_yes_no("Apply this setup plan? [Y/n] ", true)? {
        println!("Setup plan declined. No configuration, GNOME extension, input-source, or service changes were made.");
        return Ok(());
    }
    Config::ensure_user_config(default_config_path()).map_err(|error| {
        config_error(format!(
            "could not initialize configuration: {}",
            error.safe_summary()
        ))
    })?;
    let terminals_requested = terminals;
    let evdev_apps_requested = evdev_apps;
    let setup_result = configure_setup_backend(
        &backend,
        crate::setup_ibus::IbusSetupOptions {
            assume_yes,
            accept_non_atomic_replacement,
            // Terminal capture is a separately authorized compatibility
            // route. Set it up only after the core IBus path passes a real
            // insertion check below.
            terminals: false,
            evdev_apps: backend == "evdev" && evdev_apps_requested,
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
        let quoted_trigger = crate::commands::shell_single_quote(test_trigger);
        let preview_command = runnable_cli_command(&format!("preview {quoted_trigger}"));
        let test_command = runnable_cli_command(&format!("debug --test {quoted_trigger}"));
        if assume_yes {
            println!("Activation completed. Real application insertion: NOT TESTED.");
            println!("Review the expected text first with `{preview_command}`.");
            println!("Next: run `{test_command}` in a disposable text field and confirm the replacement.");
        } else if crate::prompt_yes_no(
            &format!("Activation checks completed; real insertion is not yet verified. Run the optional test with {test_trigger:?} now? [Y/n] "),
            true,
        )? {
            println!("Review the expected text first with `{preview_command}`.");
            desktop_test_result = match crate::commands::debug::desktop_test(test_trigger) {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("SETUP RESULT: ACTION REQUIRED");
                    return Err(error);
                }
            };
        } else {
            println!("Activation completed. Real application insertion: NOT TESTED.");
            println!("Review the expected text first with `{preview_command}`.");
            println!("Next: run `{test_command}` in a disposable text field and confirm the replacement.");
        }
    } else {
        println!("Activation completed. No enabled, safe-to-test expansion is configured; real application insertion remains NOT TESTED.");
        let preview_command = runnable_cli_command("preview 'TRIGGER'");
        let test_command = runnable_cli_command("debug --test 'TRIGGER'");
        println!("Setup is configured, but insertion is unverified. In the GUI, open Test Expansion to create a dedicated harmless snippet and verify it. From the CLI, replace `TRIGGER` below with the exact trigger and keep the quotes so punctuation such as `;` is passed safely: `{preview_command}`, then `{test_command}` in a disposable field.");
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
            if let Some(test_trigger) = test_trigger.as_deref() {
                let quoted_trigger = crate::commands::shell_single_quote(test_trigger);
                let preview_command = runnable_cli_command(&format!("preview {quoted_trigger}"));
                let test_command = runnable_cli_command(&format!("debug --test {quoted_trigger}"));
                println!("Review the expected text first with `{preview_command}`.");
                println!("Next: run `{test_command}` in a disposable text field and confirm the replacement.");
            }
        }
    }
    if backend == "ibus" {
        let enable_evdev_scope = if real_insertion_verified
            && !terminals_requested
            && !evdev_apps_requested
            && !assume_yes
        {
            println!("\nOptional evdev route observes raw keyboard input globally, including password fields. An application-ID allowlist limits matching only; it does not limit keyboard capture. The app list must be configured first; use this for browser applications only when you accept the privacy and non-atomic replacement limits.");
            crate::prompt_yes_no(
                "Configure the global-observation evdev route with app-scoped matching beside IBus? [y/N] ",
                false,
            )?
        } else {
            terminals_requested || evdev_apps_requested
        };
        if enable_evdev_scope && real_insertion_verified {
            if let Err(error) = configure_setup_backend(
                "ibus",
                crate::setup_ibus::IbusSetupOptions {
                    assume_yes,
                    accept_non_atomic_replacement,
                    terminals: terminals_requested,
                    evdev_apps: evdev_apps_requested || !terminals_requested,
                    real_insertion_verified: true,
                    install_system_integration,
                },
            ) {
                eprintln!("SETUP RESULT: WORKING (core IBus); optional evdev app fallback: ACTION REQUIRED.");
                return Err(error);
            }
        } else if terminals_requested || evdev_apps_requested {
            println!("The evdev app fallback was not configured because a real IBus insertion was not verified.");
            println!("{}", runnable_cli_text("After verifying a harmless expansion in a disposable text field, configure settings.raw_capture_apps and rerun `wayexpand setup --backend ibus --evdev-apps`."));
        }
    }
    Ok(())
}

/// Pick an enabled, static expansion for the optional real-client test. The
/// conventional sample remains preferred when it is testable, but custom
/// configurations need not keep it. Command-backed and templated entries are
/// excluded so setup never executes actions or starts an incomplete form.
pub(crate) fn setup_test_trigger(config: &Config) -> Option<String> {
    const TUTORIAL_REPLACEMENT: &str = "WayExpand is working!";
    let is_testable = |expansion: &&wayexpand_core::ExpansionConfig| {
        expansion.is_keyboard_matchable()
            && expansion.app_filter.is_empty()
            && expansion.command.is_none()
            && !expansion.replacement.contains("{{")
    };
    config
        .expansion
        .iter()
        // Keep CLI setup and the GUI's first-run test on the same harmless
        // tutorial snippet when it exists. Otherwise setup --check can show
        // one trigger while the GUI asks the user to test another.
        .find(|expansion| {
            is_testable(expansion)
                && expansion.replacement == TUTORIAL_REPLACEMENT
                && expansion.tags.iter().any(|tag| tag == "tutorial")
        })
        .or_else(|| {
            config
                .expansion
                .iter()
                .find(|expansion| is_testable(expansion) && expansion.answers_to(";;hello"))
        })
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
            .map(crate::args::config_path_arg)
            .transpose()?
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
        .map(crate::args::config_path_arg)
        .transpose()?
        .unwrap_or_else(default_config_path);
    if requested_json {
        let (healthy, failure_hint) = print_json_diagnostics(&config_path)?;
        if !healthy {
            if let Some(hint) = failure_hint {
                bail!("doctor found an unhealthy selected IBus route: {hint}");
            }
            bail!("{}", runnable_cli_text("doctor reported unhealthy host readiness or runtime health; run `wayexpand why` for the prioritized next action, or inspect the JSON checks for detail"));
        }
        return Ok(());
    }
    println!(
        "WayExpand {} (commit {})",
        build_info::VERSION,
        build_info::COMMIT
    );
    let quick_diagnosis =
        crate::commands::install::make_cli_command_runnable("wayexpand why".into());
    println!(
        "Quick answer for a missed expansion: run `{quick_diagnosis}`; this command shows the full readiness details."
    );
    println!("Session: {}", session_description());
    println!("Feature limits: WayExpand does not observe another IME's active preedit; commit or cancel composition before typing a trigger. Local dead-key/Compose guards and committed-Unicode support depend on the selected route. App-filter window tracking requires KWin or the WayExpand GNOME Shell extension and fails closed when unavailable.");
    let config_ok = print_config_diagnostics(&config_path);
    let control_socket_ok = print_control_socket_diagnostics();
    let output_ok = crate::doctor::status::print_output_health_diagnostics();
    let policy_ok = print_policy_diagnostics();
    let broker_ok = print_broker_diagnostics(
        Config::load_read_only(&config_path)
            .map(|loaded| loaded.config)
            .ok()
            .as_ref(),
    );
    print_action_broker_resource_headroom();
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
        bail!("doctor found configuration, runtime, or backend problems; follow the failed check's Next guidance above");
    }
    Ok(())
}

fn print_action_broker_resource_headroom() {
    let report = crate::services::broker_resource_headroom();
    if report["available"].as_bool() != Some(true) {
        println!(
            "Action Broker resource headroom: unavailable (systemd user manager not responding)"
        );
        return;
    }
    let quantity = |name: &str, unit: &str| {
        let current = report[name]["current"].as_u64();
        let limit = report[name]["limit"].as_u64();
        let remaining = report[name]["remaining"].as_u64();
        match (current, limit, remaining) {
            (Some(current), Some(limit), Some(remaining)) if unit == "MiB" => format!(
                "{} {unit} current / {} {unit} limit ({} {unit} remaining)",
                current / (1024 * 1024),
                limit / (1024 * 1024),
                remaining / (1024 * 1024)
            ),
            (Some(current), Some(limit), Some(remaining)) => {
                format!("{current} / {limit} ({remaining} remaining)")
            }
            _ => "unavailable".into(),
        }
    };
    println!("Action Broker resource headroom:");
    println!("  Memory: {}", quantity("memory_bytes", "MiB"));
    println!("  Tasks: {}", quantity("tasks", "count"));
    println!("  Open files: {}", quantity("open_files", "count"));
    if let Some(causes) = report["failure_cause_hints"]
        .as_array()
        .filter(|items| !items.is_empty())
    {
        let causes = causes
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        println!("  Recent resource failure signatures: {causes}");
    }
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
            "the running daemon's status protocol differs from this CLI; use the active installation's CLI to assess compatibility before deciding whether a restart is needed",
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
            "the running daemon build cannot be verified; use the active installation's CLI to assess compatibility before deciding whether a restart is needed",
        )
    } else {
        Some(
            "the running daemon differs from this CLI; verify this CLI belongs to the active installation before deciding whether a restart is needed",
        )
    }
}

#[cfg(test)]
mod policy_check_tests {
    use super::{
        policy_check_findings, setup_can_apply_without_prompt, setup_plan_actions,
        setup_test_trigger,
    };
    use wayexpand_core::{Config, OrganizationPolicy};

    #[test]
    fn read_only_ibus_setup_plan_discloses_possible_shared_registration_authorization() {
        let actions = setup_plan_actions("ibus", false, false, false, false, false);
        assert!(actions
            .iter()
            .any(|action| { action.contains("shared-manifest safety") }));
        assert!(actions.iter().any(|action| {
            action.contains("user-scoped shared registration")
                && action.contains("administrator authorization")
        }));

        let explicitly_authorized = setup_plan_actions("ibus", true, false, true, false, false);
        assert!(explicitly_authorized
            .iter()
            .any(|action| action.contains("explicitly authorized system-integration repair")));
    }

    #[test]
    fn setup_does_not_apply_a_plan_on_closed_noninteractive_stdin() {
        assert!(!setup_can_apply_without_prompt(false, false, false));
        assert!(setup_can_apply_without_prompt(false, true, false));
        assert!(setup_can_apply_without_prompt(true, false, false));
        assert!(setup_can_apply_without_prompt(false, false, true));
    }

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
}
