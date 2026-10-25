//! Isolated, real-client insertion smoke certification.
//!
//! The existing KWin harness starts a private compositor, Wayland sink clients,
//! the production daemon and the production libei output backend. Capture is
//! intentionally simulated, so this report can establish output/focus
//! behavior but can never claim full desktop certification.

use crate::*;
use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const SCENARIOS: &[&str] = &[
    "expansion-replaces-trigger",
    "word-boundary-reinserts-delimiter",
    "cursor-placement",
    "multiline-replacement",
    "unicode-insertion",
    "app-filter-follows-focus",
    "focus-change-during-expansion",
    "target-closes-during-expansion",
    "command-expansion-completes",
    "config-reload",
    "daemon-restart",
    "unreachable-character-is-refused",
];

pub(crate) fn run(json: bool) -> Result<()> {
    let result = run_harness();
    let report = make_report(result);
    if json {
        println!("{report}");
    } else {
        print_human_report(&report);
    }
    if report["overall"] != "partial" {
        bail!("isolated certification did not complete successfully; see the report above");
    }
    Ok(())
}

struct HarnessResult {
    status: &'static str,
    version: Option<String>,
    tested_commit: Option<String>,
    capture_source: Option<String>,
    injection_backend: Option<String>,
    insertion_mode: Option<String>,
    backend_mode: Option<String>,
    sensitive_field_detection: Option<bool>,
    outcomes: BTreeMap<String, &'static str>,
    reason: Option<&'static str>,
}

fn run_harness() -> HarnessResult {
    let Some((script, prebuilt_bin)) = harness_paths() else {
        return unavailable("the isolated KWin harness is not included in this installation");
    };
    for dependency in ["kwin_wayland", "dbus-run-session", "gdbus", "jq", "timeout"] {
        if !command_exists(dependency) {
            return unavailable(
                "required harness dependency is missing; see docs/INTEGRATION_TESTING.md",
            );
        }
    }
    if prebuilt_bin.is_none() && !command_exists("cargo") {
        return unavailable("Rust build tools are required for the source-checkout harness");
    }

    // Limit the entire build and private-session run, including the Cargo
    // build that happens before the harness's own compositor timeout.
    let mut command = Command::new("timeout");
    command
        .args(["--signal=TERM", "--kill-after=10s", "960s"])
        .arg(&script)
        .arg("--allow-unverified-unicode")
        .args(SCENARIOS)
        .stdin(Stdio::null());
    if let Some((helper_bin, app_bin)) = prebuilt_bin {
        command
            .env("WAYEXPAND_E2E_PREBUILT_BIN", helper_bin)
            .env("WAYEXPAND_E2E_APP_BIN", app_bin);
    }
    let timeout = match command.output() {
        Ok(output) => output,
        Err(_) => return unavailable("could not start the isolated KWin harness"),
    };
    let stdout = String::from_utf8_lossy(&timeout.stdout);
    let stderr = String::from_utf8_lossy(&timeout.stderr);
    let mut outcomes = BTreeMap::new();
    let mut tested_commit = None;
    let mut capture_source = None;
    let mut injection_backend = None;
    let mut insertion_mode = None;
    let mut backend_mode = None;
    let mut sensitive_field_detection = None;
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("tested-build commit ") {
            tested_commit = nonempty(value);
            continue;
        }
        if let Some(value) = line.strip_prefix("route capture-source ") {
            capture_source = nonempty(value);
            continue;
        }
        if let Some(value) = line.strip_prefix("route injection-backend ") {
            injection_backend = nonempty(value);
            continue;
        }
        if let Some(value) = line.strip_prefix("route insertion-mode ") {
            insertion_mode = nonempty(value);
            continue;
        }
        if let Some(value) = line.strip_prefix("route backend-mode ") {
            backend_mode = nonempty(value);
            continue;
        }
        if let Some(value) = line.strip_prefix("route sensitive-field-detection ") {
            sensitive_field_detection = match value {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(status), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !SCENARIOS.contains(&name) {
            continue;
        }
        let status = match status {
            "pass" => "passed",
            "fail" => "failed",
            "unverified" => "not-tested",
            _ => "not-run",
        };
        outcomes.insert(name.to_owned(), status);
    }
    let version = stdout
        .lines()
        .find_map(|line| line.split_once("(KWin ").map(|(_, tail)| tail))
        .and_then(|tail| tail.split([',', ')']).next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if !timeout.status.success() && outcomes.is_empty() {
        let reason = if timeout.status.code() == Some(124) || timeout.status.code() == Some(137) {
            "the isolated harness exceeded its 16-minute deadline"
        } else if stderr.contains("private KWin session exceeded its") {
            "the private KWin session exceeded its deadline"
        } else if stderr.contains("error:") {
            "the isolated harness could not start; check the required desktop dependencies"
        } else {
            "the isolated harness exited before producing scenario results"
        };
        return HarnessResult {
            status: "unavailable",
            version,
            tested_commit,
            capture_source,
            injection_backend,
            insertion_mode,
            backend_mode,
            sensitive_field_detection,
            outcomes,
            reason: Some(reason),
        };
    }
    if outcomes.is_empty() {
        return HarnessResult {
            status: "unavailable",
            version,
            tested_commit,
            capture_source,
            injection_backend,
            insertion_mode,
            backend_mode,
            sensitive_field_detection,
            outcomes,
            reason: Some("the private compositor session produced no scenario results"),
        };
    }
    for scenario in SCENARIOS {
        outcomes.entry((*scenario).to_owned()).or_insert("not-run");
    }
    if matches!(timeout.status.code(), Some(124 | 137)) {
        return HarnessResult {
            status: "incomplete",
            version,
            tested_commit,
            capture_source,
            injection_backend,
            insertion_mode,
            backend_mode,
            sensitive_field_detection,
            outcomes,
            reason: Some(
                "the isolated harness exceeded its deadline after producing scenario results",
            ),
        };
    }
    // A driver can emit passing scenario lines and still fail during session
    // teardown or evidence collection. Never present that run as a successful
    // partial report: the process result is part of the evidence contract.
    if !timeout.status.success() {
        return HarnessResult {
            status: "failed",
            version,
            tested_commit,
            capture_source,
            injection_backend,
            insertion_mode,
            backend_mode,
            sensitive_field_detection,
            outcomes,
            reason: Some(
                "the isolated harness exited unsuccessfully after producing scenario results",
            ),
        };
    }
    HarnessResult {
        status: "completed",
        version,
        tested_commit,
        capture_source,
        injection_backend,
        insertion_mode,
        backend_mode,
        sensitive_field_detection,
        outcomes,
        reason: None,
    }
}

fn harness_paths() -> Option<(PathBuf, Option<(PathBuf, PathBuf)>)> {
    let executable = env::current_exe().ok()?;
    // An installed binary certifies its own installed helpers. The checkout
    // it was built from (if it still exists) may be at another commit, and
    // its script would build and test those binaries instead.
    if let Some(installed) = installed_harness(&executable) {
        return Some(installed);
    }
    // A development binary running from inside its checkout uses the source
    // harness, which builds that same checkout.
    let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .ok()?;
    let source_script = source_root.join("scripts/test-kwin-e2e.sh");
    (executable.starts_with(&source_root) && source_script.is_file())
        .then_some((source_script, None))
}

fn installed_harness(executable: &Path) -> Option<(PathBuf, Option<(PathBuf, PathBuf)>)> {
    let prefix = executable.parent()?.parent()?;
    for directory in [
        prefix.join("libexec/wayexpand-certification"),
        prefix.join("share/wayexpand/certification"),
    ] {
        let script = directory.join("test-kwin-e2e.sh");
        let bin = directory.join("bin");
        if script.is_file()
            && ["wx-e2e-sink", "wx-e2e-type", "wx-e2e-eis-relay"]
                .iter()
                .all(|name| bin.join(name).is_file())
        {
            return Some((script, Some((bin, prefix.join("bin")))));
        }
    }
    None
}

fn unavailable(reason: &'static str) -> HarnessResult {
    HarnessResult {
        status: "unavailable",
        version: None,
        tested_commit: None,
        capture_source: None,
        injection_backend: None,
        insertion_mode: None,
        backend_mode: None,
        sensitive_field_detection: None,
        outcomes: BTreeMap::new(),
        reason: Some(reason),
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn command_exists(name: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|path| env::split_paths(&path).any(|directory| directory.join(name).is_file()))
}

fn make_report(result: HarnessResult) -> serde_json::Value {
    let mut checks = vec![check(
        "simple-trigger-insertion",
        result
            .outcomes
            .get("expansion-replaces-trigger")
            .copied()
            .unwrap_or("not-run"),
        "The isolated Wayland test client verified the received replacement text.",
    )];
    checks.push(check(
        "word-boundary-delimiter",
        result
            .outcomes
            .get("word-boundary-reinserts-delimiter")
            .copied()
            .unwrap_or("not-run"),
        "A word-boundary expansion preserves the terminating delimiter after replacement.",
    ));
    checks.push(check(
        "cursor-placement",
        result
            .outcomes
            .get("cursor-placement")
            .copied()
            .unwrap_or("not-run"),
        "The isolated client verifies the final text and caret position for a cursor marker.",
    ));
    checks.push(check(
        "multiline-replacement",
        result
            .outcomes
            .get("multiline-replacement")
            .copied()
            .unwrap_or("not-run"),
        "The isolated client verifies that line breaks arrive in the replacement.",
    ));
    checks.push(check(
        "focus-switching",
        result
            .outcomes
            .get("app-filter-follows-focus")
            .copied()
            .unwrap_or("not-run"),
        "Two isolated clients checked that focus and app filtering target the intended window.",
    ));
    checks.push(check(
        "focus-change-during-expansion",
        result
            .outcomes
            .get("focus-change-during-expansion")
            .copied()
            .unwrap_or("not-run"),
        "A delayed expansion is cancelled after focus moves; the old trigger remains and the newly focused client receives no output.",
    ));
    checks.push(check(
        "target-closes-during-expansion",
        result
            .outcomes
            .get("target-closes-during-expansion")
            .copied()
            .unwrap_or("not-run"),
        "Closing the target during a delayed expansion must not send output to the client that receives focus, and the daemon must remain alive.",
    ));
    checks.push(check(
        "command-completion",
        result
            .outcomes
            .get("command-expansion-completes")
            .copied()
            .unwrap_or("not-run"),
        "A delayed command result is inserted when the original target remains focused.",
    ));
    checks.push(check(
        "configuration-reload",
        result
            .outcomes
            .get("config-reload")
            .copied()
            .unwrap_or("not-run"),
        "The isolated daemon applies a configuration edit without restarting.",
    ));
    checks.push(check(
        "daemon-restart",
        result
            .outcomes
            .get("daemon-restart")
            .copied()
            .unwrap_or("not-run"),
        "The isolated daemon reconnects to its output route and expands after restart.",
    ));
    checks.push(check(
        "unicode-insertion",
        result
            .outcomes
            .get("unicode-insertion")
            .copied()
            .unwrap_or("not-run"),
        "The isolated client checks exact UTF-8 output when the negotiated route offers direct text insertion; keysym-only routes are not counted as tested.",
    ));
    checks.push(check(
        "unavailable-unicode-refusal",
        result
            .outcomes
            .get("unreachable-character-is-refused")
            .copied()
            .unwrap_or("not-run"),
        "The test client verifies that unsupported output is refused before the trigger is erased.",
    ));
    checks.push(check(
        "suspend-resume",
        "not-tested",
        "Suspend and resume are outside this isolated smoke run.",
    ));
    checks.push(check(
        "keyboard-capture",
        "not-tested",
        "The test harness supplies trigger input over stdin; it does not test physical keyboard capture.",
    ));
    let build_matches_reporter = result
        .tested_commit
        .as_deref()
        .is_some_and(|commit| commit != "unknown" && commit == env!("WAYEXPAND_COMMIT"));
    let build_identity_status = match result.tested_commit.as_deref() {
        Some(_) if build_matches_reporter => "passed",
        Some(_) => "failed",
        None => "not-tested",
    };
    checks.push(check(
        "tested-build-identity",
        build_identity_status,
        "The CLI build identity must match the CLI and daemon binaries exercised by the isolated harness.",
    ));

    let required_checks_passed = [
        "simple-trigger-insertion",
        "word-boundary-delimiter",
        "cursor-placement",
        "multiline-replacement",
        "focus-switching",
        "focus-change-during-expansion",
        "target-closes-during-expansion",
        "command-completion",
        "configuration-reload",
        "daemon-restart",
        "unavailable-unicode-refusal",
        "tested-build-identity",
    ]
    .into_iter()
    .all(|name| {
        checks
            .iter()
            .any(|item| item["name"] == name && item["status"] == "passed")
    });
    let route_evidence_complete = result.capture_source.as_deref() == Some("stdin")
        && result.injection_backend.as_deref() == Some("libei")
        && result.insertion_mode.is_some()
        && result.backend_mode.is_some()
        && result.sensitive_field_detection.is_some();
    let any_failed = checks.iter().any(|check| check["status"] == "failed");
    let overall = if result.status == "unavailable" {
        "unavailable"
    } else if result.status == "failed" || any_failed {
        "failed"
    } else if result.status == "completed" && required_checks_passed && route_evidence_complete {
        "partial"
    } else {
        "incomplete"
    };
    let generated_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    serde_json::json!({
        "schema": 1,
        "generated_at_unix": generated_at_unix,
        "wayexpand": {
            "version": env!("WAYEXPAND_BUILD_VERSION"),
            "commit": env!("WAYEXPAND_COMMIT"),
        },
        "overall": overall,
        "certified": false,
        "test_session": {
            "desktop": "Private headless KWin / Wayland",
            "compositor": "KWin / Wayland",
            "version": result.version,
            "isolation": "private headless compositor session",
            "capture_route": "stdin simulation",
            "capture_source": result.capture_source,
            "injection_backend": result.injection_backend,
            "negotiated_insertion_mode": result.insertion_mode,
            "backend_mode_detail": result.backend_mode,
            "sensitive_field_detection": result.sensitive_field_detection,
        },
        "harness": {
            "status": result.status,
            "reason": result.reason,
            "tested_commit": result.tested_commit,
            "build_matches_reporter": build_matches_reporter,
        },
        "checks": checks,
        "limitations": [
            "This is a reproducible output/focus smoke certification, not production compositor certification.",
            "Keyboard capture, password-field behavior, suspend/resume, and long-running stability are not certified by this smoke run.",
            "The private KWin result does not describe the user's active compositor session."
        ],
    })
}

fn check(name: &str, status: &str, evidence: &str) -> serde_json::Value {
    let label = match name {
        "simple-trigger-insertion" => "Simple trigger insertion",
        "word-boundary-delimiter" => "Word-boundary delimiter",
        "cursor-placement" => "Cursor placement",
        "multiline-replacement" => "Multiline replacement",
        "unicode-insertion" => "Unicode insertion",
        "focus-switching" => "Focus switching",
        "focus-change-during-expansion" => "Focus change during expansion",
        "target-closes-during-expansion" => "Target closes during expansion",
        "command-completion" => "Command completion",
        "configuration-reload" => "Configuration reload",
        "daemon-restart" => "Daemon restart",
        "unavailable-unicode-refusal" => "Unavailable character refusal",
        "suspend-resume" => "Suspend/resume",
        "keyboard-capture" => "Keyboard capture",
        "tested-build-identity" => "Tested build identity",
        _ => name,
    };
    serde_json::json!({ "name": name, "label": label, "status": status, "evidence": evidence })
}

fn print_human_report(report: &serde_json::Value) {
    println!("WayExpand isolated desktop compatibility report");
    println!(
        "  WayExpand: {} (commit {})",
        report["wayexpand"]["version"].as_str().unwrap_or("unknown"),
        report["wayexpand"]["commit"].as_str().unwrap_or("unknown")
    );
    println!(
        "  Test session: {}",
        report["test_session"]["desktop"]
            .as_str()
            .unwrap_or("unknown")
    );
    if let Some(version) = report["test_session"]["version"].as_str() {
        println!("  Compositor: KWin {version} / Wayland");
    } else {
        println!("  Compositor: KWin / Wayland (version unavailable)");
    }
    println!("  Isolation: private headless session; current desktop is not modified");
    println!("  Capture: simulated stdin");
    if let Some(backend) = report["test_session"]["injection_backend"].as_str() {
        println!("  Injection backend: {backend}");
    }
    if let Some(mode) = report["test_session"]["negotiated_insertion_mode"].as_str() {
        println!("  Insertion mode: {mode}");
        if mode.contains("keysym fallback") {
            println!("  Warning: key-by-key fallback is slower and does not establish full Unicode support.");
        }
    }
    if let Some(detail) = report["test_session"]["backend_mode_detail"].as_str() {
        println!("  Backend detail: {detail}");
    }
    match report["test_session"]["sensitive_field_detection"].as_bool() {
        Some(true) => println!("  Sensitive-field signal: available on this route"),
        Some(false) => println!("  Sensitive-field signal: unavailable on this route"),
        None => println!("  Sensitive-field signal: not reported"),
    }
    if let Some(checks) = report["checks"].as_array() {
        for item in checks {
            let status = match item["status"].as_str().unwrap_or("unknown") {
                "passed" => "Pass",
                "failed" => "Fail",
                "not-tested" | "not-run" => "Not tested",
                other => other,
            };
            println!(
                "  {:30} {}",
                item["label"].as_str().unwrap_or("unknown"),
                status
            );
        }
    }
    let overall = match report["overall"].as_str().unwrap_or("unavailable") {
        "partial" => "Partial certification",
        "certified" => "Certified",
        "failed" => "Certification failed",
        "incomplete" => "Certification incomplete",
        "unavailable" => "Certification unavailable",
        other => other,
    };
    println!("Overall: {overall}");
    println!("This report is partial evidence, not a compositor certification.");
    if let Some(reason) = report["harness"]["reason"].as_str() {
        println!("Reason: {reason}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_completed_scenarios() -> BTreeMap<String, &'static str> {
        SCENARIOS
            .iter()
            .map(|scenario| ((*scenario).to_owned(), "passed"))
            .collect()
    }

    #[test]
    fn installed_binary_uses_the_helpers_installed_beside_it() {
        let prefix = std::env::temp_dir().join(format!(
            "wayexpand-certify-prefix-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let harness = prefix.join("libexec/wayexpand-certification");
        std::fs::create_dir_all(harness.join("bin")).unwrap();
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        let executable = prefix.join("bin/wayexpand");
        std::fs::write(&executable, "").unwrap();
        assert!(installed_harness(&executable).is_none());

        std::fs::write(harness.join("test-kwin-e2e.sh"), "").unwrap();
        for helper in ["wx-e2e-sink", "wx-e2e-type", "wx-e2e-eis-relay"] {
            std::fs::write(harness.join("bin").join(helper), "").unwrap();
        }
        let (script, prebuilt) = installed_harness(&executable).unwrap();
        assert_eq!(script, harness.join("test-kwin-e2e.sh"));
        assert_eq!(prebuilt, Some((harness.join("bin"), prefix.join("bin"))));
        std::fs::remove_dir_all(prefix).unwrap();
    }

    #[test]
    fn timeout_after_passing_scenarios_is_incomplete_not_failed_or_partial() {
        let report = make_report(HarnessResult {
            status: "incomplete",
            version: Some("test".into()),
            tested_commit: None,
            capture_source: None,
            injection_backend: None,
            insertion_mode: None,
            backend_mode: None,
            sensitive_field_detection: None,
            outcomes: all_completed_scenarios(),
            reason: Some("deadline"),
        });

        assert_eq!(report["overall"], "incomplete");
        assert_eq!(report["certified"], false);
    }

    #[test]
    fn unexpected_harness_exit_cannot_be_reported_as_partial() {
        let report = make_report(HarnessResult {
            status: "failed",
            version: Some("test".into()),
            tested_commit: None,
            capture_source: None,
            injection_backend: None,
            insertion_mode: None,
            backend_mode: None,
            sensitive_field_detection: None,
            outcomes: all_completed_scenarios(),
            reason: Some("exit failure"),
        });

        assert_eq!(report["overall"], "failed");
        assert_eq!(report["certified"], false);
    }

    #[test]
    fn compatibility_report_records_the_negotiated_test_route() {
        let report = make_report(HarnessResult {
            status: "completed",
            version: Some("6.6.6".into()),
            tested_commit: Some(env!("WAYEXPAND_COMMIT").into()),
            capture_source: Some("stdin".into()),
            injection_backend: Some("libei".into()),
            insertion_mode: Some("libei keysym fallback".into()),
            backend_mode: Some("ei_keyboard keysym fallback (12ms key pacing)".into()),
            sensitive_field_detection: Some(false),
            outcomes: all_completed_scenarios(),
            reason: None,
        });

        assert!(report["generated_at_unix"].as_u64().unwrap() > 0);
        assert_eq!(
            report["wayexpand"]["version"],
            env!("WAYEXPAND_BUILD_VERSION")
        );
        assert_eq!(report["wayexpand"]["commit"], env!("WAYEXPAND_COMMIT"));
        assert_eq!(report["test_session"]["capture_route"], "stdin simulation");
        assert_eq!(report["test_session"]["capture_source"], "stdin");
        assert_eq!(report["test_session"]["injection_backend"], "libei");
        assert_eq!(
            report["test_session"]["negotiated_insertion_mode"],
            "libei keysym fallback"
        );
        assert_eq!(
            report["test_session"]["backend_mode_detail"],
            "ei_keyboard keysym fallback (12ms key pacing)"
        );
        assert_eq!(report["test_session"]["sensitive_field_detection"], false);
        assert_eq!(report["harness"]["build_matches_reporter"], true);
        assert_eq!(report["harness"]["tested_commit"], env!("WAYEXPAND_COMMIT"));
        assert_eq!(report["certified"], false);
        assert_eq!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|check| check["name"] == "focus-change-during-expansion")
                .unwrap()["status"],
            "passed"
        );
        assert_eq!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|check| check["name"] == "target-closes-during-expansion")
                .unwrap()["status"],
            "passed"
        );
    }

    #[test]
    fn missing_negotiated_route_evidence_keeps_the_report_incomplete() {
        let report = make_report(HarnessResult {
            status: "completed",
            version: Some("6.6.6".into()),
            tested_commit: None,
            capture_source: None,
            injection_backend: None,
            insertion_mode: None,
            backend_mode: None,
            sensitive_field_detection: None,
            outcomes: all_completed_scenarios(),
            reason: None,
        });

        assert_eq!(report["overall"], "incomplete");
        assert_eq!(report["certified"], false);
    }

    #[test]
    fn missing_focus_safety_scenario_keeps_the_report_incomplete() {
        let mut outcomes = all_completed_scenarios();
        outcomes.remove("target-closes-during-expansion");
        let report = make_report(HarnessResult {
            status: "completed",
            version: Some("6.6.6".into()),
            tested_commit: Some(env!("WAYEXPAND_COMMIT").into()),
            capture_source: Some("stdin".into()),
            injection_backend: Some("libei".into()),
            insertion_mode: Some("libei ei_text".into()),
            backend_mode: Some("ei_keyboard and ei_text".into()),
            sensitive_field_detection: Some(false),
            outcomes,
            reason: None,
        });

        assert_eq!(report["overall"], "incomplete");
        assert_eq!(report["certified"], false);
        assert_eq!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|check| check["name"] == "target-closes-during-expansion")
                .unwrap()["status"],
            "not-run"
        );
    }

    #[test]
    fn missing_reload_scenario_keeps_the_report_incomplete() {
        let mut outcomes = all_completed_scenarios();
        outcomes.remove("config-reload");
        let report = make_report(HarnessResult {
            status: "completed",
            version: Some("6.6.6".into()),
            tested_commit: Some(env!("WAYEXPAND_COMMIT").into()),
            capture_source: Some("stdin".into()),
            injection_backend: Some("libei".into()),
            insertion_mode: Some("libei ei_text".into()),
            backend_mode: Some("ei_keyboard and ei_text".into()),
            sensitive_field_detection: Some(false),
            outcomes,
            reason: None,
        });

        assert_eq!(report["overall"], "incomplete");
        assert_eq!(report["certified"], false);
        assert_eq!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|check| check["name"] == "configuration-reload")
                .unwrap()["status"],
            "not-run"
        );
    }

    #[test]
    fn stale_cli_cannot_claim_partial_certification_of_newer_test_binaries() {
        let report = make_report(HarnessResult {
            status: "completed",
            version: Some("6.6.6".into()),
            tested_commit: Some("different-build".into()),
            capture_source: Some("stdin".into()),
            injection_backend: Some("libei".into()),
            insertion_mode: Some("libei ei_text".into()),
            backend_mode: Some("ei_keyboard and ei_text".into()),
            sensitive_field_detection: Some(false),
            outcomes: all_completed_scenarios(),
            reason: None,
        });

        assert_eq!(report["overall"], "failed");
        assert_eq!(report["certified"], false);
        assert_eq!(report["harness"]["build_matches_reporter"], false);
        assert_eq!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|check| check["name"] == "tested-build-identity")
                .unwrap()["status"],
            "failed"
        );
    }
}
