//! Sanitized, stable-ID diagnostics shared by human and machine-readable CLI
//! troubleshooting. This module deliberately never reads or prints input text.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Component {
    Installation,
    Configuration,
    Service,
    Portal,
    Ibus,
    Input,
    Output,
    Version,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiagnosticFinding {
    pub(crate) id: &'static str,
    pub(crate) severity: Severity,
    pub(crate) component: Component,
    pub(crate) summary: String,
    pub(crate) evidence: Vec<String>,
    pub(crate) suggested_actions: Vec<String>,
}

fn finding(
    id: &'static str,
    severity: Severity,
    component: Component,
    summary: impl Into<String>,
    evidence: impl IntoIterator<Item = impl Into<String>>,
    actions: impl IntoIterator<Item = impl Into<String>>,
) -> DiagnosticFinding {
    DiagnosticFinding {
        id,
        severity,
        component,
        summary: summary.into(),
        evidence: evidence.into_iter().map(Into::into).collect(),
        suggested_actions: actions.into_iter().map(Into::into).collect(),
    }
}

/// Explain a terminal-only capture refusal without publishing application
/// identity. The focus CLI can provide more detail on request, but the normal
/// debug report must remain safe to attach to support tickets.
fn focused_scope_finding(daemon: &serde_json::Value) -> Option<DiagnosticFinding> {
    if daemon
        .get("terminal_apps_only")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return None;
    }

    let scope = daemon
        .get("capture_app_scope_state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if daemon
        .get("window_process_identity_state")
        .and_then(serde_json::Value::as_str)
        == Some("compositor-process-id")
    {
        return Some(finding(
            "WX-EVDEV-FOCUS-006",
            Severity::Error,
            Component::Input,
            "Terminal-only matching is suppressed because KWin reported a compositor surface",
            ["The reported process ID belongs to KWin, not to a focused application."],
            ["Focus the actual terminal or browser window and rerun `wayexpand debug --focused-app-id --json`; do not allowlist `kwin_wayland` or use a window title as a substitute for client identity."],
        ));
    }
    match scope {
        "allowed" | "unrestricted" => None,
        "outside_allowlist"
            if daemon
                .get("window_tracker_identity_source")
                .and_then(serde_json::Value::as_str)
                == Some("process-name") =>
        {
            Some(finding(
                "WX-EVDEV-FOCUS-004",
                Severity::Warning,
                Component::Input,
                "Terminal-only matching is suppressed; only a mutable process name is available",
                ["A process-name filter is lower assurance than an executable path and can be changed by the application."],
                ["Use `wayexpand debug --focused-app-id --json` to inspect the exact process name. If your policy permits this weaker identity, add an explicit `process_name_exact:<name>` filter and retest; otherwise use an application-ID-capable route or manual paste."],
            ))
        }
        "outside_allowlist" => Some(finding(
            "WX-EVDEV-FOCUS-002",
            Severity::Error,
            Component::Input,
            "Terminal-only matching is suppressed for the focused application",
            ["The focused application does not match the configured capture scope."],
            ["Run `wayexpand debug --focused-app-id --json` to inspect the scope decision; add only a verified exact application ID or executable path, then retest."],
        )),
        _ if daemon
            .get("window_tracker_identity_source")
            .and_then(serde_json::Value::as_str)
            == Some("process-executable-unavailable") =>
        {
            Some(finding(
                "WX-EVDEV-FOCUS-003",
                Severity::Error,
                Component::Input,
                "The focused window cannot be authorized for terminal-only matching",
                ["KWin provided a window process ID, but WayExpand could not verify its executable identity."],
                ["Run `wayexpand debug --focused-app-id --json` to confirm the host limitation. Do not weaken process permissions or replace exact scope with a title or wildcard; use an application-ID-capable route or manual paste, then retest."],
            ))
        }
        _ => Some(finding(
            "WX-EVDEV-FOCUS-001",
            Severity::Error,
            Component::Input,
            "Terminal-only matching is suppressed because focused application identity is unknown",
            ["The daemon cannot confirm that the focused window is in the configured application scope."],
            ["Focus the intended terminal and run `wayexpand debug --focused-app-id --json`. Keep terminal-only capture enabled; do not use a title or wildcard to bypass an unknown identity."],
        )),
    }
}

pub(crate) fn collect(trigger: Option<&str>) -> serde_json::Value {
    use crate::commands::install;
    let checks = install::checks();
    let config_path = wayexpand_core::default_config_path();
    let config_result = wayexpand_core::Config::load(&config_path);
    let (config_valid, expansion_count, enabled_count, config_reason) = match config_result {
        Ok(config) => {
            let count = config.expansion.len();
            let enabled = config
                .expansion
                .iter()
                .filter(|expansion| expansion.is_keyboard_matchable())
                .count();
            (true, count, enabled, None)
        }
        Err(error) => (false, 0, 0, Some(error.safe_summary().to_owned())),
    };
    let policy = crate::doctor::policy::load_policy().ok();
    let capabilities = wayexpand_backend_selection::probe_capabilities();
    let recommendation = policy.as_ref().map(|policy| {
        crate::setup::recommended_setup_backend_with_ibus(
            &capabilities,
            wayexpand_backend_ibus::engine_available()
                || wayexpand_backend_ibus::engine_installed(),
            policy,
        )
    });
    let ibus = wayexpand_backend_ibus::chain::diagnose();
    let ibus_readiness = ibus.readiness();
    let daemon = crate::doctor::status::read_daemon_status()
        .and_then(|response| crate::doctor::status::status_as_json(&response))
        .ok();
    let trigger_explanation = trigger.and_then(|typed| {
        if typed.is_empty() || typed.chars().any(char::is_control) {
            return None;
        }
        let config = wayexpand_core::Config::load(&config_path).ok()?;
        let policy = policy.as_ref()?;
        let mut engine = wayexpand_core::ExpansionEngine::new(config).ok()?;
        engine.apply_administrator_policy(policy).ok()?;
        let backend = daemon
            .as_ref()
            .and_then(|status| status.get("backend"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        serde_json::to_value(engine.explain_diagnostic(typed, backend)).ok()
    });
    let services = crate::services::ServiceInventory::probe();

    let mut findings = Vec::new();
    for check in &checks {
        if check.status == "fail" && check.component != "configuration" {
            let (id, component) = match check.component {
                "control_service" | "input_method_service" => {
                    ("WX-SERVICE-001", Component::Service)
                }
                "configuration" => ("WX-CONFIG-001", Component::Configuration),
                _ => ("WX-INSTALL-001", Component::Installation),
            };
            let action = if component == Component::Service {
                "wayexpand repair --check; reinstall if the service unit is absent"
            } else {
                "wayexpand install-check --json"
            };
            findings.push(finding(
                id,
                Severity::Error,
                component,
                format!(
                    "Required {} component is missing or not ready",
                    check.component
                ),
                [check.path.clone()],
                [action],
            ));
        }
    }
    if !config_valid {
        findings.push(finding(
            "WX-CONFIG-001",
            Severity::Error,
            Component::Configuration,
            "Configuration could not be loaded",
            [config_reason.unwrap_or_else(|| "configuration unavailable".into())],
            ["Run `wayexpand validate` and correct the reported configuration issue."],
        ));
    } else if enabled_count == 0 {
        findings.push(finding(
            "WX-CONFIG-002",
            Severity::Warning,
            Component::Configuration,
            "No enabled expansions are loaded",
            [format!(
                "{expansion_count} total expansion(s); zero enabled"
            )],
            ["Enable an expansion with `wayexpand set-enabled <trigger> on`."],
        ));
    }
    let plasma_environment = ibus
        .check("toolkit-environment")
        .filter(|check| check.state != wayexpand_backend_ibus::chain::IbusCheckState::Pass)
        .or_else(|| {
            ibus.check("desktop-shell-environment")
                .filter(|check| check.state != wayexpand_backend_ibus::chain::IbusCheckState::Pass)
        });
    let plasma_environment_blocked = plasma_environment.is_some();
    let ibus_input_path_blocked = !ibus_readiness.ibus_running
        || ibus.check("desktop").is_some_and(|check| {
            check.state == wayexpand_backend_ibus::chain::IbusCheckState::Fail
                && matches!(
                    check.problem_id,
                    Some("WX-KWIN-IBUS-001" | "WX-KWIN-IBUS-002")
                )
        });
    if let Some(check) = plasma_environment {
        findings.push(finding(
            "WX-IBUS-TOOLKIT-001",
            Severity::Warning,
            Component::Ibus,
            "Legacy GTK/Qt environment overrides conflict with Plasma's native IBus Wayland route",
            [check.detail.clone()],
            [check.fix.as_deref().unwrap_or(
                "Remove GTK_IM_MODULE and QT_IM_MODULE from Plasma login files, log out and back in, then rerun `wayexpand ibus status`.",
            )],
        ));
    }
    if let Some(check) = ibus.check("desktop") {
        if let Some(bridge_finding) = kwin_bridge_finding(check) {
            findings.push(bridge_finding);
        }
    }
    if let Some(check) = ibus
        .check("config-generation")
        .filter(|check| check.state == wayexpand_backend_ibus::chain::IbusCheckState::Fail)
    {
        findings.push(finding(
            "WX-IBUS-RELOAD-001",
            Severity::Error,
            Component::Ibus,
            "IBus contexts are not using the latest configuration generation",
            [check.detail.clone()],
            [check.fix.as_deref().unwrap_or(
                "Restart the WayExpand IBus engine from the desktop input-source controls, then rerun `wayexpand ibus status`.",
            )],
        ));
    }
    if !ibus_readiness.ibus_running {
        if let Some(check) = ibus.check("daemon") {
            if check.state == wayexpand_backend_ibus::chain::IbusCheckState::Fail {
                findings.push(finding(
                    "WX-IBUS-004",
                    Severity::Error,
                    Component::Ibus,
                    "The IBus daemon is not responding; browser and other IBus clients cannot reach WayExpand",
                    [check.detail.clone()],
                    [check.fix.as_deref().unwrap_or(
                        "Restore the desktop-managed IBus session, then run `wayexpand ibus status --json` and confirm the daemon check passes.",
                    )],
                ));
            }
        }
    }
    if !ibus_readiness.registered {
        findings.push(finding(
            "WX-IBUS-001",
            Severity::Warning,
            Component::Ibus,
            "IBus does not report the WayExpand engine as registered",
            [format!(
                "component installed: {}",
                ibus_readiness.component_installed
            )],
            ["Run `wayexpand setup --backend ibus`, then `wayexpand ibus status`."],
        ));
    } else if !ibus_readiness.selected {
        findings.push(finding(
            "WX-IBUS-002",
            Severity::Info,
            Component::Ibus,
            "The WayExpand IBus engine is registered but not selected",
            [format!("IBus running: {}", ibus_readiness.ibus_running)],
            ["Select WayExpand from the desktop input-method menu, then run `wayexpand ibus status`."],
        ));
    } else if !ibus_readiness.text_received
        && !plasma_environment_blocked
        && !ibus_input_path_blocked
    {
        findings.push(finding(
            "WX-IBUS-003",
            Severity::Warning,
            Component::Ibus,
            "The selected IBus engine has not reported text from a focused field",
            [format!("engine running: {}; text observed: {}", ibus_readiness.engine_running, ibus_readiness.text_received)],
            ["Focus a supported text editor and type harmless text, then run `wayexpand debug --live`."],
        ));
    } else if !ibus_readiness.replacement_context_received
        && !plasma_environment_blocked
        && !ibus_input_path_blocked
    {
        findings.push(finding(
            "WX-IBUS-008",
            Severity::Warning,
            Component::Ibus,
            "IBus received normal-field input but no usable surrounding text",
            ["The client cannot confirm which text would be replaced; automatic trigger replacement remains disabled for this field."],
            ["Use a text field that reports surrounding text, or open Quick Picker and copy a plain-text snippet for manual paste. Run `wayexpand ibus status --json` to retest."],
        ));
    }
    if let Some(daemon) = &daemon {
        if let Some(scope_finding) = focused_scope_finding(daemon) {
            findings.push(scope_finding);
        }
        let route_state = daemon.get("state").and_then(serde_json::Value::as_str);
        if matches!(route_state, Some("permission_required" | "portal_revoked")) {
            findings.push(finding(
                "WX-PORTAL-001",
                Severity::Warning,
                Component::Portal,
                if route_state == Some("portal_revoked") {
                    "Desktop input-emulation authorization was revoked"
                } else {
                    "Desktop input-emulation authorization is required"
                },
                [format!("route state: {}", route_state.unwrap_or("unknown"))],
                ["Review the expected desktop portal prompt; run `wayexpand backend test --injection` to request a harmless test session."],
            ));
        }
        if daemon
            .get("route_available")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
            || daemon
                .get("matching_available")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
        {
            findings.push(finding(
                "WX-INPUT-001",
                Severity::Error,
                Component::Input,
                "The active capture route cannot currently match text",
                [daemon.get("matching_unavailable_reason").and_then(serde_json::Value::as_str).unwrap_or("route reports matching unavailable")],
                ["Run `wayexpand explain-backend` and inspect the active route's layout and capture requirements."],
            ));
        }
        if !crate::doctor::status::daemon_build_matches(
            daemon
                .get("daemon_commit")
                .and_then(serde_json::Value::as_str),
            daemon
                .get("daemon_version")
                .and_then(serde_json::Value::as_str),
        ) {
            findings.push(finding(
                "WX-VERSION-001",
                Severity::Warning,
                Component::Version,
                "The running daemon and CLI build identities differ or cannot be verified",
                ["CLI and daemon build identities differ, or an older daemon cannot report an exact source-tree version"],
                ["Run `wayexpand upgrade --check`, then `wayexpand upgrade --apply`."],
            ));
        }
        if daemon
            .get("output_health")
            .and_then(serde_json::Value::as_str)
            == Some("stalled")
            || daemon
                .get("inject_insertion_mode")
                .and_then(serde_json::Value::as_str)
                == Some("unavailable")
        {
            findings.push(finding(
                "WX-OUTPUT-001",
                Severity::Error,
                Component::Output,
                "The output route is unavailable or stalled",
                [format!("output health: {}; insertion mode: {}", daemon.get("output_health").and_then(serde_json::Value::as_str).unwrap_or("unknown"), daemon.get("inject_insertion_mode").and_then(serde_json::Value::as_str).unwrap_or("unknown"))],
                ["Run `wayexpand doctor --json` and inspect the route-specific output recovery guidance."],
            ));
        }
        if daemon
            .get("output_uncertain_total")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            > 0
        {
            findings.push(finding(
                "WX-OUTPUT-002",
                Severity::Warning,
                Component::Output,
                "One or more output transactions have an uncertain or partial result",
                [format!("{} uncertain transaction(s)", daemon["output_uncertain_total"])],
                ["Inspect `wayexpand status` and the target application before retrying any insertion."],
            ));
        }
    } else {
        let installed = crate::services::UserService::ALL
            .into_iter()
            .any(|service| services.state(service).installed);
        findings.push(finding(
            if installed { "WX-SERVICE-002" } else { "WX-INPUT-001" },
            Severity::Warning,
            if installed { Component::Service } else { Component::Input },
            if installed { "No WayExpand daemon responded; an installed service may be stopped" } else { "No input daemon is responding" },
            ["The local control socket did not return a status snapshot"],
            [if installed { "Run `wayexpand status`, then `wayexpand setup --apply` or restart the selected user service." } else { "Run `wayexpand setup --check`, then configure a supported route." }],
        ));
    }
    if recommendation
        .as_ref()
        .is_some_and(|route| route.backend == "unavailable")
    {
        findings.push(finding(
            "WX-INPUT-002",
            Severity::Warning,
            Component::Input,
            "No safe automatic input route is available in this session",
            [recommendation.as_ref().map(|route| route.detail.clone()).unwrap_or_default()],
            ["Review `wayexpand explain-backend`; do not enable global evdev capture unless its privacy tradeoff is acceptable."],
        ));
    }
    if findings.is_empty() {
        findings.push(finding(
            "WX-DEBUG-000",
            Severity::Info,
            Component::Input,
            "No current configuration or runtime failures were detected",
            ["This report does not prove correct insertion in a target application."],
            ["Test a harmless expansion in the application you intend to use."],
        ));
    }

    let ready = findings
        .iter()
        .all(|item| !matches!(item.severity, Severity::Error));
    let recommendation = recommendation.map(|route| {
        serde_json::json!({
            "backend":route.backend,"label":route.label,"reason":route.detail
        })
    });
    let output = daemon.as_ref().map(|status| {
        serde_json::json!({
            "route_available":status.get("route_available"),
            "insertion_mode":status.get("inject_insertion_mode"),
            "latency_p95_us":status.get("injection_latency_p95_us"),
            "latency_p99_us":status.get("injection_latency_p99_us"),
            "uncertain_transactions":status.get("output_uncertain_total"),
            "partial_writes":status.get("partial_write_total"),
            "tested_in_target_application":false
        })
    });
    serde_json::json!({
        "schema":1,
        "version":{"version":crate::build_info::VERSION,"commit":crate::build_info::COMMIT},
        "desktop":{"session":crate::doctor::backends::session_description(),"recommendation":recommendation},
        "configuration":{"valid":config_valid,"expansions":expansion_count,"enabled_expansions":enabled_count},
        "installation":checks,
        "input":{"daemon":daemon,"ibus":ibus.readiness()},
        "output":output,
        "services":crate::services::UserService::ALL.into_iter().map(|service| {
            let state=services.state(service);
            serde_json::json!({"unit":service.unit(),"installed":state.installed,"enabled":state.enabled,"active":state.active,"failed":state.failed})
        }).collect::<Vec<_>>(),
        "output_tested":false,
        "trigger_explanation":trigger_explanation,
        "ready":ready,
        "findings":findings,
        "privacy":{"raw_input_logged":false,"captured_trigger_logged":false,"replacement_logged":false,"clipboard_logged":false,"explicit_trigger_argument_included":trigger.is_some()}
    })
}

fn kwin_bridge_finding(
    check: &wayexpand_backend_ibus::chain::IbusCheck,
) -> Option<DiagnosticFinding> {
    let id = check.problem_id?;
    let (summary, fallback) = match id {
        "WX-KWIN-IBUS-001" => (
            "KWin's IBus Wayland bridge is unavailable",
            "Select IBus Wayland in Plasma's Virtual Keyboard settings, log out and back in, then run `wayexpand ibus status --json`.",
        ),
        "WX-KWIN-IBUS-002" => (
            "KWin's IBus Wayland helper recently crashed",
            "Install the latest IBus package supported by your distribution, log out and back in, then rerun `wayexpand ibus status --json`. If the crash recurs, report its backtrace and exact package version to your distribution or IBus maintainers.",
        ),
        _ => return None,
    };
    Some(finding(
        id,
        Severity::Error,
        Component::Ibus,
        summary,
        [check.detail.clone()],
        [check.fix.as_deref().unwrap_or(fallback)],
    ))
}

pub(crate) fn print(report: &serde_json::Value, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    println!("WayExpand Debug Report\n───────────────────────────────────────────────");
    println!(
        "Version:       {} ({})",
        report["version"]["version"].as_str().unwrap_or("unknown"),
        report["version"]["commit"].as_str().unwrap_or("unknown")
    );
    println!(
        "Desktop:       {}",
        report["desktop"]["session"].as_str().unwrap_or("unknown")
    );
    println!(
        "Configuration: {} valid, {} enabled of {}",
        if report["configuration"]["valid"] == true {
            "valid"
        } else {
            "invalid"
        },
        report["configuration"]["enabled_expansions"]
            .as_u64()
            .unwrap_or(0),
        report["configuration"]["expansions"].as_u64().unwrap_or(0)
    );
    println!("\nINSTALLATION");
    for check in report["installation"].as_array().into_iter().flatten() {
        let marker = if check["status"] == "pass" {
            "✓"
        } else {
            "✗"
        };
        println!(
            "  {marker} {}",
            check["component"].as_str().unwrap_or("component")
        );
    }
    println!("\nINPUT");
    println!(
        "  Daemon: {}",
        report["input"]["daemon"]["state"]
            .as_str()
            .unwrap_or("not responding")
    );
    println!(
        "  IBus selected: {}",
        report["input"]["ibus"]["selected"]
            .as_bool()
            .unwrap_or(false)
    );
    println!("\nOUTPUT");
    println!(
        "  Insertion mode: {}",
        report["output"]["insertion_mode"]
            .as_str()
            .unwrap_or("not available")
    );
    println!("  Integration test: not performed; output acceptance in a target application is unverified\n");
    if let Some(explanation) = report.get("trigger_explanation").filter(|v| !v.is_null()) {
        println!("TRIGGER MATCH PREVIEW");
        println!(
            "  Would expand: {}",
            explanation["checks"]
                .as_array()
                .is_some_and(|checks| checks.iter().all(|check| check["status"] != "fail"))
        );
        for check in explanation["checks"].as_array().into_iter().flatten() {
            println!(
                "  {} {}: {}",
                if check["status"] == "pass" {
                    "✓"
                } else if check["status"] == "fail" {
                    "✗"
                } else {
                    "·"
                },
                check["name"].as_str().unwrap_or("check"),
                check["detail"].as_str().unwrap_or("")
            );
        }
    }
    for finding in report["findings"].as_array().into_iter().flatten() {
        let severity = finding["severity"]
            .as_str()
            .unwrap_or("info")
            .to_uppercase();
        println!(
            "[{severity}] {} — {}",
            finding["id"].as_str().unwrap_or("WX-UNKNOWN"),
            finding["summary"].as_str().unwrap_or("diagnostic finding")
        );
        for action in finding["suggested_actions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            println!("  Next: {}", action.as_str().unwrap_or(""));
        }
    }
    println!(
        "\nCaptured sensitive input, trigger text, replacements, and clipboard contents are never printed. An explicitly supplied --trigger is analyzed without rendering its replacement."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{focused_scope_finding, kwin_bridge_finding, Component, Severity};
    use wayexpand_backend_ibus::chain::{IbusCheck, IbusCheckState};

    fn check(problem_id: &'static str) -> IbusCheck {
        IbusCheck {
            id: "desktop",
            title: "KWin sends input to IBus",
            state: IbusCheckState::Fail,
            problem_id: Some(problem_id),
            detail: "sanitized bridge diagnostic".into(),
            fix: Some("specific repair and retest".into()),
        }
    }

    #[test]
    fn doctor_preserves_kwin_bridge_problem_ids_and_remediation() {
        for id in ["WX-KWIN-IBUS-001", "WX-KWIN-IBUS-002"] {
            let finding = kwin_bridge_finding(&check(id)).unwrap();
            assert_eq!(finding.id, id);
            assert_eq!(finding.severity, Severity::Error);
            assert_eq!(finding.component, Component::Ibus);
            assert_eq!(finding.evidence, ["sanitized bridge diagnostic"]);
            assert_eq!(finding.suggested_actions, ["specific repair and retest"]);
        }
    }

    #[test]
    fn doctor_ignores_unrelated_kwin_check_ids() {
        assert!(kwin_bridge_finding(&check("WX-IBUS-001")).is_none());
    }

    #[test]
    fn terminal_scope_diagnostics_distinguish_unknown_allowlist_and_host_denial() {
        let unknown = focused_scope_finding(&serde_json::json!({
            "terminal_apps_only": true,
            "capture_app_scope_state": "unknown"
        }))
        .unwrap();
        assert_eq!(unknown.id, "WX-EVDEV-FOCUS-001");
        assert_eq!(unknown.component, Component::Input);
        assert!(unknown
            .suggested_actions
            .iter()
            .any(|action| action.contains("debug --focused-app-id --json")));

        let denied = focused_scope_finding(&serde_json::json!({
            "terminal_apps_only": true,
            "capture_app_scope_state": "unknown",
            "window_tracker_identity_source": "process-executable-unavailable",
            "focused_app_id": "process-executable-exact:/home/user/private-terminal",
            "window_title": "secret production session"
        }))
        .unwrap();
        assert_eq!(denied.id, "WX-EVDEV-FOCUS-003");
        assert!(denied
            .suggested_actions
            .iter()
            .any(|action| action.contains("Do not weaken process permissions")));
        let serialized = serde_json::to_string(&denied).unwrap();
        assert!(!serialized.contains("private-terminal"));
        assert!(!serialized.contains("secret production session"));

        let outside = focused_scope_finding(&serde_json::json!({
            "terminal_apps_only": true,
            "capture_app_scope_state": "outside_allowlist"
        }))
        .unwrap();
        assert_eq!(outside.id, "WX-EVDEV-FOCUS-002");

        let process_name = focused_scope_finding(&serde_json::json!({
            "terminal_apps_only": true,
            "capture_app_scope_state": "outside_allowlist",
            "window_tracker_identity_source": "process-name"
        }))
        .unwrap();
        assert_eq!(process_name.id, "WX-EVDEV-FOCUS-004");
        let serialized = serde_json::to_string(&process_name).unwrap();
        assert!(serialized.contains("mutable process name"));
        assert!(serialized.contains("process_name_exact:<name>"));
        assert!(!serialized.contains("konsole"));

        let compositor = focused_scope_finding(&serde_json::json!({
            "terminal_apps_only": true,
            "capture_app_scope_state": "unknown",
            "window_process_identity_state": "compositor-process-id"
        }))
        .unwrap();
        assert_eq!(compositor.id, "WX-EVDEV-FOCUS-006");
        assert!(compositor
            .suggested_actions
            .iter()
            .any(|action| action.contains("do not allowlist `kwin_wayland`")));
        let serialized = serde_json::to_string(&compositor).unwrap();
        assert!(!serialized.contains("focused-window-pid"));

        assert!(focused_scope_finding(&serde_json::json!({
            "terminal_apps_only": false,
            "capture_app_scope_state": "unknown"
        }))
        .is_none());
        assert!(focused_scope_finding(&serde_json::json!({
            "terminal_apps_only": true,
            "capture_app_scope_state": "allowed"
        }))
        .is_none());
    }
}
