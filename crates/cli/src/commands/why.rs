//! `wayexpand why`: the most likely reason the last trigger did not expand.
//!
//! Combines the daemon's content-free status (`ExpansionDiagnosis`) with the
//! IBus engine's per-field telemetry. Neither records typed text, triggers
//! or replacements, so the output is safe to paste into a bug report.

use std::time::{SystemTime, UNIX_EPOCH};

use wayexpand_backend_ibus::{
    telemetry::{self, IbusFieldActivity},
    IbusFieldVerdict,
};
use wayexpand_core::{
    DaemonStatus, ExpansionDiagnosis, MatchingBlock, OutcomeKind, RecentOutcome, Verdict,
};

use super::Args;
use crate::errors::{classified_error, CliErrorKind};
use crate::*;

const USAGE: &str = "usage: wayexpand why [--json]";

/// The keyboard-route half: a capturing daemon and what its status says.
struct DaemonRoute {
    source: String,
    backend: String,
    diagnosis: ExpansionDiagnosis,
}

pub(crate) fn why_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let json = take_json_flag(&mut rest);
    if !rest.is_empty() {
        usage_bail!("{USAGE}");
    }
    let status = control_operation(wayexpand_core::DaemonOperation::Status)
        .ok()
        .map(|body| DaemonStatus::parse(&body));
    // The control-only service (stdin source) captures no typing.
    let daemon = status
        .as_ref()
        .filter(|status| {
            status
                .field("source")
                .is_some_and(|source| source != "stdin")
        })
        .map(|status| DaemonRoute {
            source: status.field("source").unwrap_or_default().to_owned(),
            backend: status.field("backend").unwrap_or_default().to_owned(),
            diagnosis: ExpansionDiagnosis::from_status(status),
        });
    let ibus_field = telemetry::read_status()
        .filter(|reading| reading.live)
        .map(|reading| newest_typed_field(&reading.status.recent_fields).cloned());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report_json(daemon.as_ref(), ibus_field.as_ref(), now))?
        );
    } else {
        print_report(daemon.as_ref(), ibus_field.as_ref(), now);
    }
    if daemon.is_none() && ibus_field.is_none() {
        return Err(classified_error(
            CliErrorKind::Daemon,
            "no WayExpand typing route is running",
        ));
    }
    Ok(())
}

/// The newest field the user typed in: the place the trigger most likely
/// went. Fields only focused, with no keys, say nothing about it.
fn newest_typed_field(fields: &[IbusFieldActivity]) -> Option<&IbusFieldActivity> {
    fields.iter().rev().find(|field| field.keys > 0)
}

fn report_json(
    daemon: Option<&DaemonRoute>,
    ibus_field: Option<&Option<IbusFieldActivity>>,
    now: u64,
) -> serde_json::Value {
    let daemon = daemon.map(|route| {
        let diagnosis = &route.diagnosis;
        let verdict = diagnosis.verdict(now);
        serde_json::json!({
            "source": route.source,
            "backend": route.backend,
            "verdict": verdict.code(),
            "matching_block": diagnosis.matching_block.map(MatchingBlock::code),
            "last_outcome": match verdict {
                Verdict::LastStopped(kind) => Some(kind.code()),
                Verdict::LastExpanded => diagnosis.recent.first().map(|outcome| outcome.kind.code()),
                _ => None,
            },
            "terminal_apps_only": diagnosis.terminal_apps_only,
            "input_events_total": diagnosis.input_events,
            "matches_found_total": diagnosis.matches_found,
            "stopped_total": diagnosis.stopped_total,
            "recent": diagnosis.recent.iter().map(|outcome| serde_json::json!({
                "at_unix": outcome.at_unix,
                "outcome": outcome.kind.code(),
                "completion": outcome.completion,
            })).collect::<Vec<_>>(),
        })
    });
    let ibus = ibus_field.map(|field| {
        let field = field.as_ref();
        serde_json::json!({
            "verdict": field.map(|field| IbusFieldVerdict::of(field).label()),
            "keys": field.map(|field| field.keys),
            "expansions": field.map(|field| field.expansions),
        })
    });
    serde_json::json!({ "daemon": daemon, "ibus": ibus })
}

fn print_report(
    daemon: Option<&DaemonRoute>,
    ibus_field: Option<&Option<IbusFieldActivity>>,
    now: u64,
) {
    if daemon.is_none() && ibus_field.is_none() {
        println!("No WayExpand typing route is running, so nothing can expand.");
        println!("Fix: run `wayexpand setup`, then `wayexpand doctor`.");
        return;
    }
    if let Some(field) = ibus_field {
        println!("Apps through IBus");
        match field {
            Some(field) => {
                let verdict = IbusFieldVerdict::of(field);
                println!(
                    "  The last field you typed in: {} key(s), {} expansion(s).",
                    field.keys, field.expansions
                );
                println!("  {}", sentence(verdict.explanation()));
            }
            None => {
                println!("  No typing has reached the IBus engine yet in this login session.");
                println!(
                    "  Check that WayExpand is the active input method: `wayexpand ibus status`."
                );
            }
        }
        println!();
    }
    if let Some(route) = daemon {
        let diagnosis = &route.diagnosis;
        print!("Keyboard route ({} → {}", route.source, route.backend);
        if diagnosis.terminal_apps_only {
            print!(", terminal apps only");
        }
        println!(")");
        let (answer, fix) = verdict_text(diagnosis, now);
        println!("  {answer}");
        if let Some(fix) = fix {
            println!("  Fix: {fix}");
        }
        if !diagnosis.recent.is_empty() {
            println!("  Recent expansions (newest first; no text is recorded):");
            for outcome in &diagnosis.recent {
                println!(
                    "    {:>8}  {}",
                    ago(now, outcome.at_unix),
                    outcome_line(outcome)
                );
            }
        }
        println!();
    }
    println!("To check one trigger: wayexpand explain 'the text you typed'");
}

/// The answer for the keyboard route and, when there is one, what to do.
fn verdict_text(diagnosis: &ExpansionDiagnosis, now: u64) -> (String, Option<String>) {
    use MatchingBlock as B;
    match diagnosis.verdict(now) {
        Verdict::MatchingOff(block) => {
            let (answer, fix) = match block {
                B::CaptureDisconnected => (
                    "Keyboard capture is not connected, so nothing you type reaches WayExpand.",
                    "`wayexpand doctor`",
                ),
                B::KeyboardLayoutUnavailable => (
                    "WayExpand cannot read your keyboard layout, so it cannot tell which characters you typed.",
                    "`wayexpand doctor`",
                ),
                B::OutputUnavailable => (
                    "No output route is connected, so WayExpand cannot replace a trigger.",
                    "`wayexpand doctor`",
                ),
                B::PolicyRoute => (
                    "Your organization's policy does not allow this typing route.",
                    "`wayexpand doctor --security`",
                ),
                B::Paused => ("WayExpand is paused.", "`wayexpand resume`"),
                B::ConfigurationUnhealthy => (
                    "The service could not load the configuration, so it has no usable snippets.",
                    "`wayexpand validate`, then `wayexpand reload`",
                ),
                B::ReplacementNotAllowed => (
                    "This route types keystrokes and has not been allowed to replace typed text.",
                    "allow it with `compatibility_mode` (see `wayexpand doctor`), or run `wayexpand setup`",
                ),
                B::Other => (
                    "Matching is off for a reason this version of the CLI does not know.",
                    "`wayexpand doctor`",
                ),
            };
            (answer.to_owned(), Some(fix.to_owned()))
        }
        Verdict::NoTypingSeen => (
            if diagnosis.terminal_apps_only {
                "No typing has reached this route since it started. It only handles your configured terminal apps."
            } else {
                "No typing has reached this route since it started."
            }
            .to_owned(),
            Some("type the trigger again while `wayexpand debug --live` runs".to_owned()),
        ),
        Verdict::NothingMatched => (
            format!(
                "Typing reaches this route, but nothing has expanded in the last two minutes. Usually the text did not match: a typo, a word-boundary trigger typed right after a letter, or an app filter that leaves this app out{}.",
                if diagnosis.terminal_apps_only {
                    ". This route also skips every app that is not one of your terminal apps"
                } else {
                    ""
                }
            ),
            Some("`wayexpand explain 'the text you typed'` checks it".to_owned()),
        ),
        Verdict::LastStopped(kind) => {
            let completion = diagnosis
                .recent
                .first()
                .map(|outcome| outcome.completion.as_str())
                .unwrap_or_default();
            stopped_text(kind, completion)
        }
        Verdict::LastExpanded => (
            "The route accepted the last expansion. If the text did not appear, the app ignored or changed it; WayExpand cannot see the app's text."
                .to_owned(),
            None,
        ),
    }
}

fn stopped_text(kind: OutcomeKind, completion: &str) -> (String, Option<String>) {
    use OutcomeKind as K;
    let (answer, fix): (String, Option<&str>) = match kind {
        K::NotApplied => (
            "The route refused the last expansion before changing anything.".into(),
            Some("`wayexpand doctor`"),
        ),
        K::Unknown => (
            "The route could not tell whether the app received the last expansion.".into(),
            Some("check the field before typing on"),
        ),
        K::Partial => (
            "The last expansion was only partly written.".into(),
            Some("check the field and correct it by hand"),
        ),
        K::BlockedByPolicy => (
            "Organization or command policy stopped the last expansion.".into(),
            Some("`wayexpand doctor --security`"),
        ),
        K::InterruptedByTyping => (
            "More typing arrived before WayExpand could type the replacement, so it dropped the expansion rather than risk typing in the wrong place.".into(),
            Some("pause briefly after the trigger"),
        ),
        K::SnippetActionFailed => (
            format!("The command or form behind the last snippet failed: {completion}."),
            Some("`wayexpand test 'the trigger'` runs it with its error"),
        ),
        K::Expanded | K::ExpandedCursorMisplaced | K::Other => (
            format!("The last expansion stopped: {completion}."),
            Some("`wayexpand doctor`"),
        ),
    };
    (answer, fix.map(str::to_owned))
}

fn outcome_line(outcome: &RecentOutcome) -> String {
    use OutcomeKind as K;
    match outcome.kind {
        K::Expanded => "expanded (the app's text is not checked)".into(),
        K::ExpandedCursorMisplaced => "expanded, but the cursor could not be placed".into(),
        K::NotApplied => "refused before changing anything".into(),
        K::Unknown => "result unknown".into(),
        K::Partial => "partly written".into(),
        K::BlockedByPolicy => "stopped by policy".into(),
        K::InterruptedByTyping => "dropped: typing continued".into(),
        K::SnippetActionFailed | K::Other => format!("stopped: {}", outcome.completion),
    }
}

/// "12 s ago", "5 min ago", "3 h ago": no time zone needed.
fn ago(now: u64, at: u64) -> String {
    let seconds = now.saturating_sub(at);
    match seconds {
        0..=59 => format!("{seconds} s ago"),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

/// IBus explanations are clauses; print them as a sentence.
fn sentence(clause: &str) -> String {
    let mut characters = clause.chars();
    let mut sentence = characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect::<String>())
        .unwrap_or_default();
    if !sentence.ends_with('.') {
        sentence.push('.');
    }
    sentence
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnosis(body: &str) -> ExpansionDiagnosis {
        ExpansionDiagnosis::from_status(&DaemonStatus::parse(&format!("running\n{body}")))
    }

    #[test]
    fn every_verdict_has_an_answer_and_only_real_commands_as_fixes() {
        let bodies = [
            "matching_unavailable_reason=paused\n",
            "matching_unavailable_reason=capture_disconnected\n",
            "matching_unavailable_reason=keyboard_layout_unavailable\n",
            "matching_unavailable_reason=replacement_route_unavailable\n",
            "matching_unavailable_reason=organization_policy_route_incompatible\n",
            "matching_unavailable_reason=configuration_unhealthy\n",
            "matching_unavailable_reason=replacement_not_admitted\n",
            "matching_unavailable_reason=something_new\n",
            "input_events_total=0\n",
            "input_events_total=5\n",
            "recent_expansion_outcomes=[{\"at_unix\":1,\"completion\":\"applied\"}]\n",
            "recent_expansion_outcomes=[{\"at_unix\":1,\"completion\":\"partial_write\"}]\n",
            "recent_expansion_outcomes=[{\"at_unix\":1,\"completion\":\"helper timed out\"}]\n",
        ];
        let known = crate::help::COMMANDS
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();
        for body in bodies {
            let (answer, fix) = verdict_text(&diagnosis(body), 1);
            assert!(answer.ends_with('.'), "{body}: {answer}");
            let commands = fix
                .iter()
                .flat_map(|fix| fix.split('`').skip(1).step_by(2))
                .filter_map(|quoted| quoted.strip_prefix("wayexpand "));
            for command in commands {
                let name = command.split_whitespace().next().unwrap_or_default();
                assert!(known.contains(&name), "{body}: unknown command {name}");
            }
        }
    }

    #[test]
    fn json_names_the_verdict_without_any_text() {
        let route = DaemonRoute {
            source: "evdev".into(),
            backend: "libei".into(),
            diagnosis: diagnosis(
                "matching_unavailable_reason=none\ninput_events_total=9\n\
                 recent_expansion_outcomes=[{\"at_unix\":4,\"completion\":\"input arrived before output\"}]\n",
            ),
        };
        let value = report_json(Some(&route), None, 5);
        assert_eq!(value["daemon"]["verdict"], "last_stopped");
        assert_eq!(value["daemon"]["last_outcome"], "interrupted_by_typing");
        assert_eq!(
            value["daemon"]["recent"][0]["outcome"],
            "interrupted_by_typing"
        );
        assert!(value["ibus"].is_null());
        let no_field = report_json(None, Some(&None), 5);
        assert!(no_field["daemon"].is_null());
        assert!(no_field["ibus"]["verdict"].is_null());
    }

    #[test]
    fn times_are_relative() {
        assert_eq!(ago(100, 88), "12 s ago");
        assert_eq!(ago(1000, 700), "5 min ago");
        assert_eq!(ago(20_000, 9_000), "3 h ago");
        // A clock that went backwards is not "in the future".
        assert_eq!(ago(5, 10), "0 s ago");
    }

    #[test]
    fn ibus_clauses_become_sentences() {
        assert_eq!(sentence("no keys were seen"), "No keys were seen.");
        assert_eq!(sentence("done."), "Done.");
    }
}
