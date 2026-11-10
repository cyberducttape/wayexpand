//! Sanitized, bounded diagnostics for the currently selected input route.
use super::Args;
use anyhow::{bail, Result};
use std::{
    collections::{HashSet, VecDeque},
    io::{self, Write},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn focused_app_scope_from_status(status_response: Option<&str>) -> (Option<String>, Option<bool>) {
    let Some(status_response) = status_response else {
        return (None, None);
    };
    let status = wayexpand_core::DaemonStatus::parse(status_response);
    (
        status.field("capture_app_scope_state").map(str::to_owned),
        status.bool_field("terminal_apps_only"),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DesktopTestResult {
    Verified,
    NotTested,
    Failed,
    AwaitingReview,
}

#[derive(Default)]
struct DesktopTestOptions {
    application_class: Option<String>,
    focus_confirmed: bool,
    observe_only: bool,
}

pub(crate) fn debug_command(args: Args) -> Result<()> {
    let mut live = false;
    let mut json = false;
    let mut seconds = 60_u64;
    let mut trigger: Option<String> = None;
    let mut test_trigger: Option<String> = None;
    let mut test_application_class: Option<String> = None;
    let mut test_focus_confirmed = false;
    let mut test_observe_only = false;
    let mut gnome_interactive = false;
    let mut gnome = false;
    let mut focused_app_id = false;
    let mut values: Vec<String> = args.collect();
    let mut index = 0;
    while index < values.len() {
        match values[index].as_str() {
            "--live" => {
                live = true;
                values.remove(index);
            }
            "--json" => {
                json = true;
                values.remove(index);
            }
            "--gnome" => {
                gnome = true;
                values.remove(index);
            }
            "--focused-app-id" => {
                focused_app_id = true;
                values.remove(index);
            }
            "--interactive" => {
                gnome_interactive = true;
                values.remove(index);
            }
            "--seconds" => {
                if index + 1 >= values.len() {
                    bail!("--seconds requires a value from 1 to 300");
                }
                seconds = values[index + 1]
                    .parse::<u64>()
                    .ok()
                    .filter(|n| (1..=300).contains(n))
                    .ok_or_else(|| anyhow::anyhow!("--seconds requires a value from 1 to 300"))?;
                values.drain(index..=index + 1);
            }
            "--trigger" => {
                if index + 1 >= values.len() {
                    bail!("--trigger requires text to explain");
                }
                trigger = Some(values.remove(index + 1));
                values.remove(index);
            }
            "--test" => {
                if index + 1 >= values.len() {
                    bail!("--test requires a configured trigger to test");
                }
                test_trigger = Some(values.remove(index + 1));
                values.remove(index);
            }
            "--application-class" => {
                if index + 1 >= values.len() {
                    bail!("--application-class requires a non-empty application class");
                }
                test_application_class = Some(values.remove(index + 1));
                values.remove(index);
            }
            "--focus-confirmed" => {
                test_focus_confirmed = true;
                values.remove(index);
            }
            "--observe-only" => {
                test_observe_only = true;
                values.remove(index);
            }
            _ => {
                index += 1;
            }
        }
    }
    if !values.is_empty() {
        bail!("usage: wayexpand debug [--json] [--trigger TEXT] [--live [--seconds 1..300]] | --focused-app-id [--json] | --test TRIGGER [--application-class CLASS --focus-confirmed [--observe-only]]");
    }
    if focused_app_id {
        if live
            || trigger.is_some()
            || test_trigger.is_some()
            || test_application_class.is_some()
            || test_focus_confirmed
            || test_observe_only
            || gnome
            || gnome_interactive
        {
            bail!("--focused-app-id cannot be combined with other debug modes");
        }
        let response = crate::control_operation(wayexpand_core::DaemonOperation::FocusedAppId)?;
        if response.starts_with("unknown command;") {
            bail!("the running daemon does not support focused-app diagnostics; update and restart the daemon, then retry");
        }
        let app_id = response
            .lines()
            .find_map(|line| line.strip_prefix("focused_app_id="))
            .ok_or_else(|| anyhow::anyhow!("daemon returned an invalid focused app ID response"))?;
        // Read the scope decision beside the identity. This turns the command
        // into a useful diagnosis for terminal-scoped evdev routes instead of
        // making users compare the ID against configuration by hand. Older
        // daemons may not publish these fields, so keep them optional.
        let status_response =
            crate::control_operation(wayexpand_core::DaemonOperation::Status).ok();
        let (scope_state, terminal_apps_only) =
            focused_app_scope_from_status(status_response.as_deref());
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "focused_app_id": if app_id == "unavailable" { None } else { Some(app_id) },
                    "capture_app_scope_state": scope_state,
                    "terminal_apps_only": terminal_apps_only,
                })
            );
        } else if app_id == "unavailable" {
            println!("Focused application ID: unavailable");
            if let Some(scope_state) = scope_state {
                println!("Capture app scope: {scope_state}");
            }
        } else {
            println!("Focused application ID: {app_id}");
            if let Some(scope_state) = scope_state {
                println!("Capture app scope: {scope_state}");
                if terminal_apps_only == Some(true) && scope_state == "outside_allowlist" {
                    println!("Expansion matching is suppressed for this focused application.");
                }
            }
        }
        return Ok(());
    }
    if gnome || gnome_interactive {
        if !gnome
            || !gnome_interactive
            || live
            || json
            || trigger.is_some()
            || test_trigger.is_some()
            || test_application_class.is_some()
            || test_focus_confirmed
            || test_observe_only
        {
            bail!("usage: wayexpand debug --gnome --interactive");
        }
        return gnome_interactive_test();
    }
    if test_trigger.is_some() && (live || json || trigger.is_some()) {
        bail!("--test cannot be combined with --json, --trigger, or --live");
    }
    validate_desktop_test_options(
        test_trigger.is_some(),
        test_application_class.as_deref(),
        test_focus_confirmed,
        test_observe_only,
    )?;
    if let Some(trigger) = test_trigger {
        return desktop_test_for(
            &trigger,
            false,
            DesktopTestOptions {
                application_class: test_application_class,
                focus_confirmed: test_focus_confirmed,
                observe_only: test_observe_only,
            },
        )
        .map(|_| ());
    }
    let report = crate::diagnostics::collect(trigger.as_deref());
    if json && live {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        crate::diagnostics::print(&report, json)?;
    }
    if live {
        run_live(seconds, json)?;
    }
    Ok(())
}

fn validate_desktop_test_options(
    has_test_trigger: bool,
    application_class: Option<&str>,
    focus_confirmed: bool,
    observe_only: bool,
) -> Result<()> {
    if !has_test_trigger && (application_class.is_some() || focus_confirmed || observe_only) {
        bail!("--application-class, --focus-confirmed, and --observe-only apply only to --test");
    }
    if focus_confirmed != application_class.is_some() {
        bail!("--focus-confirmed and --application-class must be provided together");
    }
    if observe_only && !focus_confirmed {
        bail!("--observe-only requires an explicit --application-class and --focus-confirmed");
    }
    if let Some(application_class) = application_class {
        if application_class.trim().is_empty()
            || application_class.chars().count() > 64
            || application_class.chars().any(char::is_control)
        {
            bail!("--application-class must be 1 to 64 characters without controls");
        }
    }
    Ok(())
}

/// Interactive, target-application test. It never sends Enter, and only
/// accepts a static, enabled snippet without command execution or form fields.
pub(crate) fn desktop_test(trigger: &str) -> Result<DesktopTestResult> {
    desktop_test_for(trigger, false, DesktopTestOptions::default())
}

fn desktop_test_for(
    trigger: &str,
    gnome: bool,
    options: DesktopTestOptions,
) -> Result<DesktopTestResult> {
    if trigger.is_empty() || trigger.chars().any(char::is_control) {
        bail!("test trigger must be non-empty and contain no control characters");
    }
    let config = wayexpand_core::Config::load(wayexpand_core::default_config_path())
        .map_err(|error| anyhow::anyhow!("configuration check failed: {}", error.safe_summary()))?;
    let Some(expansion) = config
        .expansion
        .iter()
        .find(|item| item.is_keyboard_matchable() && item.answers_to(trigger))
    else {
        bail!("no enabled expansion matches that trigger; run `wayexpand debug --trigger <trigger>` to inspect matching")
    };
    if expansion.command.is_some() || expansion.replacement.contains("{{") {
        bail!("desktop test is limited to static text snippets; command-backed and templated snippets are excluded")
    }
    let test_id = diagnostic_test_id();
    println!("WayExpand Desktop Integration Test");
    println!("Diagnostic test ID: {test_id}");
    println!("Configuration [{test_id}]: PASS — enabled static expansion found");
    println!("Capture [{test_id}]: pending — no input content will be recorded");
    println!("Output [{test_id}]: pending — no Enter key will be sent");
    println!("Replacement [{test_id}]: pending — you will verify the final text in the target app");
    println!("\nReview the expected text with `wayexpand preview {trigger:?}` first.");
    println!("This test types the configured replacement into the focused application when you type {trigger:?}.");
    let application_class = if let Some(application_class) = options.application_class.as_deref() {
        application_class.trim().to_owned()
    } else {
        print!(
            "Target application class (GTK/Qt/Electron/terminal/browser/remote/sandboxed/other): "
        );
        io::stdout().flush()?;
        let mut application_class = String::new();
        io::stdin().read_line(&mut application_class)?;
        application_class.trim().to_owned()
    };
    if application_class.is_empty() || application_class.chars().any(char::is_control) {
        bail!("enter a non-empty application class without control characters");
    }
    if !options.focus_confirmed {
        print!("Focus a disposable text field, then continue? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Test cancelled; no text was inserted.");
            println!("DESKTOP TEST RESULT: CANCELLED");
            return Ok(DesktopTestResult::NotTested);
        }
    }
    let (initial_daemon, _, base_input, base_keys, base_expansions, base_ibus_transaction) =
        snapshot();
    let mut known_transactions: HashSet<String> = initial_daemon
        .as_ref()
        .and_then(|s| s.get("recent_expansion_outcomes"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|record| {
            record
                .get("transaction_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    if let Some(id) = base_ibus_transaction {
        known_transactions.insert(id);
    }
    println!("\nFor the next 30 seconds, switch to the target application and type the trigger once. Do not press Enter.");
    let mut capture_activity_seen = false;
    let mut output_seen = false;
    let mut transaction = None;
    let mut completion = None;
    let input_deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < input_deadline {
        thread::sleep(Duration::from_millis(250));
        let (_, _, input_events, keys, expansions, ibus_transaction) = snapshot();
        if input_events > base_input || keys > base_keys {
            capture_activity_seen = true;
            break;
        }
        if expansions > base_expansions {
            capture_activity_seen = true;
            output_seen = true;
            completion = Some("activity observed (IBus aggregate counter; test trigger not independently confirmed)".into());
            transaction = ibus_transaction;
            break;
        }
    }
    if !capture_activity_seen {
        println!(
            "Capture [{test_id}]: FAIL — no sanitized input activity reached WayExpand within 30 seconds."
        );
        println!("Next: check the active input method with `wayexpand ibus status` and inspect `wayexpand debug --live`.");
        println!("DESKTOP TEST RESULT: INPUT NOT OBSERVED");
        if gnome {
            println!("Problem: WX-GNOME-IM-003 — GNOME/IBus is configured, but no safe-field input activity was observed. Check the selected GNOME source and whether this application uses IBus.");
        }
        return Ok(DesktopTestResult::Failed);
    }
    println!("Capture [{test_id}]: ACTIVITY OBSERVED — test trigger not independently confirmed; aggregate counters do not correlate events to this test ID.");
    if !output_seen {
        let output_deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < output_deadline {
            thread::sleep(Duration::from_millis(250));
            let (daemon, _, _, _, expansions, ibus_transaction) = snapshot();
            if expansions > base_expansions {
                output_seen = true;
                completion = Some("activity observed (IBus aggregate counter; test trigger not independently confirmed)".into());
                transaction = ibus_transaction;
                break;
            }
            if let Some(record) = daemon
                .as_ref()
                .and_then(|s| s.get("recent_expansion_outcomes"))
                .and_then(serde_json::Value::as_array)
                .and_then(|records| {
                    records.iter().find(|r| {
                        r.get("transaction_id")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|id| !known_transactions.contains(id))
                    })
                })
            {
                transaction = record
                    .get("transaction_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                completion = record
                    .get("completion")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                output_seen = true;
                break;
            }
        }
    }
    if !output_seen {
        println!("Output [{test_id}]: NO CORRELATED RESULT — no output completion was reported during this test window.");
        println!("No second insertion was attempted. Inspect `wayexpand debug --live` and the target field.");
        println!("DESKTOP TEST RESULT: NO OUTPUT OUTCOME");
        if gnome {
            println!("Problem: WX-GNOME-IM-004 — activity was observed, but no expansion outcome was correlated with this test window. Check matching, configuration and replacement consent.");
        }
        return Ok(DesktopTestResult::Failed);
    }
    println!(
        "Output activity [{test_id}]: {}",
        completion.as_deref().unwrap_or("completion reported")
    );
    if let Some(id) = &transaction {
        println!("Observed backend transaction ID (time-window only; not linked to this test trigger): {id}");
    }
    println!("Correlation [{test_id}]: backend events are not bound to this diagnostic ID; verify the focused application's result manually.");
    if completion.as_deref().is_some_and(|value| {
        value == "partial_write" || value.contains("unknown") || value.contains("not applied")
    }) {
        println!("Output outcome is uncertain or failed. Do not retry automatically; inspect the target field manually.");
        println!("DESKTOP TEST RESULT: UNCERTAIN OUTPUT");
        println!(
            "Test scope: {}; result requires manual review.",
            application_class
        );
        return Ok(DesktopTestResult::Failed);
    }
    if options.observe_only {
        println!("DESKTOP TEST RESULT: OUTPUT OBSERVED, AWAITING REVIEW");
        println!("Correlation: backend activity was not bound to this diagnostic test ID.");
        println!("Confirm the exact target text visually in the GUI before recording a pass.");
        return Ok(DesktopTestResult::AwaitingReview);
    }
    println!("Check the target application, then return here. Was the expected text inserted exactly once, without loss or interleaving? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let verified = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    println!(
        "Replacement [{test_id}]: {} — application class reported by user: {}",
        if verified {
            "PASS"
        } else {
            "FAIL / NEEDS REVIEW"
        },
        application_class
    );
    println!("Test scope: user-reported {} only. This does not certify other toolkits, terminals, browsers, remote sessions, or sandboxed apps.", application_class);
    println!(
        "DESKTOP TEST RESULT: {}",
        if verified {
            "USER CONFIRMED"
        } else {
            "NOT CONFIRMED"
        }
    );
    if gnome && !verified {
        println!("Problem: WX-GNOME-IM-005 — output activity was reported, but the user did not confirm exact target text. Inspect the field; do not retry automatically.");
    }
    Ok(if verified {
        DesktopTestResult::Verified
    } else {
        DesktopTestResult::Failed
    })
}

fn gnome_interactive_test() -> Result<()> {
    let report = wayexpand_backend_ibus::chain::diagnose();
    if report.desktop != wayexpand_backend_ibus::chain::IbusDesktop::Gnome {
        bail!("--gnome --interactive requires a GNOME session; run `wayexpand doctor --gnome` for available diagnostics");
    }
    println!("GNOME interactive test — no keystrokes or snippet contents are recorded.");
    for id in [
        "gnome-session",
        "daemon",
        "component",
        "registered",
        "gnome-source-registered",
        "gnome-source-active",
        "consent",
    ] {
        if let Some(check) = report.check(id) {
            println!(
                "{} {}: {}",
                check.state.symbol(),
                check.title,
                check.state.label()
            );
            if check.state != wayexpand_backend_ibus::chain::IbusCheckState::Pass {
                let problem = check.problem_id.unwrap_or(match id {
                    "gnome-session" => "WX-GNOME-SESSION-001",
                    "daemon" => "WX-GNOME-IBUS-001",
                    "component" | "registered" => "WX-IBUS-001",
                    "gnome-source-active" => "WX-GNOME-IM-002",
                    "consent" => "WX-POLICY-001",
                    _ => "WX-GNOME-IM-001",
                });
                println!("Problem: {problem}");
                println!("Next: run `wayexpand doctor --gnome` and follow its fix. The test stopped before inserting anything.");
                return Ok(());
            }
        }
    }
    let Some(trigger) = wayexpand_core::Config::load(wayexpand_core::default_config_path())
        .ok()
        .and_then(|config| crate::commands::system::setup_test_trigger(&config))
    else {
        println!("No enabled static expansion is available for the harmless test.");
        println!(
            "Add a harmless static expansion, then rerun `wayexpand debug --gnome --interactive`."
        );
        return Ok(());
    };
    println!("Using an enabled static snippet for the optional test; `;;hello` is preferred when available.");
    println!("Use a normal GNOME text field. Do not use a password field, terminal command prompt, or production session.");
    let test_result = desktop_test_for(&trigger, true, DesktopTestOptions::default())?;
    if test_result == DesktopTestResult::Verified {
        println!("GNOME test: user-confirmed insertion in the reported application only.");
        println!("Logout persistence and other application/toolkit coverage remain untested.");
    } else if test_result == DesktopTestResult::NotTested {
        println!("GNOME test was cancelled; no final insertion result is available.");
    } else {
        println!("GNOME test not verified. Run `wayexpand doctor --gnome` for the next likely failing stage.");
    }
    Ok(())
}

fn diagnostic_test_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!(
        "desktop-{:x}-{:x}-{:x}",
        std::process::id(),
        timestamp,
        sequence
    )
}

#[cfg(test)]
mod desktop_test_option_tests {
    use super::validate_desktop_test_options;

    #[test]
    fn observe_only_requires_a_test_class_and_explicit_focus_acknowledgment() {
        assert!(validate_desktop_test_options(true, Some("GTK"), true, true).is_ok());
        assert!(validate_desktop_test_options(true, None, true, true).is_err());
        assert!(validate_desktop_test_options(true, Some("GTK"), false, true).is_err());
        assert!(validate_desktop_test_options(false, Some("GTK"), true, true).is_err());
    }

    #[test]
    fn application_class_is_bounded_and_control_free() {
        assert!(validate_desktop_test_options(true, Some("GTK"), true, true).is_ok());
        assert!(validate_desktop_test_options(true, Some(""), true, true).is_err());
        assert!(validate_desktop_test_options(true, Some("gtk\nterminal"), true, true).is_err());
        assert!(validate_desktop_test_options(true, Some(&"a".repeat(65)), true, true).is_err());
    }
}

#[cfg(test)]
mod focused_app_scope_tests {
    use super::focused_app_scope_from_status;

    #[test]
    fn reports_terminal_scope_and_fail_closed_allowlist_mismatch() {
        assert_eq!(
            focused_app_scope_from_status(Some(
                "running\nterminal_apps_only=true\ncapture_app_scope_state=outside_allowlist\n",
            )),
            (Some("outside_allowlist".into()), Some(true))
        );
    }

    #[test]
    fn old_or_ambiguous_status_keeps_scope_unavailable() {
        assert_eq!(focused_app_scope_from_status(None), (None, None));
        assert_eq!(
            focused_app_scope_from_status(Some(
                "running\nterminal_apps_only=true\ncapture_app_scope_state=allowed\ncapture_app_scope_state=outside_allowlist\n",
            )),
            (None, Some(true))
        );
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn snapshot() -> (
    Option<serde_json::Value>,
    wayexpand_backend_ibus::chain::IbusReadiness,
    u64,
    u64,
    u64,
    Option<String>,
) {
    let daemon = crate::doctor::status::read_daemon_status()
        .and_then(|response| crate::doctor::status::status_as_json(&response))
        .ok();
    let ibus = wayexpand_backend_ibus::chain::diagnose();
    let readiness = ibus.readiness();
    let activity = ibus.activity;
    let keys = activity.as_ref().map_or(0, |a| a.status.totals.keys);
    let expansions = activity.as_ref().map_or(0, |a| a.status.totals.expansions);
    let ibus_transaction = activity
        .as_ref()
        .and_then(|a| a.status.last_transaction_id.clone());
    let input_events = daemon
        .as_ref()
        .and_then(|s| s.get("input_events_total"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    (
        daemon,
        readiness,
        input_events,
        keys,
        expansions,
        ibus_transaction,
    )
}

fn emit(json: bool, started: Instant, event: &str, detail: &str) {
    let elapsed_ms = started.elapsed().as_millis();
    if json {
        println!(
            "{}",
            serde_json::json!({"type":"event","elapsed_ms":elapsed_ms,"at_unix":now_seconds(),"event":event,"detail":detail,"content_logged":false})
        );
    } else {
        println!("+{elapsed_ms:>6} ms  {event}: {detail}");
    }
}

fn status_counter(status: Option<&serde_json::Value>, key: &str) -> u64 {
    status
        .and_then(|snapshot| snapshot.get(key))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .unwrap_or(0)
}

fn app_scope_detail(state: &str) -> &'static str {
    match state {
        "unrestricted" => "application scope is unrestricted by this daemon route",
        "allowed" => "focused application is in the configured capture allowlist",
        "outside_allowlist" => "focused application is outside the configured capture allowlist; matching is suppressed",
        _ => "focused application identity is unavailable; app-scoped matching fails closed",
    }
}

fn run_live(seconds: u64, json: bool) -> Result<()> {
    let started = Instant::now();
    if json {
        println!(
            "{}",
            serde_json::json!({"type":"live_start","duration_seconds":seconds,"raw_text_logging":false,"replacement_logging":false,"clipboard_logging":false})
        );
    } else {
        println!("\nLive debugging enabled for {seconds}s\nSensitive text logging: DISABLED\nClipboard and replacement contents: DISABLED");
    }
    let (
        initial_daemon,
        initial_ibus,
        mut last_input_events,
        mut last_keys,
        mut last_expansions,
        mut last_ibus_transaction,
    ) = snapshot();
    let mut last_state = initial_daemon
        .as_ref()
        .and_then(|s| s.get("state"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unavailable")
        .to_owned();
    let mut last_ibus_selected = initial_ibus.selected;
    let initial_scope = initial_daemon
        .as_ref()
        .and_then(|status| status.get("capture_app_scope_state"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let mut last_scope = initial_scope.to_owned();
    let mut last_matcher_evaluations =
        status_counter(initial_daemon.as_ref(), "matcher_evaluations_total");
    let mut last_matches = status_counter(initial_daemon.as_ref(), "matches_found_total");
    let mut last_attempts = status_counter(initial_daemon.as_ref(), "expansion_attempts_total");
    let mut last_applied = status_counter(initial_daemon.as_ref(), "expansions_applied_total");
    let mut last_refused = status_counter(initial_daemon.as_ref(), "output_refused_total");
    let mut last_partial = status_counter(initial_daemon.as_ref(), "partial_write_total");
    let mut last_evdev_raw_keys =
        status_counter(initial_daemon.as_ref(), "evdev_raw_key_events_total");
    let mut last_evdev_layout_unavailable = status_counter(
        initial_daemon.as_ref(),
        "evdev_layout_unavailable_key_events_total",
    );
    emit(
        json,
        started,
        "session",
        &format!("initial state {last_state}"),
    );
    emit(json, started, "app_scope", app_scope_detail(&last_scope));
    if initial_ibus.selected {
        emit(json, started, "ibus", "WayExpand input method selected");
    }
    let mut seen = VecDeque::<String>::new();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(500));
        let (daemon, ibus, input_events, keys, expansions, ibus_transaction) = snapshot();
        let state = daemon
            .as_ref()
            .and_then(|s| s.get("state"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unavailable");
        if state != last_state {
            emit(json, started, "route_state", state);
            last_state = state.to_owned();
        }
        let scope = daemon
            .as_ref()
            .and_then(|status| status.get("capture_app_scope_state"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        if scope != last_scope {
            emit(json, started, "app_scope", app_scope_detail(scope));
            last_scope = scope.to_owned();
        }
        if ibus.selected && !last_ibus_selected {
            emit(json, started, "input_context", "WayExpand engine selected");
        }
        last_ibus_selected = ibus.selected;
        if input_events > last_input_events {
            emit(
                json,
                started,
                "input_activity",
                &format!(
                    "{} sanitized text/key event(s) reached the matcher; text omitted",
                    input_events - last_input_events
                ),
            );
            last_input_events = input_events;
        }
        let evdev_raw_keys = status_counter(daemon.as_ref(), "evdev_raw_key_events_total");
        if evdev_raw_keys < last_evdev_raw_keys {
            emit(
                json,
                started,
                "evdev_capture",
                "raw key counter reset; new daemon input session observed",
            );
            last_evdev_raw_keys = 0;
            last_evdev_layout_unavailable = 0;
        }
        if evdev_raw_keys > last_evdev_raw_keys {
            emit(
                json,
                started,
                "evdev_capture",
                &format!(
                    "{} raw evdev key event(s) observed; key values omitted",
                    evdev_raw_keys - last_evdev_raw_keys
                ),
            );
            last_evdev_raw_keys = evdev_raw_keys;
        }
        let layout_unavailable =
            status_counter(daemon.as_ref(), "evdev_layout_unavailable_key_events_total");
        if layout_unavailable > last_evdev_layout_unavailable {
            emit(
                json,
                started,
                "capture_layout",
                &format!(
                    "{} raw key event(s) were ignored because the active keyboard layout was unavailable",
                    layout_unavailable - last_evdev_layout_unavailable
                ),
            );
        }
        last_evdev_layout_unavailable = layout_unavailable;
        if keys < last_keys {
            emit(
                json,
                started,
                "ibus_engine",
                "engine counters reset; new engine session observed",
            );
            last_keys = 0;
            last_expansions = 0;
        }
        if keys > last_keys {
            emit(
                json,
                started,
                "input_activity",
                &format!(
                    "{} sanitized key event(s) observed; text omitted",
                    keys - last_keys
                ),
            );
            last_keys = keys;
        }
        if expansions > last_expansions {
            emit(
                json,
                started,
                "expansion_activity",
                &format!(
                    "{} expansion(s) committed by IBus; trigger and replacement omitted",
                    expansions - last_expansions
                ),
            );
            last_expansions = expansions;
        }
        let matcher_evaluations = status_counter(daemon.as_ref(), "matcher_evaluations_total");
        let matches = status_counter(daemon.as_ref(), "matches_found_total");
        let attempts = status_counter(daemon.as_ref(), "expansion_attempts_total");
        let applied = status_counter(daemon.as_ref(), "expansions_applied_total");
        if matcher_evaluations > last_matcher_evaluations
            || matches > last_matches
            || attempts > last_attempts
            || applied > last_applied
        {
            emit(
                json,
                started,
                "matcher",
                &format!(
                    "{} evaluation(s), {} match(es), {} insertion attempt(s), {} applied; content omitted",
                    matcher_evaluations.saturating_sub(last_matcher_evaluations),
                    matches.saturating_sub(last_matches),
                    attempts.saturating_sub(last_attempts),
                    applied.saturating_sub(last_applied),
                ),
            );
        }
        last_matcher_evaluations = matcher_evaluations;
        last_matches = matches;
        last_attempts = attempts;
        last_applied = applied;
        let refused = status_counter(daemon.as_ref(), "output_refused_total");
        let partial = status_counter(daemon.as_ref(), "partial_write_total");
        if refused > last_refused || partial > last_partial {
            emit(
                json,
                started,
                "output_health",
                &format!(
                    "{} output refusal(s), {} partial write(s); no automatic retry",
                    refused.saturating_sub(last_refused),
                    partial.saturating_sub(last_partial),
                ),
            );
        }
        last_refused = refused;
        last_partial = partial;
        if ibus_transaction.is_some() && ibus_transaction != last_ibus_transaction {
            if let Some(id) = &ibus_transaction {
                emit(json, started, "output_transaction", &format!("IBus transaction {id} emitted; target application confirmation is separate"));
            }
            last_ibus_transaction = ibus_transaction;
        }
        if let Some(outcomes) = daemon
            .as_ref()
            .and_then(|s| s.get("recent_expansion_outcomes"))
            .and_then(|v| v.as_array())
        {
            for outcome in outcomes {
                let Some(id) = outcome
                    .get("transaction_id")
                    .and_then(serde_json::Value::as_str)
                else {
                    continue;
                };
                if seen.iter().any(|known| known == id) {
                    continue;
                }
                let completion = outcome
                    .get("completion")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                emit(
                    json,
                    started,
                    "output_transaction",
                    &format!("transaction {id} completed with state {completion}; no text logged"),
                );
                seen.push_back(id.to_owned());
                if seen.len() > 32 {
                    seen.pop_front();
                }
            }
        }
    }
    if json {
        println!(
            "{}",
            serde_json::json!({"type":"live_end","duration_seconds":seconds,"expired":true,"raw_text_logged":false})
        );
    } else {
        println!("Live debugging expired. No keystroke, trigger, replacement, or clipboard text was recorded.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::diagnostic_test_id;

    #[test]
    fn desktop_test_ids_are_unique_and_do_not_encode_input_content() {
        let first = diagnostic_test_id();
        let second = diagnostic_test_id();
        assert!(first.starts_with("desktop-"));
        assert_ne!(first, second);
        assert!(!first.contains("trigger"));
    }
}

#[cfg(test)]
mod status_counter_tests {
    use super::status_counter;
    use serde_json::json;

    #[test]
    fn reads_numeric_and_decimal_string_status_counters() {
        let status = json!({"numeric": 12, "string": "34"});
        assert_eq!(status_counter(Some(&status), "numeric"), 12);
        assert_eq!(status_counter(Some(&status), "string"), 34);
    }

    #[test]
    fn invalid_or_signed_status_counters_fail_closed_to_zero() {
        let status = json!({"negative": "-1", "invalid": "not-a-counter"});
        assert_eq!(status_counter(Some(&status), "negative"), 0);
        assert_eq!(status_counter(Some(&status), "invalid"), 0);
        assert_eq!(status_counter(None, "missing"), 0);
    }
}
