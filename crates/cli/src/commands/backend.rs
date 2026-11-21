//! `backend`: inspect input routes and test capture or output once.

use super::system::runnable_cli_text;
use super::Args;
use crate::*;

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
        .map(backend_route_json)
        .collect::<Vec<_>>();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 2,
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
            println!(
                "  trigger_precondition={} output_transaction={} application_result={}",
                route["trigger_precondition"].as_str().unwrap_or("unknown"),
                route["output_transaction"].as_str().unwrap_or("unknown"),
                route["application_result"].as_str().unwrap_or("unknown"),
            );
            println!(
                "  destination_identity_available={} destination_identity_verified={}",
                route["destination_identity_available"]
                    .as_bool()
                    .unwrap_or(false),
                route["destination_identity_verified"]
                    .as_bool()
                    .unwrap_or(false),
            );
            println!(
                "  physical_input_interruption_observable={} physical_input_interruption_verified={}",
                route["physical_input_interruption_observable"].as_bool().unwrap_or(false),
                route["physical_input_interruption_verified"].as_bool().unwrap_or(false),
            );
            println!(
                "  recovery_outcome_known={}",
                route["recovery_outcome_known"].as_bool().unwrap_or(false),
            );
            for gap in route["production_integrity_gaps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
            {
                println!("  production-integrity gap: {gap}");
            }
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

fn backend_route_json(planned: &wayexpand_backend_selection::PlannedRoute) -> serde_json::Value {
    let contract = planned.contract;
    let assessment = &planned.assessment;
    let integrity_gaps = contract.production_integrity_gaps();
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
        "trigger_precondition": contract.trigger_precondition,
        "output_transaction": contract.output_transaction,
        "application_result": contract.application_result,
        "atomic_replace": contract.atomic_replace,
        "destination_identity_available": contract.destination_identity_available,
        "destination_identity_verified": contract.destination_identity_verified,
        "physical_input_interruption_observable": contract.physical_input_interruption_observable,
        "physical_input_interruption_verified": contract.physical_input_interruption_verified,
        "recovery_outcome_known": contract.recovery_outcome_known,
        "production_integrity_ready": integrity_gaps.is_empty(),
        "production_integrity_gaps": integrity_gaps,
        "sensitive_field_state_observable": contract.sensitive_field_support.implemented,
        "sensitive_field_detection": contract.sensitive_fields,
    })
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
        println!("{}", runnable_cli_text(
            "Use `wayexpand backend profile --current` to inspect the detected desktop profile."
        ));
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

#[cfg(test)]
mod tests {
    use super::backend_route_json;

    #[test]
    fn backend_route_json_does_not_conflate_trigger_check_with_applied_text() {
        let plan = wayexpand_backend_selection::plan_routes(
            &wayexpand_backend_selection::Capabilities::default(),
            true,
            |_| true,
        );
        let ibus = plan
            .routes
            .iter()
            .find(|route| route.contract.id == "ibus")
            .expect("IBus route is present in the shared catalog");
        let report = backend_route_json(ibus);

        assert_eq!(
            report["trigger_precondition"],
            "surrounding_text_matches_trigger"
        );
        assert_eq!(report["output_transaction"], "separate_delete_and_commit");
        assert_eq!(report["application_result"], "unobservable");
        assert_eq!(report["atomic_replace"], false);
        assert!(report.get("replacement_assurance").is_none());
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/contracts/cli-json-shapes.json"
        ))
        .unwrap();
        let expected = contract["backend_list_route"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field.as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        let actual = report
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
    }
}
