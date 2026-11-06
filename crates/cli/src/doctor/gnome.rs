//! Unified GNOME diagnostics for IBus input, snippet insertion, and
//! GNOME Shell focused-window tracking.

use anyhow::Result;
use serde_json::{json, Value};
use wayexpand_backend_ibus::chain::{self, IbusCheck, IbusCheckState, IbusDesktop};

const INPUT_CAPTURE_CHECKS: [&str; 11] = [
    "gnome-session",
    "tools",
    "daemon",
    "component",
    "manifest",
    "registered",
    "gnome-source-registered",
    "gnome-source-active",
    "selected",
    "engine",
    "field-input",
];

#[derive(Clone, Debug)]
struct DoctorCheck {
    id: &'static str,
    title: String,
    state: &'static str,
    detail: String,
    problem_id: Option<&'static str>,
    remedy: Option<String>,
}

impl DoctorCheck {
    fn from_ibus(check: &IbusCheck) -> Self {
        Self {
            id: check.id,
            title: check.title.to_owned(),
            state: check.state.label(),
            detail: check.detail.clone(),
            problem_id: problem_for(check),
            remedy: check.fix.clone(),
        }
    }

    fn new(
        id: &'static str,
        title: impl Into<String>,
        state: &'static str,
        detail: impl Into<String>,
        problem_id: Option<&'static str>,
        remedy: Option<String>,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            state,
            detail: detail.into(),
            problem_id,
            remedy,
        }
    }

    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "state": self.state,
            "detail": self.detail,
            "problem_id": self.problem_id,
            "remedy": self.remedy,
        })
    }
}

pub(crate) fn run(json_output: bool) -> Result<()> {
    let report = chain::diagnose();
    let input_capture = INPUT_CAPTURE_CHECKS
        .iter()
        .filter_map(|id| report.check(id))
        .map(DoctorCheck::from_ibus)
        .collect::<Vec<_>>();
    let window_status = crate::doctor::status::read_daemon_status().ok();
    let insertion = ["consent", "target-insertion"]
        .iter()
        .filter_map(|id| report.check(id))
        .map(|check| {
            let mut check = DoctorCheck::from_ibus(check);
            if check.id == "target-insertion" {
                check.title = "End-to-end expansion result".into();
            }
            check
        })
        .chain(std::iter::once(output_backend_check(
            window_status.as_deref(),
        )))
        .collect::<Vec<_>>();
    let tracking = window_tracking_checks(
        crate::gnome_extension::doctor_report(),
        window_status.as_deref(),
    );
    let (input_state, input_problem) = section_state(&input_capture);
    let (insertion_state, insertion_problem) = section_state(&insertion);
    let (tracking_state, tracking_problem) = section_state(&tracking);
    let is_gnome = report.desktop == IbusDesktop::Gnome;
    let conclusion = if !is_gnome {
        "not_gnome_or_unknown"
    } else if [input_state, insertion_state, tracking_state].contains(&"fail")
        || [input_state, insertion_state, tracking_state].contains(&"warn")
    {
        "incomplete"
    } else if [input_state, insertion_state, tracking_state].contains(&"unknown") {
        "unknown"
    } else {
        "operational_evidence_available"
    };
    let problem = if !is_gnome {
        Some("WX-GNOME-SESSION-001")
    } else {
        [input_problem, insertion_problem, tracking_problem]
            .into_iter()
            .flatten()
            .next()
    };
    let healthy = is_gnome && conclusion == "operational_evidence_available";

    if json_output {
        let all_checks = input_capture
            .iter()
            .chain(insertion.iter())
            .chain(tracking.iter())
            .map(DoctorCheck::json)
            .collect::<Vec<_>>();
        let value = json!({
            "schema": 2,
            "command": "doctor --gnome",
            "desktop": if is_gnome { "gnome" } else { "other_or_unknown" },
            "conclusion": conclusion,
            "healthy": healthy,
            "problem_id": problem,
            "sections": {
                "input_capture": { "state": input_state, "checks": input_capture.iter().map(DoctorCheck::json).collect::<Vec<_>>() },
                "snippet_insertion": { "state": insertion_state, "checks": insertion.iter().map(DoctorCheck::json).collect::<Vec<_>>() },
                "window_tracking": { "state": tracking_state, "checks": tracking.iter().map(DoctorCheck::json).collect::<Vec<_>>() },
            },
            // Keep the flat list for consumers of schema 1 while giving new
            // callers the sectioned, detailed schema above.
            "checks": all_checks,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        if !healthy {
            anyhow::bail!("GNOME integration readiness is {conclusion}; inspect the structured checks and problem_id");
        }
        return Ok(());
    }

    println!("WayExpand GNOME Health");
    println!("Conclusion: {}", conclusion.replace('_', " "));
    print_section("Input capture", &input_capture);
    print_section("Snippet insertion", &insertion);
    print_section("GNOME window tracking", &tracking);
    if let Some(problem) = problem {
        println!("\nProblem: {problem}");
    }
    println!("\nSuggested diagnostic commands (content-free):");
    println!("  gsettings get org.gnome.desktop.input-sources sources");
    println!("  ibus list-engine --name-only");
    println!("  ibus engine");
    println!("  wayexpand ibus status --json");
    println!("  wayexpand status");
    println!("\nA connected engine or window tracker does not certify insertion in every application. Use `wayexpand debug --gnome --interactive` for a user-verified test.");
    if !healthy {
        anyhow::bail!(
            "GNOME integration readiness is {conclusion}; follow the reported remedies and retest"
        );
    }
    Ok(())
}

fn print_section(title: &str, checks: &[DoctorCheck]) {
    println!("\n{title}:");
    for check in checks {
        println!(
            "  {} {} — {}",
            state_symbol(check.state),
            check.title,
            check.state
        );
        println!("      {}", check.detail);
        if let Some(remedy) = &check.remedy {
            println!("      Fix: {remedy}");
        }
    }
}

fn state_symbol(state: &str) -> &'static str {
    match state {
        "pass" => "✓",
        "warn" | "fail" => "!",
        "skip" => "·",
        _ => "?",
    }
}

fn output_backend_check(status: Option<&str>) -> DoctorCheck {
    let state = status.and_then(|status| status_field(status, "output_connection_state"));
    let mode = status.and_then(|status| status_field(status, "inject_insertion_mode"));
    match (state, mode) {
        (Some("connected"), Some(mode)) if !mode.is_empty() => DoctorCheck::new(
            "output-backend",
            "Output backend available",
            "pass",
            format!("the daemon reports a connected output route using {mode}; this is capability evidence, not a client insertion test"),
            None,
            None,
        ),
        (Some("disconnected" | "unavailable"), _) => DoctorCheck::new(
            "output-backend",
            "Output backend available",
            "fail",
            "the daemon reports that the output route is disconnected or unavailable",
            Some("WX-OUTPUT-001"),
            Some("run `wayexpand backend list` and restore the selected output route".into()),
        ),
        _ => DoctorCheck::new(
            "output-backend",
            "Output backend available",
            "unknown",
            "the daemon did not provide enough live status to verify an output route",
            Some("WX-OUTPUT-001"),
            Some("start the user service, then run `wayexpand status` and `wayexpand doctor --gnome` again".into()),
        ),
    }
}

fn status_field<'a>(status: &'a str, key: &str) -> Option<&'a str> {
    status.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name == key).then_some(value)
    })
}

fn window_tracking_checks(
    extension: Option<Value>,
    daemon_status: Option<&str>,
) -> Vec<DoctorCheck> {
    let report = extension.as_ref();
    let installed_path = report.and_then(|value| value.get("installed_path"));
    let installed_matches = report
        .and_then(|value| value.get("installed_matches_bundled"))
        .and_then(Value::as_bool);
    let installed = match (installed_path, installed_matches) {
        (Some(path), Some(true)) if !path.is_null() => DoctorCheck::new(
            "window-extension-installed",
            "Extension installed",
            "pass",
            "the installed extension files match this WayExpand build",
            None,
            None,
        ),
        (Some(path), _) if !path.is_null() => DoctorCheck::new(
            "window-extension-installed",
            "Extension installed",
            "warn",
            "an extension copy exists, but it does not match the bundled files",
            Some("WX-GNOME-WINDOW-001"),
            Some("run `wayexpand setup` to update the GNOME Shell extension".into()),
        ),
        _ => DoctorCheck::new(
            "window-extension-installed",
            "Extension installed",
            "fail",
            "the WayExpand GNOME Shell extension is not installed",
            Some("WX-GNOME-WINDOW-001"),
            Some("run `wayexpand setup` to install the GNOME Shell window tracker".into()),
        ),
    };

    let extension_state = report
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let loaded = match extension_state {
        "active" => DoctorCheck::new(
            "window-extension-loaded",
            "Extension loaded in GNOME Shell",
            "pass",
            "GNOME Shell reports the WayExpand extension active",
            None,
            None,
        ),
        "inactive" | "error" | "out-of-date" => DoctorCheck::new(
            "window-extension-loaded",
            "Extension loaded in GNOME Shell",
            "fail",
            report.and_then(|value| value.get("detail")).and_then(Value::as_str).unwrap_or("GNOME Shell did not load the WayExpand extension"),
            Some("WX-GNOME-WINDOW-002"),
            report.and_then(|value| value.get("fix")).and_then(Value::as_str).map(str::to_owned).or_else(|| Some("run `wayexpand setup` and log out and back in if GNOME requires a session reload".into())),
        ),
        _ => DoctorCheck::new(
            "window-extension-loaded",
            "Extension loaded in GNOME Shell",
            "unknown",
            report.and_then(|value| value.get("detail")).and_then(Value::as_str).unwrap_or("the GNOME Shell extension state could not be determined"),
            Some("WX-GNOME-WINDOW-002"),
            report.and_then(|value| value.get("fix")).and_then(Value::as_str).map(str::to_owned),
        ),
    };

    let build = report.and_then(|value| value.get("build_identity"));
    let build_state = build
        .and_then(|value| value.get("runtime_state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let build_check = match build_state {
        "current" => DoctorCheck::new(
            "window-extension-build",
            "Running extension build",
            "pass",
            format!(
                "GNOME Shell is running the installed build {}",
                build
                    .and_then(|value| value.get("live"))
                    .and_then(Value::as_str)
                    .unwrap_or("(identity unavailable)")
            ),
            None,
            None,
        ),
        "logout-required" | "update-required" | "invalid-on-disk" => DoctorCheck::new(
            "window-extension-build",
            "Running extension build",
            "warn",
            build
                .and_then(|value| value.get("detail"))
                .and_then(Value::as_str)
                .unwrap_or("the live extension build does not match the installed build"),
            Some("WX-GNOME-WINDOW-003"),
            build
                .and_then(|value| value.get("fix"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
        _ => DoctorCheck::new(
            "window-extension-build",
            "Running extension build",
            "unknown",
            build
                .and_then(|value| value.get("detail"))
                .and_then(Value::as_str)
                .unwrap_or("the running extension build could not be verified"),
            Some("WX-GNOME-WINDOW-003"),
            build
                .and_then(|value| value.get("fix"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
    };

    let protocol = report.and_then(|value| value.get("protocol_handshake"));
    let protocol_state = match protocol
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
    {
        "pass" => "pass",
        "fail" => "fail",
        _ => "unknown",
    };
    let handshake = DoctorCheck::new(
        "window-extension-handshake",
        "Tracker D-Bus handshake",
        protocol_state,
        protocol
            .and_then(|value| value.get("detail"))
            .and_then(Value::as_str)
            .unwrap_or("the tracker protocol handshake was not observed"),
        (protocol_state != "pass").then_some("WX-GNOME-WINDOW-004"),
        (protocol_state != "pass").then(|| {
            "run `wayexpand setup` and verify that the extension is active in GNOME Shell".into()
        }),
    );

    let handshake_verified = extension
        .as_ref()
        .and_then(|value| value.get("protocol_handshake"))
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
        == Some("pass");
    let tracker_connected = handshake_verified
        .then_some(daemon_status)
        .flatten()
        .and_then(|status| status_field(status, "window_tracker_connected"));
    let exact_identity = handshake_verified
        .then_some(daemon_status)
        .flatten()
        .and_then(|status| status_field(status, "window_identity_exact"));
    let focus = if !handshake_verified {
        DoctorCheck::new(
            "window-focus-events",
            "Live focus events",
            "unknown",
            "the GNOME tracker handshake is not verified, so focus status cannot be attributed to this extension",
            Some("WX-GNOME-WINDOW-005"),
            Some("resolve the tracker D-Bus handshake, then rerun `wayexpand doctor --gnome`".into()),
        )
    } else {
        match (tracker_connected, exact_identity) {
        (Some("true"), Some("true")) => DoctorCheck::new(
            "window-focus-events",
            "Live focus events",
            "pass",
            "the daemon has a current exact focus identity from the tracker; identity and title are omitted",
            None,
            None,
        ),
        (Some("true"), _) => DoctorCheck::new(
            "window-focus-events",
            "Live focus events",
            "warn",
            "the daemon is connected to a tracker, but no exact focused-window identity is currently available",
            Some("WX-GNOME-WINDOW-005"),
            Some("focus a normal application window and rerun `wayexpand doctor --gnome`; avoid shell overview and transient menus".into()),
        ),
        (Some("false"), _) => DoctorCheck::new(
            "window-focus-events",
            "Live focus events",
            "fail",
            "the daemon reports that its window tracker is disconnected",
            Some("WX-GNOME-WINDOW-005"),
            Some("run `wayexpand setup` and inspect the extension and D-Bus checks above".into()),
        ),
        _ => DoctorCheck::new(
            "window-focus-events",
            "Live focus events",
            "unknown",
            "daemon status is unavailable, so receipt of live focus events cannot be verified",
            Some("WX-GNOME-WINDOW-005"),
            Some("start the user service and rerun `wayexpand doctor --gnome`".into()),
        ),
        }
    };

    vec![installed, loaded, build_check, handshake, focus]
}

fn problem_for(check: &IbusCheck) -> Option<&'static str> {
    if matches!(check.state, IbusCheckState::Pass | IbusCheckState::Skip) {
        return None;
    }
    Some(match check.id {
        "gnome-session" => "WX-GNOME-SESSION-001",
        "daemon" => "WX-GNOME-IBUS-001",
        "component" | "manifest" | "registered" | "tools" => "WX-IBUS-001",
        "gnome-source-registered" => "WX-GNOME-IM-001",
        "gnome-source-active" | "selected" => "WX-GNOME-IM-002",
        "consent" => "WX-POLICY-001",
        "field-input" | "engine" => "WX-GNOME-IM-003",
        "target-insertion" => "WX-GNOME-IM-004",
        _ => check.problem_id.unwrap_or("WX-GNOME-IM-001"),
    })
}

fn section_state(checks: &[DoctorCheck]) -> (&'static str, Option<&'static str>) {
    let mut state = "pass";
    let mut problem = None;
    for check in checks {
        match check.state {
            "fail" => {
                state = "fail";
                if problem.is_none() {
                    problem = check.problem_id;
                }
            }
            "warn" if state != "fail" => {
                state = "warn";
                if problem.is_none() {
                    problem = check.problem_id;
                }
            }
            "unknown" | "skip" => {
                if state == "pass" {
                    state = "unknown";
                }
                if problem.is_none() {
                    problem = check.problem_id;
                }
            }
            _ => {}
        }
    }
    (state, problem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension_report(state: &str, build_state: &str, handshake: &str) -> Value {
        json!({
            "installed_path": "/redacted/extension",
            "installed_matches_bundled": true,
            "state": state,
            "detail": "extension state detail",
            "fix": "run setup",
            "build_identity": {"runtime_state": build_state, "live": "sha256:test", "detail": "build detail", "fix": "log out and in"},
            "protocol_handshake": {"state": handshake, "detail": "handshake detail"},
        })
    }

    #[test]
    fn window_tracking_report_separates_installed_loaded_build_handshake_and_focus() {
        let checks = window_tracking_checks(
            Some(extension_report("active", "current", "pass")),
            Some("window_tracker_connected=true\nwindow_identity_exact=true\n"),
        );
        assert_eq!(checks.len(), 5);
        assert!(checks.iter().all(|check| check.state == "pass"));
    }

    #[test]
    fn stale_extension_build_and_missing_focus_identity_are_not_reported_as_healthy() {
        let checks = window_tracking_checks(
            Some(extension_report("active", "logout-required", "pass")),
            Some("window_tracker_connected=true\nwindow_identity_exact=false\n"),
        );
        assert_eq!(checks[2].state, "warn");
        assert_eq!(checks[4].state, "warn");
        assert_eq!(section_state(&checks).0, "warn");
    }

    #[test]
    fn missing_extension_and_unavailable_runtime_are_reported_independently() {
        let checks = window_tracking_checks(
            None,
            Some("window_tracker_connected=true\nwindow_identity_exact=true\n"),
        );
        assert_eq!(checks[0].id, "window-extension-installed");
        assert_eq!(checks[0].state, "fail");
        assert_eq!(checks[1].state, "unknown");
        assert_eq!(checks[3].state, "unknown");
        assert_eq!(checks[4].state, "unknown");
        assert!(!checks
            .iter()
            .any(|check| check.detail.contains("/redacted/extension")));
    }

    #[test]
    fn output_route_capability_does_not_claim_end_to_end_insertion() {
        let check = output_backend_check(Some(
            "output_connection_state=connected\ninject_insertion_mode=ibus commit\n",
        ));
        assert_eq!(check.state, "pass");
        assert!(check.detail.contains("not a client insertion test"));
        let disconnected = output_backend_check(Some(
            "output_connection_state=disconnected\ninject_insertion_mode=ibus commit\n",
        ));
        assert_eq!(disconnected.state, "fail");
    }

    #[test]
    fn unknown_gnome_checks_do_not_become_success() {
        let checks = [
            ("gnome-session", IbusCheckState::Pass),
            ("daemon", IbusCheckState::Pass),
            ("component", IbusCheckState::Pass),
            ("registered", IbusCheckState::Pass),
            ("gnome-source-registered", IbusCheckState::Unknown),
        ]
        .map(|(id, state)| IbusCheck {
            id,
            title: "test",
            state,
            problem_id: None,
            detail: String::new(),
            fix: None,
        });
        let checks = checks
            .iter()
            .map(DoctorCheck::from_ibus)
            .collect::<Vec<_>>();
        assert_eq!(section_state(&checks), ("unknown", Some("WX-GNOME-IM-001")));
    }
}
