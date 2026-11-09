//! Unified GNOME diagnostics for IBus input, snippet insertion, and
//! GNOME Shell focused-window tracking.

use anyhow::Result;
use serde_json::{json, Value};
use wayexpand_backend_ibus::chain::{self, IbusCheck, IbusCheckState, IbusDesktop};

const INPUT_CAPTURE_CHECKS: [&str; 13] = [
    "gnome-session",
    "tools",
    "daemon",
    "component",
    "manifest",
    "manifest-scope",
    "registered",
    "gnome-source-registered",
    "gnome-source-active",
    "selected",
    "engine",
    "config-generation",
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
    remediation_command: Option<String>,
    retest_instructions: Option<String>,
    evidence: Option<Value>,
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
            remediation_command: check
                .fix
                .as_deref()
                .and_then(first_command)
                .filter(|command| is_action_command(command))
                .or_else(|| default_remediation_command(check.id)),
            retest_instructions: (check.state != IbusCheckState::Pass)
                .then(|| retest_instructions(check.id).to_owned()),
            evidence: Some(json!({"observation": check.detail})),
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
            remediation_command: remedy
                .as_deref()
                .and_then(first_command)
                .filter(|command| is_action_command(command))
                .or_else(|| default_remediation_command(id)),
            retest_instructions: (state != "pass").then(|| retest_instructions(id).to_owned()),
            remedy,
            evidence: None,
        }
    }

    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "state": self.state,
            "detail": self.detail,
            "evidence": self.evidence.clone().unwrap_or_else(|| json!({"observation": self.detail})),
            "error_id": self.problem_id,
            "problem_id": self.problem_id,
            "remedy": self.remedy,
            "remediation_command": self.remediation_command,
            "retest_instructions": self.retest_instructions,
        })
    }
}

pub(crate) fn run(json_output: bool) -> Result<()> {
    let report = chain::diagnose();
    let input_capture = INPUT_CAPTURE_CHECKS
        .iter()
        .filter_map(|id| report.check(id))
        .map(DoctorCheck::from_ibus)
        .chain(std::iter::once(layout_provider_check(
            crate::doctor::status::read_daemon_status().ok().as_deref(),
        )))
        .collect::<Vec<_>>();
    let window_status = crate::doctor::status::read_daemon_status().ok();
    let insertion = ["consent", "surrounding-text", "target-insertion"]
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
        .chain(std::iter::once(daemon_version_check(
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
        "schema": 3,
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
        if check.state != "pass" {
            if let Some(problem_id) = check.problem_id {
                println!("      Error ID: {problem_id}");
            }
            println!("      Evidence: {}", check.detail);
            if let Some(command) = &check.remediation_command {
                println!("      Remediation command: {command}");
            }
            if let Some(remedy) = &check.remedy {
                println!("      Remediation: {remedy}");
            }
            if let Some(retest) = &check.retest_instructions {
                println!("      Retest: {retest}");
            }
        }
    }
}

fn first_command(text: &str) -> Option<String> {
    let (_, remaining) = text.split_once('`')?;
    let (command, _) = remaining.split_once('`')?;
    let command = command.trim();
    (!command.is_empty()).then(|| command.to_owned())
}

fn is_action_command(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let Some(program) = words.next() else {
        return false;
    };
    let has_action = words.next().is_some();
    let diagnostic_only = [
        "wayexpand doctor",
        "wayexpand status",
        "wayexpand health",
        "wayexpand support",
        "wayexpand ibus status",
        "wayexpand backend list",
        "ibus list-engine",
        "gnome-extensions info",
        "loginctl session-status",
    ]
    .iter()
    .any(|prefix| command == *prefix || command.starts_with(&format!("{prefix} ")))
        || command == "ibus engine";
    has_action
        && !diagnostic_only
        && matches!(
            program,
            "wayexpand"
                | "ibus"
                | "gsettings"
                | "gnome-extensions"
                | "systemctl"
                | "loginctl"
                | "apt"
                | "apt-get"
                | "dnf"
                | "pacman"
                | "pkexec"
                | "sudo"
        )
}

fn default_remediation_command(id: &str) -> Option<String> {
    Some(
        match id {
            "gnome-session" => "wayexpand setup --check",
            "tools" | "component" | "manifest" | "registered" | "engine" | "selected"
            | "consent" => "wayexpand setup --backend ibus",
            "daemon" => "ibus start",
            "manifest-scope" => "wayexpand setup --backend ibus --install-system-integration",
            "gnome-source-registered" | "gnome-source-active" => {
                "wayexpand repair --gnome-input-source"
            }
            "config-generation" => "wayexpand ibus status --json",
            "field-input" | "target-insertion" => "wayexpand debug --gnome --interactive",
            "surrounding-text" => "wayexpand ibus status --json",
            "output-backend" => "wayexpand backend list",
            "daemon-version" => "wayexpand upgrade --apply",
            "window-extension-installed" => "wayexpand setup",
            "window-extension-loaded" | "window-extension-build" => {
                "wayexpand doctor --gnome --json"
            }
            "window-extension-handshake" | "window-focus-events" => "wayexpand setup",
            "window-extension-client-auth" => "wayexpand setup",
            "evdev-layout-provider" => "wayexpand setup",
            _ => return None,
        }
        .to_owned(),
    )
}

fn layout_provider_check(status: Option<&str>) -> DoctorCheck {
    let Some(status) = status else {
        return DoctorCheck::new(
            "evdev-layout-provider",
            "GNOME keyboard-layout tracking",
            "unknown",
            "daemon status is unavailable, so the active layout provider cannot be checked",
            Some("WX-EVDEV-LAYOUT-001"),
            Some("start the WayExpand service, then run `wayexpand status` and `wayexpand doctor --gnome`".into()),
        );
    };
    let source = status_field(status, "source").unwrap_or("unknown");
    if source != "evdev" {
        return DoctorCheck::new(
            "evdev-layout-provider",
            "GNOME keyboard-layout tracking",
            "skip",
            format!("the active capture source is {source}; raw evdev layout tracking is not used"),
            None,
            None,
        );
    }
    match status_field(status, "capture_layout_state") {
        Some("tracked") => DoctorCheck::new(
            "evdev-layout-provider",
            "GNOME keyboard-layout tracking",
            "pass",
            "the daemon has a GNOME Shell-reported XKB map for its active input source",
            None,
            None,
        ),
        Some(state) => DoctorCheck::new(
            "evdev-layout-provider",
            "GNOME keyboard-layout tracking",
            "fail",
            format!("the daemon's GNOME layout provider reports {state}; evdev matching is disabled until GNOME's active XKB source can be read"),
            Some("WX-EVDEV-LAYOUT-001"),
            Some("run `wayexpand setup` to update the GNOME Shell extension; if it reports that logout is required, log out and back in, select an XKB source in Settings → Keyboard → Input Sources, then run `wayexpand upgrade --apply` and confirm `capture_layout_state=tracked` in `wayexpand status`".into()),
        ),
        None => DoctorCheck::new(
            "evdev-layout-provider",
            "GNOME keyboard-layout tracking",
            "unknown",
            "the daemon status schema does not expose the active layout state",
            Some("WX-EVDEV-LAYOUT-001"),
            Some("upgrade the daemon and run `wayexpand status` again".into()),
        ),
    }
}

fn retest_instructions(id: &str) -> &'static str {
    match id {
        "field-input" | "surrounding-text" | "engine" | "target-insertion" => {
            "Run `wayexpand debug --gnome --interactive` in a normal, disposable text field and confirm the exact result."
        }
        "config-generation" => {
            "Run `wayexpand ibus status --json`, restart the WayExpand IBus engine from the desktop input-source controls, then rerun status and confirm requested and published generations match with zero stale contexts."
        }
        "window-focus-events"
        | "window-extension-handshake"
        | "window-extension-build"
        | "window-extension-client-auth"
        | "evdev-layout-provider" => {
            "Focus a normal application window, then run `wayexpand doctor --gnome`; confirm the live tracker identity and build."
        }
        _ => "Run `wayexpand doctor --gnome` again and confirm this check reports pass.",
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

fn daemon_version_check(status: Option<&str>) -> DoctorCheck {
    let daemon_commit = status.and_then(|status| status_field(status, "daemon_commit"));
    let daemon_version = status.and_then(|status| status_field(status, "daemon_version"));
    let schema = status.and_then(|status| status_field(status, "status_schema"));
    let schema_compatible = schema
        .and_then(|schema| schema.parse::<u64>().ok())
        .is_some_and(|schema| schema == u64::from(wayexpand_core::CONTROL_STATUS_SCHEMA));
    if schema_compatible
        && crate::doctor::status::daemon_build_matches(daemon_commit, daemon_version)
    {
        DoctorCheck::new(
            "daemon-version",
            "Running daemon build matches this CLI",
            "pass",
            format!(
                "daemon version {} (commit {}) matches CLI version {} (commit {})",
                daemon_version.unwrap_or("unknown"),
                daemon_commit.unwrap_or("unknown"),
                crate::build_info::VERSION,
                crate::build_info::COMMIT
            ),
            None,
            None,
        )
    } else {
        let unknown = crate::doctor::status::daemon_build_unknown(daemon_commit, daemon_version)
            || schema.is_none();
        let state = if unknown { "unknown" } else { "fail" };
        let detail = if status.is_none() {
            "live daemon status is unavailable; the running build cannot be compared".to_owned()
        } else if !schema_compatible {
            format!("daemon status schema {} is not compatible with required schema {}; running commit {}", schema.unwrap_or("unknown"), wayexpand_core::CONTROL_STATUS_SCHEMA, daemon_commit.unwrap_or("unknown"))
        } else if unknown {
            format!(
                "daemon build identity is unavailable (status schema {}, CLI build {})",
                schema.unwrap_or("unknown"),
                crate::build_info::VERSION
            )
        } else {
            format!(
                "running daemon version {} (commit {}) differs from CLI version {} (commit {})",
                daemon_version.unwrap_or("unknown"),
                daemon_commit.unwrap_or("unknown"),
                crate::build_info::VERSION,
                crate::build_info::COMMIT
            )
        };
        DoctorCheck::new(
            "daemon-version",
            "Running daemon build matches this CLI",
            state,
            detail,
            Some("WX-VERSION-001"),
            Some("run `wayexpand upgrade --apply`, then rerun `wayexpand doctor --gnome`".into()),
        )
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
    let readiness = report
        .and_then(|value| value.get("readiness"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let loaded = match readiness {
        "not-loaded-this-session" => DoctorCheck::new(
            "window-extension-loaded",
            "Extension loaded in GNOME Shell",
            "warn",
            "GNOME Shell answered the status query and the complete current extension is installed and enabled, but this Shell session has not loaded it",
            Some("WX-GNOME-WINDOW-003"),
            Some("log out of GNOME and back in once, then rerun `wayexpand doctor --gnome --json`".into()),
        ),
        "session-bus-unavailable" => DoctorCheck::new(
            "window-extension-loaded",
            "Extension loaded in GNOME Shell",
            "unknown",
            report.and_then(|value| value.get("detail")).and_then(Value::as_str).unwrap_or("the GNOME Shell session bus could not be queried"),
            Some("WX-GNOME-WINDOW-007"),
            Some("run `wayexpand doctor --gnome --json` from the active GNOME desktop session; do not log out solely because this query failed".into()),
        ),
        "extension-failed" => DoctorCheck::new(
            "window-extension-loaded",
            "Extension loaded in GNOME Shell",
            "fail",
            report.and_then(|value| value.get("detail")).and_then(Value::as_str).unwrap_or("GNOME reported an extension or tracker-protocol failure"),
            Some("WX-GNOME-WINDOW-008"),
            report.and_then(|value| value.get("fix")).and_then(Value::as_str).map(str::to_owned).or_else(|| Some("run `wayexpand doctor --gnome --json` and inspect the extension error and tracker handshake".into())),
        ),
        "inactive" => DoctorCheck::new(
            "window-extension-loaded",
            "Extension loaded in GNOME Shell",
            "fail",
            "GNOME user extensions are disabled or the WayExpand extension is inactive",
            Some("WX-GNOME-WINDOW-002"),
            Some("enable user extensions in GNOME Settings, then run `wayexpand doctor --gnome`".into()),
        ),
        "unknown" => DoctorCheck::new(
            "window-extension-loaded",
            "Extension loaded in GNOME Shell",
            "unknown",
            report.and_then(|value| value.get("detail")).and_then(Value::as_str).unwrap_or("the GNOME Shell extension state is unknown; no session-restart diagnosis is established"),
            Some("WX-GNOME-WINDOW-009"),
            Some("run `wayexpand doctor --gnome --json` to inspect GNOME Shell availability and the extension state".into()),
        ),
        _ => match extension_state {
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
        },
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

    let client_auth = match (
        handshake_verified,
        status_field(daemon_status.unwrap_or_default(), "window_tracker_connected"),
    ) {
        (true, Some("true")) => DoctorCheck::new(
            "window-extension-client-auth",
            "Daemon authorized by GNOME tracker",
            "pass",
            "the daemon has an active tracker subscription; the extension accepted its same numeric UID and executable-identity checks",
            None,
            None,
        ),
        (true, Some("false")) => {
            DoctorCheck::new(
                "window-extension-client-auth",
                "Daemon authorized by GNOME tracker",
                "fail",
                "the extension D-Bus interface responds, but the daemon has no accepted subscription; the caller must have the current user's numeric UID and an actual executable basename of `wayexpand-daemon` or `wayexpand` (process arguments are not used for authorization)",
                Some("WX-GNOME-WINDOW-006"),
                Some("run `wayexpand setup` to install the matching extension, then `wayexpand upgrade --apply` to restart the daemon; rerun `wayexpand doctor --gnome`".into()),
            )
        }
        _ => DoctorCheck::new(
            "window-extension-client-auth",
            "Daemon authorized by GNOME tracker",
            "unknown",
            "no live daemon status is available to verify the authenticated tracker subscription",
            Some("WX-GNOME-WINDOW-006"),
            Some("start the daemon and rerun `wayexpand doctor --gnome`".into()),
        ),
    };

    vec![
        installed,
        loaded,
        build_check,
        handshake,
        client_auth,
        focus,
    ]
}

fn problem_for(check: &IbusCheck) -> Option<&'static str> {
    if matches!(check.state, IbusCheckState::Pass | IbusCheckState::Skip) {
        return None;
    }
    if check.id == "surrounding-text" && check.state != IbusCheckState::Fail {
        return check.problem_id;
    }
    Some(match check.id {
        "gnome-session" => "WX-GNOME-SESSION-001",
        "daemon" => "WX-GNOME-IBUS-001",
        "component" | "manifest" | "registered" | "tools" => "WX-IBUS-001",
        "manifest-scope" => "WX-IBUS-MULTIUSER-001",
        "gnome-source-registered" => "WX-GNOME-IM-001",
        "gnome-source-active" | "selected" => "WX-GNOME-IM-002",
        "config-generation" => "WX-IBUS-RELOAD-001",
        "consent" => "WX-POLICY-001",
        "field-input" | "engine" => "WX-GNOME-IM-003",
        "surrounding-text" => "WX-IBUS-008",
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
            "readiness": if state == "active" && build_state == "current" && handshake == "pass" { "ready" } else { "unknown" },
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
        assert_eq!(checks.len(), 6);
        assert!(checks.iter().all(|check| check.state == "pass"));
    }

    #[test]
    fn stale_extension_build_and_missing_focus_identity_are_not_reported_as_healthy() {
        let checks = window_tracking_checks(
            Some(extension_report("active", "logout-required", "pass")),
            Some("window_tracker_connected=true\nwindow_identity_exact=false\n"),
        );
        assert_eq!(checks[2].state, "warn");
        assert_eq!(checks[5].state, "warn");
        assert_eq!(section_state(&checks).0, "warn");
    }

    #[test]
    fn extension_readiness_does_not_turn_bus_or_unknown_errors_into_restart_advice() {
        for (readiness, expected_state, expected_problem, restart_advice) in [
            (
                "not-loaded-this-session",
                "warn",
                Some("WX-GNOME-WINDOW-003"),
                true,
            ),
            (
                "session-bus-unavailable",
                "unknown",
                Some("WX-GNOME-WINDOW-007"),
                false,
            ),
            (
                "extension-failed",
                "fail",
                Some("WX-GNOME-WINDOW-008"),
                false,
            ),
            ("unknown", "unknown", Some("WX-GNOME-WINDOW-009"), false),
        ] {
            let mut report = extension_report("shell-unavailable", "unknown", "unknown");
            report["readiness"] = json!(readiness);
            report["detail"] = json!(format!("observed {readiness}"));
            let loaded = window_tracking_checks(Some(report), None)
                .into_iter()
                .find(|check| check.id == "window-extension-loaded")
                .unwrap();
            assert_eq!(loaded.state, expected_state, "{readiness}");
            assert_eq!(loaded.problem_id, expected_problem, "{readiness}");
            let remedy = loaded.remedy.as_deref().unwrap_or_default();
            let recommends_restart =
                remedy.contains("log out") && !remedy.contains("do not log out");
            assert_eq!(recommends_restart, restart_advice, "{readiness}");
        }
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
        assert_eq!(checks[5].state, "unknown");
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
    fn doctor_reports_gnome_layout_failure_only_when_evdev_uses_it() {
        let failed =
            layout_provider_check(Some("source=evdev\ncapture_layout_state=unavailable\n"));
        assert_eq!(failed.state, "fail");
        assert_eq!(failed.problem_id, Some("WX-EVDEV-LAYOUT-001"));
        assert!(failed.detail.contains("active XKB source"));
        assert!(failed
            .remediation_command
            .unwrap()
            .contains("wayexpand setup"));

        let not_applicable =
            layout_provider_check(Some("source=ibus\ncapture_layout_state=not-applicable\n"));
        assert_eq!(not_applicable.state, "skip");

        let tracked = layout_provider_check(Some("source=evdev\ncapture_layout_state=tracked\n"));
        assert_eq!(tracked.state, "pass");
    }

    #[test]
    fn doctor_explains_tracker_client_authorization_failure() {
        let checks = window_tracking_checks(
            Some(extension_report("active", "current", "pass")),
            Some("window_tracker_connected=false\ndaemon_executable=/usr/bin/wayexpand-daemon\n"),
        );
        let auth = checks
            .iter()
            .find(|check| check.id == "window-extension-client-auth")
            .unwrap();
        assert_eq!(auth.problem_id, Some("WX-GNOME-WINDOW-006"));
        assert_eq!(auth.state, "fail");
        assert!(auth.detail.contains("current user's numeric UID"));
        assert!(auth
            .remedy
            .as_deref()
            .unwrap()
            .contains("wayexpand upgrade --apply"));
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

    #[test]
    fn actionable_nonpassing_stages_have_stable_id_evidence_command_and_retest() {
        for id in INPUT_CAPTURE_CHECKS {
            let check = DoctorCheck::from_ibus(&IbusCheck {
                id,
                title: "GNOME integration stage",
                state: IbusCheckState::Unknown,
                problem_id: None,
                detail: format!("evidence for {id}"),
                fix: None,
            });
            let json = check.json();
            assert!(json["error_id"].as_str().is_some(), "{id}");
            assert_eq!(
                json["evidence"]["observation"],
                format!("evidence for {id}")
            );
            assert!(json["remediation_command"].as_str().is_some(), "{id}");
            assert!(json["retest_instructions"].as_str().is_some(), "{id}");
        }

        let check = DoctorCheck::from_ibus(&IbusCheck {
            id: "field-input",
            title: "Normal-field input observed",
            state: IbusCheckState::Unknown,
            problem_id: None,
            detail: "no safe-field input telemetry was observed".into(),
            fix: Some("focus a normal text field and type harmless text".into()),
        });
        let json = check.json();
        assert_eq!(json["error_id"], "WX-GNOME-IM-003");
        assert_eq!(
            json["remediation_command"],
            "wayexpand debug --gnome --interactive"
        );
        assert!(json["retest_instructions"]
            .as_str()
            .unwrap()
            .contains("disposable text field"));

        let extension = window_tracking_checks(None, None);
        for check in extension.iter().filter(|check| check.state != "pass") {
            let json = check.json();
            assert!(json["error_id"].as_str().is_some(), "{}", check.id);
            assert!(json["evidence"].is_object(), "{}", check.id);
            assert!(
                json["remediation_command"].as_str().is_some(),
                "{}",
                check.id
            );
            assert!(
                json["retest_instructions"].as_str().is_some(),
                "{}",
                check.id
            );
        }
    }

    #[test]
    fn running_daemon_build_is_reported_separately_from_route_health() {
        let status = format!(
            "daemon_commit=old-build\ndaemon_version=old-version\nstatus_schema={}\n",
            wayexpand_core::CONTROL_STATUS_SCHEMA
        );
        let check = daemon_version_check(Some(&status));
        let identity_unverified =
            crate::doctor::status::daemon_build_unknown(Some("old-build"), Some("old-version"));
        assert_eq!(
            check.state,
            if identity_unverified {
                "unknown"
            } else {
                "fail"
            }
        );
        assert_eq!(check.problem_id, Some("WX-VERSION-001"));
        if !identity_unverified {
            assert!(check.detail.contains("old-build"));
        }
        assert_eq!(
            check.remediation_command.as_deref(),
            Some("wayexpand upgrade --apply")
        );
        let unknown = daemon_version_check(None);
        assert_eq!(unknown.state, "unknown");
        assert_eq!(unknown.problem_id, Some("WX-VERSION-001"));
    }
}
