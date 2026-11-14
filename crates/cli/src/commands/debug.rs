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

#[derive(Debug, Default, PartialEq, Eq)]
struct FocusedAppScopeStatus {
    daemon_status_schema: Option<u64>,
    daemon_schema_compatible: Option<bool>,
    scope_state: Option<String>,
    terminal_apps_only: Option<bool>,
    window_tracker_connected: Option<bool>,
    window_context_available: Option<bool>,
    window_app_id_available: Option<bool>,
    window_process_executable_available: Option<bool>,
    window_process_name_available: Option<bool>,
    window_tracker_identity_source: Option<String>,
    window_process_identity_state: Option<String>,
}

fn focused_app_scope_from_status(status_response: Option<&str>) -> FocusedAppScopeStatus {
    let Some(status_response) = status_response else {
        return FocusedAppScopeStatus::default();
    };
    let status = wayexpand_core::DaemonStatus::parse(status_response);
    let daemon_status_schema = status
        .field("status_schema")
        .and_then(|schema| schema.parse::<u64>().ok());
    FocusedAppScopeStatus {
        daemon_status_schema,
        daemon_schema_compatible: daemon_status_schema
            .map(|schema| schema == u64::from(crate::CONTROL_STATUS_SCHEMA)),
        scope_state: status.field("capture_app_scope_state").map(str::to_owned),
        terminal_apps_only: status.bool_field("terminal_apps_only"),
        window_tracker_connected: status.bool_field("window_tracker_connected"),
        window_context_available: status.bool_field("window_context_available"),
        window_app_id_available: status.bool_field("window_app_id_available"),
        window_process_executable_available: status
            .bool_field("window_process_executable_available"),
        window_process_name_available: status.bool_field("window_process_name_available"),
        window_tracker_identity_source: status
            .field("window_tracker_identity_source")
            .map(str::to_owned),
        window_process_identity_state: status
            .field("window_process_identity_state")
            .map(str::to_owned),
    }
}

fn focused_app_matching_status(
    app_id: Option<&str>,
    scope_state: Option<&str>,
    terminal_apps_only: Option<bool>,
    identity_source: Option<&str>,
    daemon_schema_compatible: Option<bool>,
) -> &'static str {
    if daemon_schema_compatible == Some(false) {
        return "daemon_status_incompatible";
    }
    match terminal_apps_only {
        Some(false) => "unrestricted",
        Some(true)
            if scope_state == Some("allowed") && identity_source == Some("process-executable") =>
        {
            "allowed_by_executable"
        }
        Some(true) if scope_state == Some("allowed") && identity_source == Some("process-name") => {
            "allowed_by_process_name"
        }
        Some(true) if scope_state == Some("allowed") => "allowed",
        Some(true) if scope_state == Some("outside_allowlist") => "suppressed_outside_allowlist",
        Some(true) if app_id.is_none() || app_id == Some("unavailable") => {
            "suppressed_unknown_focus"
        }
        Some(true) => "scope_unverified",
        None => "unknown",
    }
}

fn process_executable_from_app_identity(app_id: &str) -> Option<&str> {
    app_id.strip_prefix(wayexpand_core::PROCESS_EXECUTABLE_APP_ID_PREFIX)
}

fn process_name_from_app_identity(app_id: &str) -> Option<&str> {
    app_id.strip_prefix(wayexpand_core::PROCESS_NAME_APP_ID_PREFIX)
}

fn focused_app_diagnostic(
    matching_status: &str,
    focus: &FocusedAppScopeStatus,
) -> Option<(&'static str, &'static str)> {
    match matching_status {
        "daemon_status_incompatible" => Some((
            "WX-VERSION-001",
            "The running daemon reports a different status protocol schema, so this focus diagnosis may be incomplete. Run `wayexpand upgrade-status` and `wayexpand upgrade --check`; restart or upgrade the daemon before using this result to change application allowlists.",
        )),
        "suppressed_unknown_focus"
            if focus.window_tracker_connected == Some(true)
                && focus.window_context_available == Some(true)
                && focus.window_app_id_available == Some(false) =>
        {
            if focus.window_process_identity_state.as_deref() == Some("compositor-process-id") {
                return Some((
                    "WX-EVDEV-FOCUS-006",
                    "KWin reported its own process ID for the focused compositor surface, not an application process. Focus the actual terminal or browser window and rerun this diagnostic; do not allowlist `kwin_wayland` or use a window title as a substitute for client identity.",
                ));
            }
            if focus.window_tracker_identity_source.as_deref() == Some("process-name") {
                return Some((
                    "WX-EVDEV-FOCUS-004",
                    "KWin could not provide a verified executable identity and supplied only a mutable process name. If you accept this lower assurance, add an exact `process_name_exact:<that-name>` filter to the terminal and expansion scopes; do not use a wildcard or window title. The `window_process_identity_state` field gives the content-free executable lookup failure category.",
                ));
            }
            if focus.window_process_identity_state.as_deref()
                == Some("process-id-unavailable")
            {
                return Some((
                    "WX-EVDEV-FOCUS-005",
                    "KWin did not provide a process ID for the focused window, so WayExpand cannot resolve an executable or process name. Keep terminal-scoped matching fail-closed; use a compositor route that supplies application identity or manual paste.",
                ));
            }
            if focus.window_tracker_identity_source.as_deref()
                == Some("process-executable-permission-denied")
            {
                return Some((
                    "WX-EVDEV-FOCUS-003",
                    "KWin supplied a focused-window PID, but WayExpand could not verify its executable through `/proc`. An exact-path filter cannot authorize this window until the host allows same-user executable inspection. Do not weaken process permissions or replace this with a title/wildcard filter; use an application-ID-capable route or manual paste for this window.",
                ));
            }
            let action = if focus.window_tracker_identity_source.as_deref()
                == Some("compositor-identity-only")
            {
                "KWin supplied only compositor-owned identity metadata for this window, so an app-ID allowlist cannot authorize it. For a terminal you explicitly trust, use `process_executable_exact:/absolute/path` in `settings.terminal_apps` and, when applicable, `settings.raw_capture_apps` and the expansion's `app_filter`. KWin resolves executable identity only for exact-path filters. Keep the scope enabled; do not substitute a title or wildcard."
            } else {
                "The window tracker reports a focused window but supplies no stable application ID, so terminal-only matching is suppressed. If the tracker cannot provide an app ID, configure an explicit `process_executable_exact:/absolute/path` filter for the intended terminal in `settings.terminal_apps` and, when applicable, `settings.raw_capture_apps` and the expansion's `app_filter`. Check `wayexpand doctor --json`; do not substitute the window title or an unknown-app wildcard."
            };
            Some(("WX-EVDEV-FOCUS-001", action))
        }
        "suppressed_outside_allowlist"
            if focus.window_tracker_identity_source.as_deref() == Some("process-executable") =>
        {
            Some((
                "WX-EVDEV-FOCUS-002",
                "KWin identified the focused executable in `focused_app_id` as an exact path, but it is outside the configured allowlist. If this terminal is intended, add `process_executable_exact:<that-path>` to `settings.terminal_apps` and any affected expansion `app_filter`; keep the filter exact rather than widening it.",
            ))
        }
        "suppressed_outside_allowlist"
            if focus.window_tracker_identity_source.as_deref() == Some("process-name") =>
        {
            Some((
                "WX-EVDEV-FOCUS-004",
                "KWin identified this window using its mutable Linux process name, which is weaker than an executable path. If you accept that limitation, add the exact `process_name_exact:<that-name>` filter to `settings.terminal_apps` and any affected expansion `app_filter`; process names can be changed by the application and are not a strong security identity.",
            ))
        }
        "suppressed_unknown_focus" | "scope_unverified" => Some((
            "WX-EVDEV-FOCUS-001",
            "Run `wayexpand debug --live --seconds 60` while testing a harmless trigger in a configured application, then inspect sanitized app_scope events. Keep app-scoped capture enabled.",
        )),
        _ => None,
    }
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
        let focused_process_executable = process_executable_from_app_identity(app_id);
        let focused_process_name = process_name_from_app_identity(app_id);
        let focused_application_id = (app_id != "unavailable"
            && focused_process_executable.is_none()
            && focused_process_name.is_none())
        .then_some(app_id);
        // Read the scope decision beside the identity. This turns the command
        // into a useful diagnosis for terminal-scoped evdev routes instead of
        // making users compare the ID against configuration by hand. Older
        // daemons may not publish these fields, so keep them optional.
        let status_response =
            crate::control_operation(wayexpand_core::DaemonOperation::Status).ok();
        let focus = focused_app_scope_from_status(status_response.as_deref());
        let matching_status = focused_app_matching_status(
            (app_id != "unavailable").then_some(app_id),
            focus.scope_state.as_deref(),
            focus.terminal_apps_only,
            focus.window_tracker_identity_source.as_deref(),
            focus.daemon_schema_compatible,
        );
        let diagnostic = focused_app_diagnostic(matching_status, &focus);
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "focused_app_id": focused_application_id,
                    "focused_process_executable": focused_process_executable,
                    "focused_process_name": focused_process_name,
                    "daemon_status_schema": focus.daemon_status_schema,
                    "daemon_schema_compatible": focus.daemon_schema_compatible,
                    "capture_app_scope_state": focus.scope_state,
                    "terminal_apps_only": focus.terminal_apps_only,
                    "window_tracker_connected": focus.window_tracker_connected,
                    "window_context_available": focus.window_context_available,
                    "window_app_id_available": focus.window_app_id_available,
                    "window_process_executable_available": focus.window_process_executable_available,
                    "window_process_name_available": focus.window_process_name_available,
                    "window_tracker_identity_source": focus.window_tracker_identity_source,
                    "window_process_identity_state": focus.window_process_identity_state,
                    "matching_status": matching_status,
                    "problem_id": diagnostic.map(|(id, _)| id),
                    "suggested_action": diagnostic.map(|(_, action)| action),
                })
            );
        } else if matching_status == "daemon_status_incompatible" {
            println!("The running daemon uses an incompatible status schema.");
            println!(
                "Reported schema: {}",
                focus
                    .daemon_status_schema
                    .map_or_else(|| "unknown".into(), |schema| schema.to_string())
            );
            if let Some((problem_id, suggested_action)) = diagnostic {
                println!("Problem: {problem_id}");
                println!("Next: {suggested_action}");
            }
        } else if let Some(path) = focused_process_executable {
            println!("Focused application ID: unavailable");
            println!("Focused executable: {path}");
            if let Some(scope_state) = focus.scope_state.as_deref() {
                println!("Capture app scope: {scope_state}");
            }
            if matching_status == "suppressed_outside_allowlist" {
                println!("Expansion matching is suppressed for this focused application.");
                if let Some((problem_id, suggested_action)) = diagnostic {
                    println!("Problem: {problem_id}");
                    println!("Next: {suggested_action}");
                }
            }
        } else if let Some(name) = focused_process_name {
            println!("Focused application ID: unavailable");
            println!("Focused process name (lower assurance): {name}");
            if let Some(scope_state) = focus.scope_state.as_deref() {
                println!("Capture app scope: {scope_state}");
            }
            if let Some((problem_id, suggested_action)) = diagnostic {
                println!("Problem: {problem_id}");
                println!("Next: {suggested_action}");
            }
        } else if app_id == "unavailable" {
            println!("Focused application ID: unavailable");
            if let Some(scope_state) = focus.scope_state.as_deref() {
                println!("Capture app scope: {scope_state}");
            }
            if focus.window_tracker_connected == Some(true)
                && focus.window_context_available == Some(true)
                && focus.window_app_id_available == Some(false)
            {
                if focus.window_process_executable_available == Some(true) {
                    println!("KWin supplied an exact process executable identity; no desktop application ID is available.");
                } else {
                    println!("The focus tracker sees a window, but KWin supplied no stable application ID.");
                }
            }
            if matching_status == "suppressed_unknown_focus" {
                println!("Terminal-scoped matching is suppressed because the focused application could not be identified.");
                if let Some((problem_id, suggested_action)) = diagnostic {
                    println!("Problem: {problem_id}");
                    println!("Next: {suggested_action}");
                }
            }
        } else {
            println!("Focused application ID: {app_id}");
            if let Some(scope_state) = focus.scope_state.as_deref() {
                println!("Capture app scope: {scope_state}");
                if focus.terminal_apps_only == Some(true) && scope_state == "outside_allowlist" {
                    println!("Expansion matching is suppressed for this focused application.");
                }
            }
            if matching_status == "scope_unverified" {
                println!("Terminal-scoped matching cannot be verified from the daemon's current app-scope status.");
                if let Some((problem_id, suggested_action)) = diagnostic {
                    println!("Problem: {problem_id}");
                    println!("Next: {suggested_action}");
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
    use super::{
        focused_app_diagnostic, focused_app_matching_status, focused_app_scope_from_status,
        FocusedAppScopeStatus,
    };

    #[test]
    fn reports_terminal_scope_and_fail_closed_allowlist_mismatch() {
        assert_eq!(
            focused_app_scope_from_status(Some(
                "running\nterminal_apps_only=true\ncapture_app_scope_state=outside_allowlist\n",
            )),
            FocusedAppScopeStatus {
                scope_state: Some("outside_allowlist".into()),
                terminal_apps_only: Some(true),
                ..FocusedAppScopeStatus::default()
            }
        );
    }

    #[test]
    fn old_or_ambiguous_status_keeps_scope_unavailable() {
        assert_eq!(
            focused_app_scope_from_status(None),
            FocusedAppScopeStatus::default()
        );
        assert_eq!(
            focused_app_scope_from_status(Some(
                "running\nterminal_apps_only=true\ncapture_app_scope_state=allowed\ncapture_app_scope_state=outside_allowlist\n",
            )),
            FocusedAppScopeStatus {
                terminal_apps_only: Some(true),
                ..FocusedAppScopeStatus::default()
            }
        );
    }

    #[test]
    fn focused_app_matching_diagnosis_distinguishes_unknown_and_blocked_focus() {
        assert_eq!(
            focused_app_matching_status(None, Some("unknown"), Some(true), None, None),
            "suppressed_unknown_focus"
        );
        assert_eq!(
            focused_app_matching_status(
                Some("org.gnome.TextEditor"),
                Some("outside_allowlist"),
                Some(true),
                None,
                None,
            ),
            "suppressed_outside_allowlist"
        );
        assert_eq!(
            focused_app_matching_status(
                Some("org.kde.konsole"),
                Some("allowed"),
                Some(true),
                Some("desktop-file-name"),
                None,
            ),
            "allowed"
        );
        assert_eq!(
            focused_app_matching_status(
                Some("org.kde.konsole"),
                Some("unknown"),
                Some(true),
                None,
                None,
            ),
            "scope_unverified"
        );
        assert_eq!(
            focused_app_matching_status(None, None, Some(false), None, None),
            "unrestricted"
        );
        assert_eq!(
            focused_app_matching_status(
                None,
                Some("allowed"),
                Some(true),
                Some("process-executable"),
                None,
            ),
            "allowed_by_executable"
        );
        assert_eq!(
            focused_app_matching_status(
                Some("process-name:konsole"),
                Some("allowed"),
                Some(true),
                Some("process-name"),
                None,
            ),
            "allowed_by_process_name"
        );
    }

    #[test]
    fn unknown_focus_has_stable_problem_id_and_safe_remediation() {
        let (problem_id, suggested_action) = focused_app_diagnostic(
            "suppressed_unknown_focus",
            &FocusedAppScopeStatus::default(),
        )
        .expect("unknown focus needs an actionable diagnostic");
        assert_eq!(problem_id, "WX-EVDEV-FOCUS-001");
        assert!(suggested_action.contains("wayexpand debug --live"));
        assert!(suggested_action.contains("Keep app-scoped capture enabled"));
        assert_eq!(
            focused_app_diagnostic("allowed", &FocusedAppScopeStatus::default()),
            None
        );
        assert_eq!(
            focused_app_diagnostic(
                "suppressed_outside_allowlist",
                &FocusedAppScopeStatus::default()
            ),
            None
        );
    }

    #[test]
    fn reports_stale_daemon_schema_before_diagnosing_application_scope() {
        let current = crate::CONTROL_STATUS_SCHEMA;
        let focus = focused_app_scope_from_status(Some(&format!(
            "running\nstatus_schema={}\nterminal_apps_only=true\ncapture_app_scope_state=unknown\n",
            current.saturating_sub(1)
        )));
        assert_eq!(focus.daemon_schema_compatible, Some(false));
        assert_eq!(
            focused_app_matching_status(
                None,
                focus.scope_state.as_deref(),
                focus.terminal_apps_only,
                None,
                focus.daemon_schema_compatible,
            ),
            "daemon_status_incompatible"
        );
        let (problem_id, suggested_action) =
            focused_app_diagnostic("daemon_status_incompatible", &focus)
                .expect("incompatible schema needs a specific remediation");
        assert_eq!(problem_id, "WX-VERSION-001");
        assert!(suggested_action.contains("wayexpand upgrade-status"));
        assert!(
            suggested_action.contains("before using this result to change application allowlists")
        );

        let compatible = focused_app_scope_from_status(Some(&format!(
            "running\nstatus_schema={current}\nterminal_apps_only=true\ncapture_app_scope_state=allowed\n"
        )));
        assert_eq!(compatible.daemon_schema_compatible, Some(true));
    }

    #[test]
    fn missing_window_application_id_has_specific_safe_guidance() {
        let focus = focused_app_scope_from_status(Some(
            "running\nterminal_apps_only=true\ncapture_app_scope_state=unknown\nwindow_tracker_connected=true\nwindow_context_available=true\nwindow_app_id_available=false\n",
        ));
        let (problem_id, suggested_action) =
            focused_app_diagnostic("suppressed_unknown_focus", &focus)
                .expect("missing app ID needs a specific diagnosis");
        assert_eq!(problem_id, "WX-EVDEV-FOCUS-001");
        assert!(suggested_action.contains("window tracker reports a focused window"));
        assert!(!suggested_action.contains("KWin reported"));
        assert!(suggested_action.contains("do not substitute the window title"));
    }

    #[test]
    fn compositor_only_identity_source_is_explained_without_exposing_identity() {
        let focus = focused_app_scope_from_status(Some(
            "running\nterminal_apps_only=true\ncapture_app_scope_state=unknown\nwindow_tracker_connected=true\nwindow_context_available=true\nwindow_app_id_available=false\nwindow_tracker_identity_source=compositor-identity-only\n",
        ));
        let (_, suggested_action) =
            focused_app_diagnostic("suppressed_unknown_focus", &focus).unwrap();
        assert!(suggested_action.contains("compositor-owned identity metadata"));
        assert!(!suggested_action.contains("window title:"));
    }

    #[test]
    fn process_identity_outside_allowlist_suggests_the_exact_path_filter() {
        let focus = focused_app_scope_from_status(Some(
            "running\nterminal_apps_only=true\ncapture_app_scope_state=outside_allowlist\nwindow_tracker_connected=true\nwindow_context_available=true\nwindow_app_id_available=false\nwindow_process_executable_available=true\nwindow_tracker_identity_source=process-executable\n",
        ));
        let (problem_id, suggested_action) =
            focused_app_diagnostic("suppressed_outside_allowlist", &focus)
                .expect("an identified but unlisted terminal needs remediation");
        assert_eq!(problem_id, "WX-EVDEV-FOCUS-002");
        assert!(suggested_action.contains("focused_app_id"));
        assert!(suggested_action.contains("process_executable_exact:<that-path>"));
        assert!(suggested_action.contains("keep the filter exact"));
        assert!(!suggested_action.contains("wildcard"));
    }

    #[test]
    fn process_name_outside_allowlist_warns_about_mutable_identity() {
        let focus = focused_app_scope_from_status(Some(
            "running\nterminal_apps_only=true\ncapture_app_scope_state=outside_allowlist\nwindow_tracker_connected=true\nwindow_context_available=true\nwindow_app_id_available=false\nwindow_process_name_available=true\nwindow_tracker_identity_source=process-name\n",
        ));
        let (problem_id, suggested_action) =
            focused_app_diagnostic("suppressed_outside_allowlist", &focus).unwrap();
        assert_eq!(problem_id, "WX-EVDEV-FOCUS-004");
        assert!(suggested_action.contains("process_name_exact:<that-name>"));
        assert!(suggested_action.contains("mutable"));
        assert!(suggested_action.contains("weaker than an executable path"));
    }

    #[test]
    fn failed_process_identity_lookup_does_not_suggest_an_exact_path_filter() {
        let focus = focused_app_scope_from_status(Some(
            "running\nterminal_apps_only=true\ncapture_app_scope_state=unknown\nwindow_tracker_connected=true\nwindow_context_available=true\nwindow_app_id_available=false\nwindow_process_executable_available=false\nwindow_tracker_identity_source=process-executable-permission-denied\nwindow_process_identity_state=executable-permission-denied\n",
        ));
        let (problem_id, suggested_action) =
            focused_app_diagnostic("suppressed_unknown_focus", &focus)
                .expect("failed process lookup needs a distinct diagnosis");
        assert_eq!(problem_id, "WX-EVDEV-FOCUS-003");
        assert!(suggested_action.contains("could not verify its executable"));
        assert!(!suggested_action.contains("process_executable_exact:"));
        assert!(suggested_action.contains("Do not weaken process permissions"));
    }

    #[test]
    fn unknown_focus_diagnoses_process_id_absence_and_mutable_name_fallback() {
        let name_fallback = focused_app_scope_from_status(Some(
            "running\nterminal_apps_only=true\ncapture_app_scope_state=unknown\nwindow_tracker_connected=true\nwindow_context_available=true\nwindow_app_id_available=false\nwindow_process_name_available=true\nwindow_tracker_identity_source=process-name\nwindow_process_identity_state=executable-permission-denied\n",
        ));
        let (problem_id, action) =
            focused_app_diagnostic("suppressed_unknown_focus", &name_fallback).unwrap();
        assert_eq!(problem_id, "WX-EVDEV-FOCUS-004");
        assert!(action.contains("mutable process name"));
        assert!(action.contains("window_process_identity_state"));

        let no_pid = focused_app_scope_from_status(Some(
            "running\nterminal_apps_only=true\ncapture_app_scope_state=unknown\nwindow_tracker_connected=true\nwindow_context_available=true\nwindow_app_id_available=false\nwindow_process_identity_state=process-id-unavailable\n",
        ));
        let (problem_id, action) =
            focused_app_diagnostic("suppressed_unknown_focus", &no_pid).unwrap();
        assert_eq!(problem_id, "WX-EVDEV-FOCUS-005");
        assert!(action.contains("did not provide a process ID"));

        let compositor_pid = focused_app_scope_from_status(Some(
            "running\nterminal_apps_only=true\ncapture_app_scope_state=unknown\nwindow_tracker_connected=true\nwindow_context_available=true\nwindow_app_id_available=false\nwindow_process_identity_state=compositor-process-id\n",
        ));
        let (problem_id, action) =
            focused_app_diagnostic("suppressed_unknown_focus", &compositor_pid)
                .expect("a compositor PID must not be presented as an application identity");
        assert_eq!(problem_id, "WX-EVDEV-FOCUS-006");
        assert!(action.contains("its own process ID"));
        assert!(!action.contains("process_name_exact:"));
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

fn status_counter_compat(status: Option<&serde_json::Value>, current: &str, legacy: &str) -> u64 {
    let current_value = status_counter(status, current);
    if current_value != 0 {
        current_value
    } else {
        status_counter(status, legacy)
    }
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
    let mut last_acknowledged = status_counter_compat(
        initial_daemon.as_ref(),
        "expansion_output_acknowledgements_total",
        "expansions_applied_total",
    );
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
                    "{} IBus output(s) emitted; target text unverified, content omitted",
                    expansions - last_expansions
                ),
            );
            last_expansions = expansions;
        }
        let matcher_evaluations = status_counter(daemon.as_ref(), "matcher_evaluations_total");
        let matches = status_counter(daemon.as_ref(), "matches_found_total");
        let attempts = status_counter(daemon.as_ref(), "expansion_attempts_total");
        let acknowledged = status_counter_compat(
            daemon.as_ref(),
            "expansion_output_acknowledgements_total",
            "expansions_applied_total",
        );
        if matcher_evaluations > last_matcher_evaluations
            || matches > last_matches
            || attempts > last_attempts
            || acknowledged > last_acknowledged
        {
            emit(
                json,
                started,
                "matcher",
                &format!(
                    "{} evaluation(s), {} match(es), {} insertion attempt(s), {} output acknowledgement(s); target text unverified, content omitted",
                    matcher_evaluations.saturating_sub(last_matcher_evaluations),
                    matches.saturating_sub(last_matches),
                    attempts.saturating_sub(last_attempts),
                    acknowledged.saturating_sub(last_acknowledged),
                ),
            );
        }
        last_matcher_evaluations = matcher_evaluations;
        last_matches = matches;
        last_attempts = attempts;
        last_acknowledged = acknowledged;
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
                let application_result = outcome
                    .get("application_result")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                emit(
                    json,
                    started,
                    "output_transaction",
                    &format!(
                        "transaction {id}: backend completion {completion}; application result {application_result}; no text logged"
                    ),
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
    use super::{status_counter, status_counter_compat};
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

    #[test]
    fn output_acknowledgement_counter_prefers_new_name_and_reads_legacy_daemons() {
        let current = serde_json::json!({
            "expansion_output_acknowledgements_total": 8,
            "expansions_applied_total": 8
        });
        assert_eq!(
            status_counter_compat(
                Some(&current),
                "expansion_output_acknowledgements_total",
                "expansions_applied_total"
            ),
            8
        );

        let legacy = serde_json::json!({"expansions_applied_total": 3});
        assert_eq!(
            status_counter_compat(
                Some(&legacy),
                "expansion_output_acknowledgements_total",
                "expansions_applied_total"
            ),
            3
        );
    }
}
