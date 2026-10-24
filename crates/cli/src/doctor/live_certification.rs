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
};

const SCENARIOS: &[&str] = &[
    "expansion-replaces-trigger",
    "unicode-insertion",
    "app-filter-follows-focus",
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
    for line in stdout.lines() {
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
        } else if stderr.contains("error:") {
            "the isolated harness could not start; check the required desktop dependencies"
        } else {
            "the isolated harness exited before producing scenario results"
        };
        return HarnessResult {
            status: "unavailable",
            version,
            outcomes,
            reason: Some(reason),
        };
    }
    if outcomes.is_empty() {
        return HarnessResult {
            status: "unavailable",
            version,
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
            outcomes,
            reason: Some(
                "the isolated harness exited unsuccessfully after producing scenario results",
            ),
        };
    }
    HarnessResult {
        status: "completed",
        version,
        outcomes,
        reason: None,
    }
}

fn harness_paths() -> Option<(PathBuf, Option<(PathBuf, PathBuf)>)> {
    let source_script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-kwin-e2e.sh");
    if source_script.is_file() {
        return Some((source_script, None));
    }

    let executable = env::current_exe().ok()?;
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
        outcomes: BTreeMap::new(),
        reason: Some(reason),
    }
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
        "focus-switching",
        result
            .outcomes
            .get("app-filter-follows-focus")
            .copied()
            .unwrap_or("not-run"),
        "Two isolated clients checked that focus and app filtering target the intended window.",
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

    let required_checks_passed = [0, 1, 3]
        .into_iter()
        .all(|index| checks[index]["status"] == "passed");
    let any_failed = checks.iter().any(|check| check["status"] == "failed");
    let overall = if result.status == "unavailable" {
        "unavailable"
    } else if result.status == "failed" || any_failed {
        "failed"
    } else if result.status == "completed" && required_checks_passed {
        "partial"
    } else {
        "incomplete"
    };
    serde_json::json!({
        "schema": 1,
        "overall": overall,
        "certified": false,
        "test_session": {
            "desktop": "KDE Plasma / Wayland",
            "compositor": "KWin / Wayland",
            "version": result.version,
            "isolation": "private headless compositor session",
            "capture_route": "stdin simulation",
            "injection_backend": "production libei backend over a private EIS relay",
        },
        "harness": {
            "status": result.status,
            "reason": result.reason,
        },
        "checks": checks,
        "limitations": [
            "This is a reproducible output/focus smoke certification, not production compositor certification.",
            "Keyboard capture, Unicode insertion, password-field behavior, suspend/resume, and long-running stability are not certified.",
            "The private KWin result does not describe the user's active compositor session."
        ],
    })
}

fn check(name: &str, status: &str, evidence: &str) -> serde_json::Value {
    let label = match name {
        "simple-trigger-insertion" => "Simple trigger insertion",
        "unicode-insertion" => "Unicode insertion",
        "focus-switching" => "Focus switching",
        "unavailable-unicode-refusal" => "Unavailable character refusal",
        "suspend-resume" => "Suspend/resume",
        "keyboard-capture" => "Keyboard capture",
        _ => name,
    };
    serde_json::json!({ "name": name, "label": label, "status": status, "evidence": evidence })
}

fn print_human_report(report: &serde_json::Value) {
    println!("WayExpand isolated desktop compatibility report");
    println!("  Desktop: {}", report["test_session"]["desktop"]);
    if let Some(version) = report["test_session"]["version"].as_str() {
        println!("  Compositor: KWin {version} / Wayland");
    } else {
        println!("  Compositor: KWin / Wayland (version unavailable)");
    }
    println!("  Isolation: private headless session; current desktop is not modified");
    println!("  Capture: simulated stdin · injection: libei/EIS");
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
    fn timeout_after_passing_scenarios_is_incomplete_not_failed_or_partial() {
        let report = make_report(HarnessResult {
            status: "incomplete",
            version: Some("test".into()),
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
            outcomes: all_completed_scenarios(),
            reason: Some("exit failure"),
        });

        assert_eq!(report["overall"], "failed");
        assert_eq!(report["certified"], false);
    }
}
