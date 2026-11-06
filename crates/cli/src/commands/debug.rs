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

pub(crate) fn debug_command(args: Args) -> Result<()> {
    let mut live = false;
    let mut json = false;
    let mut seconds = 60_u64;
    let mut trigger: Option<String> = None;
    let mut test_trigger: Option<String> = None;
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
            _ => {
                index += 1;
            }
        }
    }
    if !values.is_empty() {
        bail!("usage: wayexpand debug [--json] [--trigger TEXT] [--live [--seconds 1..300]] | --test TRIGGER");
    }
    if test_trigger.is_some() && (live || json || trigger.is_some()) {
        bail!("--test cannot be combined with --json, --trigger, or --live");
    }
    if let Some(trigger) = test_trigger {
        return desktop_test(&trigger);
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

/// Interactive, target-application test. It never sends Enter, and only
/// accepts a static, enabled snippet without command execution or form fields.
pub(crate) fn desktop_test(trigger: &str) -> Result<()> {
    if trigger.is_empty() || trigger.chars().any(char::is_control) {
        bail!("test trigger must be non-empty and contain no control characters");
    }
    let config = wayexpand_core::Config::load(wayexpand_core::default_config_path())
        .map_err(|error| anyhow::anyhow!("configuration check failed: {}", error.safe_summary()))?;
    let Some(expansion) = config
        .expansion
        .iter()
        .find(|item| item.enabled && item.answers_to(trigger))
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
    print!("Target application class (GTK/Qt/Electron/terminal/browser/remote/sandboxed/other): ");
    io::stdout().flush()?;
    let mut application_class = String::new();
    io::stdin().read_line(&mut application_class)?;
    let application_class = application_class.trim();
    if application_class.is_empty() || application_class.chars().any(char::is_control) {
        bail!("enter a non-empty application class without control characters");
    }
    print!("Focus a disposable text field, then continue? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Test cancelled; no text was inserted.");
        return Ok(());
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
        return Ok(());
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
        return Ok(());
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
        println!(
            "Test scope: {}; result requires manual review.",
            application_class
        );
        return Ok(());
    }
    println!("Check the target application, then return here. Was the expected text inserted exactly once, without loss or interleaving? [y/N] ");
    answer.clear();
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
    emit(
        json,
        started,
        "session",
        &format!("initial state {last_state}"),
    );
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
