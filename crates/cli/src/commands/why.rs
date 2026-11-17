//! `wayexpand why`: the most likely reason the last trigger did not expand.
//!
//! Combines the daemon's content-free status (`ExpansionDiagnosis`) with the
//! IBus engine's per-field telemetry. Neither records typed text, triggers
//! or replacements, so the output is safe to paste into a bug report.

use std::time::{SystemTime, UNIX_EPOCH};

use wayexpand_backend_ibus::{
    chain::{self, IbusChainReport, IbusCheckState, IbusDesktop},
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

struct InputMethodGuidance {
    state: &'static str,
    answer: &'static str,
    action: &'static str,
}

fn daemon_route(status: &DaemonStatus) -> Option<DaemonRoute> {
    if status.status_schema() != Some(CONTROL_STATUS_SCHEMA) {
        return None;
    }
    // The control-only service (stdin source) captures no typing.
    if status.field("source") == Some("stdin") {
        return None;
    }
    Some(DaemonRoute {
        source: status.field("source").unwrap_or_default().to_owned(),
        backend: status.field("backend").unwrap_or_default().to_owned(),
        diagnosis: ExpansionDiagnosis::from_status(status),
    })
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
    // Never interpret counters or outcome labels from an unknown status
    // schema. In particular, stale daemon counters must not look like a
    // diagnosis of the trigger the user just typed.
    let daemon_status_schema = status.as_ref().and_then(DaemonStatus::status_schema);
    let daemon_status_compatible = status
        .as_ref()
        .map(|status| status.status_schema() == Some(CONTROL_STATUS_SCHEMA));
    let daemon = status.as_ref().and_then(daemon_route);
    // GNOME and KDE select IBus through desktop input-source controls. Inspect
    // that route only when there is no other typing daemon to diagnose, so an
    // unused IBus installation doesn't look broken.
    let desktop = IbusDesktop::detect();
    let input_method_route = (daemon.is_none()
        && daemon_status_compatible != Some(false)
        && matches!(desktop, IbusDesktop::Gnome | IbusDesktop::KdeWayland))
    .then(chain::diagnose);
    let input_method_guidance = input_method_route
        .as_ref()
        .and_then(diagnose_input_method_route);
    let ibus_field = if daemon.is_none() && daemon_status_compatible != Some(false) {
        telemetry::read_status()
            .filter(|reading| reading.live)
            .map(|reading| newest_typed_field(&reading.status.recent_fields).cloned())
    } else {
        None
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report_json(
                daemon.as_ref(),
                ibus_field.as_ref(),
                now,
                daemon_status_schema,
                daemon_status_compatible,
                input_method_guidance.as_ref(),
            ))?
        );
    } else {
        print_report(
            daemon.as_ref(),
            ibus_field.as_ref(),
            now,
            daemon_status_schema,
            daemon_status_compatible,
            input_method_guidance.as_ref(),
        );
    }
    if daemon_status_compatible == Some(false) {
        return Err(classified_error(
            CliErrorKind::Daemon,
            format!(
                "WX-VERSION-001: the running daemon status schema {} is incompatible with this CLI (expected {CONTROL_STATUS_SCHEMA}); run `wayexpand upgrade --check` before trusting this diagnosis",
                daemon_status_schema
                    .map(|schema| schema.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ),
        ));
    }
    if daemon.is_none() && ibus_field.is_none() && input_method_route.is_none() {
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

fn diagnose_input_method_route(report: &IbusChainReport) -> Option<InputMethodGuidance> {
    if !matches!(report.desktop, IbusDesktop::Gnome | IbusDesktop::KdeWayland) {
        return None;
    }
    let is_gnome = report.desktop == IbusDesktop::Gnome;
    let registered = report.check(if is_gnome {
        "gnome-source-registered"
    } else {
        "registered"
    });
    let registration_state = registered.map_or(IbusCheckState::Unknown, |check| check.state);
    if registration_state != IbusCheckState::Pass {
        return Some(match registration_state {
            IbusCheckState::Fail => InputMethodGuidance {
                state: "not_registered",
                answer: if is_gnome {
                    "WayExpand is not registered as a GNOME input source, so GNOME cannot send it your typing."
                } else {
                    "The WayExpand IBus engine is not registered, so KDE cannot route your typing to it."
                },
                action: if is_gnome {
                    "Run `wayexpand repair --gnome-input-source`, then `wayexpand doctor --gnome`. Until then, use `wayexpand-gui --picker --copy-only` and paste manually."
                } else {
                    "Run `wayexpand setup --backend ibus`, then `wayexpand ibus status`. Until then, use `wayexpand-gui --picker --copy-only` and paste manually."
                },
            },
            _ => InputMethodGuidance {
                state: "registration_unknown",
                answer: if is_gnome {
                    "WayExpand's GNOME input-source registration could not be verified."
                } else {
                    "WayExpand's IBus engine registration could not be verified."
                },
                action: if is_gnome {
                    "Run `wayexpand doctor --gnome --json` and follow the first failed or unknown check. You can use `wayexpand-gui --picker --copy-only` and paste manually."
                } else {
                    "Run `wayexpand ibus status --json` and follow the first failed or unknown check. You can use `wayexpand-gui --picker --copy-only` and paste manually."
                },
            },
        });
    }
    if is_gnome {
        let engine_registration = report.check("registered");
        if !engine_registration.is_some_and(|check| check.state == IbusCheckState::Pass) {
            let state = engine_registration.map_or(IbusCheckState::Unknown, |check| check.state);
            return Some(InputMethodGuidance {
                state: if state == IbusCheckState::Fail {
                    "ibus_engine_not_registered"
                } else {
                    "ibus_engine_registration_unknown"
                },
                answer: if state == IbusCheckState::Fail {
                    "GNOME lists WayExpand as an input source, but the IBus engine registry does not contain it."
                } else {
                    "WayExpand's IBus engine registry could not be verified."
                },
                action: "Run `wayexpand ibus status --json` and follow the engine-registration check. If the component is missing, run `wayexpand setup --backend ibus`, then rerun `wayexpand ibus status`. Use `wayexpand-gui --picker --copy-only` and paste manually meanwhile.",
            });
        }
    }
    let active = report.check(if is_gnome {
        "gnome-source-active"
    } else {
        "selected"
    });
    let selection_state = active.map_or(IbusCheckState::Unknown, |check| check.state);
    if selection_state != IbusCheckState::Pass {
        return Some(match selection_state {
            IbusCheckState::Warn | IbusCheckState::Fail => InputMethodGuidance {
                state: "not_selected",
                answer: if is_gnome {
                    "Text expansion is unavailable because GNOME has not selected the WayExpand input source."
                } else {
                    "Text expansion is unavailable because WayExpand is not the selected IBus input method."
                },
                action: if is_gnome {
                    "Select WayExpand in Settings → Keyboard or the input-source menu. Then run `wayexpand ibus watch --seconds 30` while typing harmless text in a normal field. For now, use `wayexpand-gui --picker --copy-only` and paste manually."
                } else {
                    "Select WayExpand from KDE's input-method controls, then run `wayexpand ibus watch --seconds 30` while typing harmless text in a normal field. For now, use `wayexpand-gui --picker --copy-only` and paste manually."
                },
            },
            _ => InputMethodGuidance {
                state: "selection_unknown",
                answer: if is_gnome {
                    "GNOME's active input source could not be confirmed; WayExpand cannot yet tell whether typing reaches its engine."
                } else {
                    "The selected IBus engine could not be confirmed; WayExpand cannot yet tell whether typing reaches it."
                },
                action: if is_gnome {
                    "Run `wayexpand doctor --gnome --json` and follow its GNOME source or session-bridge finding. You can use `wayexpand-gui --picker --copy-only` and paste manually."
                } else {
                    "Run `wayexpand ibus status --json` and follow its first failed or unknown check. You can use `wayexpand-gui --picker --copy-only` and paste manually."
                },
            },
        });
    }
    let consent = report
        .check("consent")
        .map_or(IbusCheckState::Unknown, |check| check.state);
    if consent != IbusCheckState::Pass {
        return Some(if consent == IbusCheckState::Fail {
            InputMethodGuidance {
                state: "replacement_not_allowed",
                answer: if is_gnome { "GNOME selected WayExpand, but replacement is disabled by consent or policy." } else { "KDE selected WayExpand, but replacement is disabled by consent or policy." },
                action: "Review `wayexpand ibus status` and authorize replacement only if you accept its non-atomic behavior; otherwise use `wayexpand-gui --picker --copy-only` and paste manually.",
            }
        } else {
            InputMethodGuidance {
                state: "replacement_consent_unknown",
                answer: "WayExpand cannot confirm that IBus replacement is authorized, so it will not claim the route is ready.",
                action: "Run `wayexpand validate`, then `wayexpand ibus status`. If you accept the non-atomic behavior, authorize it through guided setup; otherwise use `wayexpand-gui --picker --copy-only` and paste manually.",
            }
        });
    }
    let engine = report.check("engine");
    if !engine.is_some_and(|check| check.state == IbusCheckState::Pass) {
        let state = engine.map_or(IbusCheckState::Unknown, |check| check.state);
        return Some(InputMethodGuidance {
            state: if state == IbusCheckState::Warn {
                "engine_not_running"
            } else {
                "engine_state_unknown"
            },
            answer: "WayExpand's engine is registered, but its running input context has not been confirmed.",
            action: if is_gnome {
                "Select another GNOME input source and select WayExpand again, then run `wayexpand ibus status`. If the engine still does not start, run `wayexpand doctor --gnome --json`. Use `wayexpand-gui --picker --copy-only` and paste manually meanwhile."
            } else {
                "Switch away from WayExpand and select it again in KDE's input-method controls, then run `wayexpand ibus status`. Use `wayexpand-gui --picker --copy-only` and paste manually meanwhile."
            },
        });
    }
    let field_input = report.check("field-input");
    if !field_input.is_some_and(|check| check.state == IbusCheckState::Pass) {
        return Some(InputMethodGuidance {
            state: "field_input_unverified",
            answer: if is_gnome { "GNOME selected WayExpand, but no normal text field has yet been confirmed to deliver input to its engine." } else { "KDE selected WayExpand, but no normal text field has yet been confirmed to deliver input to its engine." },
            action: "Run `wayexpand ibus watch --seconds 30`, focus the affected field, and type harmless text. This checks input delivery only; use a real expansion test to confirm matching and insertion. For now, use `wayexpand-gui --picker --copy-only` and paste manually.",
        });
    }
    Some(InputMethodGuidance {
        state: "input_observed_delivery_unverified",
        answer: "WayExpand observed input from a normal IBus field in this session, but telemetry does not identify the affected application. This does not prove a trigger matched or that the application applied replacement text.",
        action: "Run a harmless trigger once in the affected application and inspect the visible text. A matcher preview checks only the pattern; use the real-application test in Getting started to verify delivery. Do not retry output whose result is uncertain. For a reliable manual path, use `wayexpand-gui --picker --copy-only` and paste.",
    })
}

fn report_json(
    daemon: Option<&DaemonRoute>,
    ibus_field: Option<&Option<IbusFieldActivity>>,
    now: u64,
    daemon_status_schema: Option<u32>,
    daemon_status_compatible: Option<bool>,
    input_method: Option<&InputMethodGuidance>,
) -> serde_json::Value {
    let daemon_json = daemon.map(|route| {
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
            "next_action": field.map(|field| ibus_field_action(IbusFieldVerdict::of(field))),
            "keys": field.map(|field| field.keys),
            "expansions": field.map(|field| field.expansions),
        })
    });
    // Keep one plain-language conclusion alongside the detailed telemetry.
    // Consumers should not have to reconstruct an action from empty fields or
    // infer that a matcher result proves delivery into an application.
    let mut diagnosis = if daemon_status_compatible == Some(false) {
        serde_json::json!({
            "state": "daemon_schema_incompatible",
            "answer": "WayExpand cannot trust the running daemon's diagnostics because its status schema is incompatible with this CLI.",
            "next_action": "Run `wayexpand upgrade --check` and resolve the reported version mismatch before using this diagnosis.",
            "application_delivery_verified": false,
        })
    } else if let Some(guidance) = input_method {
        serde_json::json!({
            "state": guidance.state,
            "answer": guidance.answer,
            "next_action": guidance.action,
            "application_delivery_verified": false,
        })
    } else if let Some(route) = daemon {
        let verdict = route.diagnosis.verdict(now);
        let (answer, next_action) = verdict_text(&route.diagnosis, now);
        serde_json::json!({
            "state": verdict.code(),
            "answer": answer,
            "next_action": next_action,
            "application_delivery_verified": false,
        })
    } else if let Some(field) = ibus_field {
        match field {
            Some(field) => {
                let verdict = IbusFieldVerdict::of(field);
                serde_json::json!({
                    "state": verdict.label(),
                    "answer": sentence(verdict.explanation()),
                    "next_action": ibus_field_action(verdict),
                    "application_delivery_verified": false,
                })
            }
            None => serde_json::json!({
                "state": "no_input_observed",
                "answer": "No typing has reached the IBus engine in this login session.",
                "next_action": "Check that WayExpand is the active input method with `wayexpand ibus status`, then type harmless text while `wayexpand ibus watch --seconds 30` runs.",
                "application_delivery_verified": false,
            }),
        }
    } else {
        serde_json::json!({
            "state": "no_typing_route",
            "answer": "Text expansion is unavailable because no WayExpand typing route is running.",
            "next_action": "Run `wayexpand setup`, then `wayexpand doctor`. For immediate manual insertion, open `wayexpand-gui --picker --copy-only` and paste the selected snippet.",
            "application_delivery_verified": false,
        })
    };
    // Keep the two user-visible stages separate in machine-readable output.
    // A recent outcome can prove that a trigger matched and output was
    // submitted, but WayExpand cannot observe whether the application applied
    // that output. Matcher previews are likewise local checks only.
    if let Some(object) = diagnosis.as_object_mut() {
        object.insert(
            "pattern_match".into(),
            serde_json::Value::String("unknown".into()),
        );
        object.insert(
            "delivery_status".into(),
            serde_json::Value::String("unknown".into()),
        );
        object.insert(
            "matcher_preview_proves_application_delivery".into(),
            serde_json::Value::Bool(false),
        );
    }
    if let Some(route) = daemon {
        let verdict = route.diagnosis.verdict(now);
        let pattern_match = match verdict {
            Verdict::LastStopped(_) | Verdict::LastExpanded => "confirmed_recently",
            Verdict::NothingMatched => "not_observed_recently",
            Verdict::NoTypingSeen => "no_input_observed",
            Verdict::MatchingOff(_) => "blocked",
            Verdict::InputStatusUnknown => "unknown",
        };
        let delivery = match verdict {
            Verdict::LastExpanded => "submitted_unverified",
            Verdict::LastStopped(OutcomeKind::Partial) => "partial_or_uncertain",
            Verdict::LastStopped(OutcomeKind::Unknown) => "uncertain",
            Verdict::LastStopped(OutcomeKind::NotApplied) => "not_submitted",
            Verdict::LastStopped(OutcomeKind::BlockedByPolicy) => "blocked_by_policy",
            Verdict::LastStopped(OutcomeKind::InterruptedByTyping) => "stopped_before_output",
            Verdict::LastStopped(OutcomeKind::SnippetActionFailed) => "snippet_action_failed",
            Verdict::LastStopped(_) => "unknown",
            Verdict::MatchingOff(_) => "blocked",
            Verdict::NoTypingSeen | Verdict::NothingMatched | Verdict::InputStatusUnknown => {
                "not_observed"
            }
        };
        if let Some(object) = diagnosis.as_object_mut() {
            object.insert(
                "pattern_match".into(),
                serde_json::Value::String(pattern_match.into()),
            );
            object.insert(
                "delivery_status".into(),
                serde_json::Value::String(delivery.into()),
            );
            object.insert(
                "matcher_preview_proves_application_delivery".into(),
                serde_json::Value::Bool(false),
            );
        }
    }
    serde_json::json!({
        "diagnosis": diagnosis,
        "daemon_status_schema": daemon_status_schema,
        "daemon_status_compatible": daemon_status_compatible,
        "daemon": daemon_json,
        "ibus": ibus,
        "desktop_input_route": input_method.map(|guidance| serde_json::json!({
            "state": guidance.state,
            "answer": guidance.answer,
            "next_action": guidance.action,
        })),
        "matcher_preview_proves_application_delivery": false,
        "application_delivery_verified": false
    })
}

fn print_report(
    daemon: Option<&DaemonRoute>,
    ibus_field: Option<&Option<IbusFieldActivity>>,
    now: u64,
    daemon_status_schema: Option<u32>,
    daemon_status_compatible: Option<bool>,
    input_method: Option<&InputMethodGuidance>,
) {
    if daemon_status_compatible == Some(false) {
        println!("Keyboard route");
        println!(
            "  Cannot diagnose: daemon status schema {} is incompatible with this CLI (expected {CONTROL_STATUS_SCHEMA}).",
            daemon_status_schema
                .map(|schema| schema.to_string())
                .unwrap_or_else(|| "unknown".into())
        );
        println!("  Fix: run `wayexpand upgrade --check` before trusting route diagnostics.");
        if ibus_field.is_some() {
            println!();
        } else {
            return;
        }
    }
    if daemon.is_none()
        && ibus_field.is_none()
        && input_method.is_none()
        && daemon_status_compatible != Some(false)
    {
        println!("No WayExpand typing route is running, so nothing can expand.");
        println!("Next: run `wayexpand setup`, then `wayexpand doctor`.");
        println!("For manual insertion meanwhile: open `wayexpand-gui --picker --copy-only` and paste the selected snippet.");
        return;
    }
    if let Some(guidance) = input_method {
        println!("Desktop input route: {}", guidance.state.replace('_', " "));
        println!("  {}", guidance.answer);
        println!("  Next: {}", guidance.action);
        println!("  A matcher preview does not verify text insertion into this application.");
        println!();
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
                println!("  Next: {}", ibus_field_action(verdict));
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
    if daemon_status_compatible != Some(false) || ibus_field.is_some() {
        println!("To check one trigger: wayexpand explain 'the text you typed'");
    }
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
        Verdict::InputStatusUnknown => (
            "The daemon did not report input-event counters, so WayExpand cannot tell whether your typing reached it.".to_owned(),
            Some("Run `wayexpand debug --live`, type harmless text in the affected application, and check whether input is observed. If not, run `wayexpand doctor`. Use `wayexpand-gui --picker --copy-only` and paste meanwhile.".to_owned()),
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
            Some("Run `wayexpand explain 'the trigger you typed'` to check the pattern and app filter; this does not test desktop input or insertion. Then run `wayexpand debug --test 'YOUR_TRIGGER'` in the affected app and inspect the visible text.".to_owned()),
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
            "WayExpand matched the trigger and submitted replacement output, but cannot verify that the application changed its text."
                .to_owned(),
            Some("inspect the field; if the text is missing or wrong, run a harmless real-application test with `wayexpand debug --test 'YOUR_TRIGGER'`; do not retry uncertain output".to_owned()),
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

fn ibus_field_action(verdict: IbusFieldVerdict) -> &'static str {
    use IbusFieldVerdict as V;
    match verdict {
        V::OutputEmittedUnverified => {
            "Inspect the field before continuing; if the text is wrong, do not replay it. Run a harmless real-application test after recovery."
        }
        V::ReplacementContextObserved => {
            "Type a harmless configured trigger in this field and verify the visible result; surrounding text alone does not prove insertion."
        }
        V::NoContentType => {
            "Use a normal, non-sensitive text field and run `wayexpand ibus watch --seconds 30` while typing harmless text."
        }
        V::Sensitive => {
            "WayExpand intentionally suppresses expansion in sensitive fields. Test in a normal text field instead."
        }
        V::NoSurroundingText => {
            "This field cannot support safe automatic replacement. Open `wayexpand-gui --picker --copy-only` and paste manually."
        }
        V::Mismatch => {
            "Check the exact trigger with `wayexpand explain 'THE_TRIGGER'`; inspect the field before trying again."
        }
        V::Policy => {
            "Run `wayexpand policy check --json` and `wayexpand doctor --security`; ask the policy administrator before changing a restriction."
        }
        V::Idle => {
            "Focus this field, run `wayexpand ibus watch --seconds 30`, and type harmless text to check input delivery."
        }
    }
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
        let value = report_json(
            Some(&route),
            None,
            5,
            Some(CONTROL_STATUS_SCHEMA),
            Some(true),
            None,
        );
        assert_eq!(value["daemon"]["verdict"], "last_stopped");
        assert_eq!(value["diagnosis"]["state"], "last_stopped");
        assert!(value["diagnosis"]["answer"].is_string());
        assert!(value["diagnosis"]["next_action"].is_string());
        assert_eq!(value["diagnosis"]["application_delivery_verified"], false);
        assert_eq!(value["daemon"]["last_outcome"], "interrupted_by_typing");
        assert_eq!(
            value["daemon"]["recent"][0]["outcome"],
            "interrupted_by_typing"
        );
        assert!(value["ibus"].is_null());
        let no_field = report_json(None, Some(&None), 5, None, None, None);
        assert!(no_field["daemon"].is_null());
        assert!(no_field["ibus"]["verdict"].is_null());
        assert_eq!(no_field["diagnosis"]["state"], "no_input_observed");
        assert!(no_field["diagnosis"]["next_action"].is_string());
        assert!(no_field["daemon_status_schema"].is_null());
        assert!(no_field["daemon_status_compatible"].is_null());

        let no_route = report_json(None, None, 5, None, None, None);
        assert_eq!(no_route["diagnosis"]["state"], "no_typing_route");
        assert!(no_route["diagnosis"]["answer"].is_string());
        assert!(no_route["diagnosis"]["next_action"].is_string());
        assert_eq!(
            no_route["diagnosis"]["application_delivery_verified"],
            false
        );
    }

    #[test]
    fn incompatible_daemon_schema_never_produces_a_route_verdict() {
        let stale = DaemonStatus::parse(&format!(
            "running\nstatus_schema={}\nsource=evdev\nbackend=libei\n\
             matching_unavailable_reason=none\ninput_events_total=9\n\
             recent_expansion_outcomes=[{{\"at_unix\":4,\"completion\":\"applied\"}}]\n",
            CONTROL_STATUS_SCHEMA - 1
        ));
        assert!(daemon_route(&stale).is_none());
        let stale_report = report_json(None, None, 5, stale.status_schema(), Some(false), None);
        assert_eq!(stale_report["daemon"], serde_json::Value::Null);
        assert_eq!(stale_report["daemon_status_compatible"], false);
        assert_eq!(
            stale_report["diagnosis"]["state"],
            "daemon_schema_incompatible"
        );
        assert!(stale_report["diagnosis"]["next_action"]
            .as_str()
            .unwrap()
            .contains("wayexpand upgrade --check"));

        let compatible = DaemonStatus::parse(&format!(
            "running\nstatus_schema={CONTROL_STATUS_SCHEMA}\nsource=evdev\nbackend=libei\n\
             matching_unavailable_reason=none\ninput_events_total=9\n\
             recent_expansion_outcomes=[{{\"at_unix\":4,\"completion\":\"applied\"}}]\n"
        ));
        assert!(daemon_route(&compatible).is_some());
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

    #[test]
    fn gnome_source_selection_diagnosis_names_the_action_and_manual_fallback() {
        let report = IbusChainReport {
            desktop: IbusDesktop::Gnome,
            checks: vec![
                wayexpand_backend_ibus::chain::IbusCheck {
                    id: "gnome-source-registered",
                    title: "WayExpand registered in GNOME",
                    state: IbusCheckState::Pass,
                    problem_id: None,
                    detail: "GNOME source is registered".into(),
                    fix: None,
                },
                wayexpand_backend_ibus::chain::IbusCheck {
                    id: "registered",
                    title: "WayExpand engine is registered in IBus",
                    state: IbusCheckState::Pass,
                    problem_id: None,
                    detail: "IBus registry lists WayExpand".into(),
                    fix: None,
                },
                wayexpand_backend_ibus::chain::IbusCheck {
                    id: "gnome-source-active",
                    title: "WayExpand selected in GNOME",
                    state: IbusCheckState::Warn,
                    problem_id: Some("WX-GNOME-IM-002"),
                    detail: "GNOME reports another source".into(),
                    fix: Some("select WayExpand".into()),
                },
            ],
            activity: None,
        };
        let guidance = diagnose_input_method_route(&report).unwrap();
        assert_eq!(guidance.state, "not_selected");
        assert!(guidance.answer.contains("has not selected"));
        assert!(guidance.action.contains("Settings → Keyboard"));
        assert!(guidance.action.contains("wayexpand ibus watch"));
        assert!(guidance.action.contains("--copy-only"));

        let value = report_json(None, None, 0, None, None, Some(&guidance));
        assert_eq!(value["diagnosis"]["state"], "not_selected");
        assert!(value["diagnosis"]["next_action"]
            .as_str()
            .unwrap()
            .contains("Settings → Keyboard"));
        assert_eq!(value["diagnosis"]["application_delivery_verified"], false);
        assert_eq!(value["desktop_input_route"]["state"], "not_selected");
        assert_eq!(value["matcher_preview_proves_application_delivery"], false);
        assert_eq!(value["application_delivery_verified"], false);
    }
}
