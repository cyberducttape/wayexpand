//! `wayexpand why`: the most likely reason the last trigger did not expand.
//!
//! Combines the daemon's content-free status (`ExpansionDiagnosis`) with the
//! IBus engine's per-field telemetry. Neither records typed text, triggers
//! or replacements, so the output is safe to paste into a bug report.

use std::time::{SystemTime, UNIX_EPOCH};

use wayexpand_backend_ibus::{
    chain::{self, IbusChainReport, IbusCheckState, IbusDesktop},
    telemetry::{self, IbusFieldActivity, IbusLatestMatch},
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
    capture_app_scope_state: Option<String>,
    raw_capture_fallback_enabled: bool,
    raw_capture_app_count: Option<usize>,
    matching_unavailable_reason: Option<String>,
    window_process_identity_state: Option<String>,
    terminal_paste_last_failure: Option<String>,
    enabled_expansions: Option<usize>,
}

/// Content-free IBus telemetry about the most recent field to receive input.
/// `field.focused` distinguishes current evidence from recent history after
/// the user switches to the terminal or diagnostics to ask for help.
struct IbusFieldEvidence {
    recent_typed: Option<IbusFieldActivity>,
    keys_seen: u64,
}

#[derive(Clone, Copy)]
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
        capture_app_scope_state: status.field("capture_app_scope_state").map(str::to_owned),
        raw_capture_fallback_enabled: status.field("raw_capture_fallback_enabled") == Some("true"),
        raw_capture_app_count: status
            .field("raw_capture_app_count")
            .and_then(|count| count.parse::<usize>().ok()),
        matching_unavailable_reason: status
            .field("matching_unavailable_reason")
            .map(str::to_owned),
        window_process_identity_state: status
            .field("window_process_identity_state")
            .map(str::to_owned),
        terminal_paste_last_failure: status
            .field("terminal_paste_last_failure")
            .filter(|failure| *failure != "none")
            .map(str::to_owned),
        enabled_expansions: status
            .field("config_enabled_expansions")
            .and_then(|count| count.parse::<usize>().ok()),
    })
}

/// Current focus-scope refusal for a terminal-only route. This is live route
/// health, not a claim about the identity of an earlier target application.
fn terminal_scope_guidance(route: &DaemonRoute) -> Option<InputMethodGuidance> {
    if !route_is_limited_for_focused_app(route) {
        return None;
    }
    match route.matching_unavailable_reason.as_deref() {
        Some("application_scope_unknown")
            if route.window_process_identity_state.as_deref() == Some("compositor-process-id") =>
        {
            Some(InputMethodGuidance {
                state: "terminal_compositor_identity",
                answer: "KWin's focus tracker is connected, but it reported KWin's own process ID instead of the focused terminal's application identity. The terminal-only route therefore blocks matching.",
                action: "Use `wayexpand-gui --picker --copy-only`, copy the snippet, and paste it in the terminal. This diagnosis already confirms that KWin supplied its own PID; no allowlist setting can make that a trustworthy terminal identity. Do not allowlist `kwin_wayland` or use a window title. Retest automatic expansion only after KWin reports a verified client application ID.",
            })
        }
        Some("application_scope_unknown") => Some(InputMethodGuidance {
            state: "terminal_focus_unknown",
            answer: "The terminal-only route is failing closed because the compositor has not supplied a focused application ID. It cannot match expansions until focus tracking identifies the target.",
            action: "Immediate workaround: run `wayexpand-gui --picker --copy-only`, copy the snippet, and paste it in the terminal. To diagnose the blocked route, focus the terminal and run `wayexpand debug --focused-app-id --json`. If `focused_app_id` is null, follow its problem ID and suggested action; do not allowlist a compositor or window title.",
        }),
        Some("application_scope_denied") => Some(InputMethodGuidance {
            state: "terminal_app_not_allowed",
            answer: if route.raw_capture_fallback_enabled {
                "The focused application is outside the configured terminal and raw-capture app lists, so this evdev route will not match there."
            } else {
                "The currently focused application is outside the configured terminal-only allowlist, so this route will not match there."
            },
            action: if route.raw_capture_fallback_enabled {
                "If you explicitly accept raw keyboard capture in this app, run `wayexpand debug --focused-app-id --json`, add only its verified application ID to `[settings].raw_capture_apps`, and retest. Otherwise use a supported IBus route or Quick Picker copy-and-paste."
            } else {
                "If this is a terminal you intend to support, run `wayexpand debug --focused-app-id --json` while it is focused and add only its verified application ID to `[settings].terminal_apps`; then restart the selected service and retest. Browser and editor fields need another route or Quick Picker copy-and-paste."
            },
        }),
        _ => None,
    }
}

/// A terminal-scoped evdev service can also own explicitly allowlisted raw
/// capture apps. When its live focus scope says the focused app is allowed,
/// diagnose that route instead of presenting KDE's parallel IBus route as
/// the one that should receive this app's input.
fn route_is_limited_for_focused_app(route: &DaemonRoute) -> bool {
    route.diagnosis.terminal_apps_only
        && route.capture_app_scope_state.as_deref() != Some("allowed")
}

fn ibus_route_relevant(daemon: Option<&DaemonRoute>) -> bool {
    daemon.is_none_or(|route| route.source == "stdin" || route_is_limited_for_focused_app(route))
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
    let daemon_build_state = status.as_ref().map(|status| {
        let commit = status.field("daemon_commit");
        let version = status.field("daemon_version");
        if crate::doctor::status::daemon_build_matches(commit, version) {
            "current"
        } else if crate::doctor::status::daemon_build_unknown(commit, version) {
            "unverified"
        } else {
            "restart_required"
        }
    });
    // A matching protocol schema does not make stale daemon telemetry current.
    // Do not diagnose a KWin focus event or output transaction produced by a
    // different build as though it described this CLI's behavior.
    if daemon_status_compatible == Some(true)
        && daemon_build_state.is_some_and(|state| state != "current")
    {
        let state = daemon_build_state.unwrap_or("unverified");
        let (answer, next_action) = if state == "restart_required" {
            (
                "The running WayExpand daemon was built differently from this CLI, so its route and expansion history may be stale.",
                "Run `wayexpand upgrade --check` with the CLI from the active installation, then apply the reported upgrade or restart and run `wayexpand why` again.",
            )
        } else {
            (
                "WayExpand cannot verify that the running daemon matches this CLI, so it cannot trust route or expansion history.",
                "Run `wayexpand upgrade --check` with the CLI from the active installation. Resolve the build identity warning, then run `wayexpand why` again.",
            )
        };
        let next_action = daemon_build_recovery_action(state, next_action);
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "diagnosis": {
                        "state": if state == "restart_required" { "daemon_build_mismatch" } else { "daemon_build_unverified" },
                        "answer": answer,
                        "next_action": next_action,
                        "pattern_match": "not_evaluated",
                        "delivery_status": "not_attempted",
                        "application_delivery_verified": false,
                        "matcher_preview_proves_application_delivery": false,
                    },
                    "daemon_status_schema": daemon_status_schema,
                    "daemon_status_compatible": daemon_status_compatible,
                    "daemon_build_state": state,
                    "daemon": status.as_ref().map(|status| serde_json::json!({
                        "source": status.field("source"),
                        "backend": status.field("backend"),
                        "version": status.field("daemon_version"),
                        "commit": status.field("daemon_commit"),
                    })),
                    "ibus": serde_json::Value::Null,
                    "desktop_input_route": serde_json::Value::Null,
                    "matcher_preview_proves_application_delivery": false,
                    "application_delivery_verified": false,
                })
            );
        } else {
            println!("Why expansion did not work: {answer}");
            println!("Next: {next_action}");
            println!("Trigger match: not evaluated. Text delivery: not attempted.");
            println!("A matcher preview does not verify insertion into an application.");
        }
        return Err(classified_error(
            CliErrorKind::Daemon,
            format!(
                "WX-VERSION-001: {answer} {}",
                runnable_cli_suggestion(
                    "Run `wayexpand upgrade --check` before trusting route diagnostics."
                )
            ),
        ));
    }
    let daemon = status.as_ref().and_then(daemon_route);
    let enabled_expansions = status.as_ref().and_then(|status| {
        (daemon_status_compatible == Some(true) && daemon_build_state == Some("current"))
            .then(|| status.field("config_enabled_expansions"))
            .flatten()
            .and_then(|count| count.parse::<usize>().ok())
    });
    // GNOME and KDE select IBus through desktop input-source controls. A
    // control-only daemon can coexist with the IBus typing engine, and a
    // terminal-only capture daemon does not cover browser/editor fields. Keep
    // IBus evidence available in both cases; focused-field evidence determines
    // which route can explain the latest application input.
    let desktop = IbusDesktop::detect();
    let ibus_route_relevant = ibus_route_relevant(daemon.as_ref());
    let input_method_route = (ibus_route_relevant
        && daemon_status_compatible != Some(false)
        && matches!(desktop, IbusDesktop::Gnome | IbusDesktop::KdeWayland))
    .then(chain::diagnose);
    // A control-only daemon does not capture typing, but its pause state also
    // governs the paired IBus engine. Keep that state for the IBus diagnosis
    // instead of discarding it with `daemon_route`.
    let input_method_paused = daemon_status_compatible == Some(true)
        && daemon_build_state == Some("current")
        && status
            .as_ref()
            .is_some_and(|status| status.field("paused") == Some("true"));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let ibus_field = if ibus_route_relevant && daemon_status_compatible != Some(false) {
        telemetry::read_status()
            .filter(|reading| reading.live)
            .map(|reading| IbusFieldEvidence {
                recent_typed: newest_typed_field(&reading.status.recent_fields, now).cloned(),
                keys_seen: reading.status.totals.keys,
            })
    } else {
        None
    };
    let input_method_guidance = input_method_route.as_ref().and_then(|report| {
        diagnose_input_method_route(
            report,
            ibus_field
                .as_ref()
                .and_then(|evidence| evidence.recent_typed.as_ref()),
            enabled_expansions,
            input_method_paused,
        )
    });
    if json {
        let mut report = report_json(
            daemon.as_ref(),
            ibus_field.as_ref(),
            now,
            daemon_status_schema,
            daemon_status_compatible,
            input_method_guidance.as_ref(),
        );
        make_report_actions_runnable(&mut report);
        println!("{}", serde_json::to_string_pretty(&report)?);
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

/// The most recent field that received keys within the bounded diagnostic
/// window. It may have lost focus while the user opened diagnostics; callers
/// must use `focused` to avoid attributing its evidence to the current app.
fn newest_typed_field(fields: &[IbusFieldActivity], now: u64) -> Option<&IbusFieldActivity> {
    fields
        .iter()
        .filter(|field| {
            field.keys > 0
                && field.last_key_unix > 0
                && now.saturating_sub(field.last_key_unix) <= 300
        })
        .max_by_key(|field| field.last_key_unix)
}

fn diagnose_input_method_route(
    report: &IbusChainReport,
    recent_field: Option<&IbusFieldActivity>,
    enabled_expansions: Option<usize>,
    paused: bool,
) -> Option<InputMethodGuidance> {
    if paused {
        return Some(InputMethodGuidance {
            state: "paused",
            answer: "WayExpand is paused, so it will not expand triggers in the selected application.",
            action: "Run `wayexpand resume`, then rerun `wayexpand why` and perform one harmless live typing test. A matcher preview does not verify application delivery.",
        });
    }
    if !matches!(report.desktop, IbusDesktop::Gnome | IbusDesktop::KdeWayland) {
        // Desktop detection can race the IBus report or identify a session
        // outside the currently supported desktop-specific branches. Keep
        // the route visible and give the user a safe diagnostic action
        // instead of later printing the contradictory "no route" result.
        return Some(InputMethodGuidance {
            state: "desktop_route_unknown",
            answer: "An IBus route responded, but WayExpand could not identify which desktop controls its input source.",
            action: "Run `wayexpand ibus status --json` and `wayexpand doctor`; use the first failed or unknown check to identify the desktop integration. For manual insertion meanwhile, open `wayexpand-gui --picker --copy-only` and paste.",
        });
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
    // Match the GUI's route-readiness gate: fresh field activity does not make
    // an unsafe shared registration ready. Keep the repair visible in both
    // interfaces even when the engine happened to receive recent input. Check
    // this before transient KWin helper health: a typing test cannot validate
    // the route while its shared executable registration is unsafe.
    if let Some(scope) = report.check("manifest-scope") {
        if scope.state != IbusCheckState::Pass {
            let invalid = scope.state == IbusCheckState::Fail;
            return Some(InputMethodGuidance {
                state: if invalid {
                    "ibus_manifest_scope_invalid"
                } else {
                    "ibus_manifest_scope_unknown"
                },
                answer: if invalid {
                    "The shared IBus registration is not safe for multi-user use because it points to a user-owned executable or untrusted shared files."
                } else {
                    "WayExpand could not verify that the shared IBus registration and engine are protected for multi-user use."
                },
                action: if invalid {
                    "Run `wayexpand setup --backend ibus --install-system-integration` to install a system-owned engine and manifest, or install the distribution package. Setup may request administrator authorization. Then run `wayexpand ibus status --json` and test in a real application."
                } else {
                    "Run `wayexpand ibus status --json` to inspect the shared-registration check. If it fails, run `wayexpand setup --backend ibus --install-system-integration` or install the distribution package; then rerun this diagnosis."
                },
            });
        }
    }
    if !is_gnome {
        if let Some(desktop) = report.check("desktop") {
            match (desktop.problem_id, desktop.state) {
                (Some("WX-KWIN-IBUS-001"), IbusCheckState::Fail) => {
                    return Some(InputMethodGuidance {
                        state: "kwin_ibus_route_unavailable",
                        answer: "KWin is not routing keyboard input through IBus, so WayExpand cannot receive typing on this route.",
                        action: "Run `wayexpand setup --backend ibus` to configure KWin's IBus Wayland route after reviewing the changes. If setup reports that `kwriteconfig` is unavailable or cannot apply the setting, select IBus Wayland in KDE System Settings → Keyboard → Virtual Keyboard. Then run `wayexpand debug --test` in a disposable text field to verify a real insertion. Use `wayexpand-gui --picker --copy-only` and paste meanwhile.",
                    });
                }
                (Some("WX-KWIN-IBUS-002"), IbusCheckState::Warn) if recent_field.is_none() => {
                    return Some(InputMethodGuidance {
                        state: "kwin_ibus_helper_recently_crashed",
                        answer: "KWin's IBus Wayland helper has crashed recently, although it is running now. Input delivery needs a live test.",
                        action: "Run `wayexpand debug --test` in a disposable text field; it uses only a known harmless tutorial snippet. If no tutorial is configured, create one with the GUI Test Expansion flow or review a snippet and select its trigger explicitly. If the test sees no input, run `wayexpand ibus watch --seconds 30`, then `coredumpctl info ibus-ui-gtk3` and check for an IBus package update. Use `wayexpand-gui --picker --copy-only` and paste meanwhile.",
                    });
                }
                _ => {}
            }
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
                    "Select WayExpand in Settings → Keyboard or the input-source menu. Then use the GUI's Test in an app flow with the intended trigger and confirm that the replacement appears in the affected application; `wayexpand ibus watch --seconds 30` checks input capture only, not insertion. For now, use `wayexpand-gui --picker --copy-only` and paste manually."
                } else {
                    "Select WayExpand from KDE's input-method controls. Then use the GUI's Test in an app flow with the intended trigger and confirm that the replacement appears in the affected application; `wayexpand ibus watch --seconds 30` checks input capture only, not insertion. For now, use `wayexpand-gui --picker --copy-only` and paste manually."
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
    // A selected source whose engine is not alive cannot receive typing.
    // Diagnose that earlier pipeline stage before asking the user to review
    // replacement consent; otherwise two simultaneous problems can produce
    // a misleading consent-first instruction.
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
    if enabled_expansions == Some(0) && recent_field.is_some() {
        return Some(InputMethodGuidance {
            state: "no_enabled_expansions",
            answer: "WayExpand's input route is ready, but its loaded configuration has no enabled keyboard expansions.",
            action: "Open WayExpand's snippet library, enable a keyboard expansion or create a text expansion, save, and reload the configuration. Then type that trigger in a real application. A matcher preview alone does not verify delivery.",
        });
    }
    if recent_field.is_none() {
        return Some(InputMethodGuidance {
            state: "no_recent_typed_field",
            answer: "WayExpand has no recent IBus field with typed-input evidence, so it cannot tell whether the affected application sent its trigger to IBus.",
            action: "Focus the affected field, run `wayexpand ibus watch --seconds 30`, type harmless text, and rerun `wayexpand why`. This checks capture only; then run a real expansion test to verify visible delivery. A terminal-only evdev route does not cover browser or editor fields. Use `wayexpand-gui --picker --copy-only` and paste meanwhile.",
        });
    }
    // The aggregate chain check requires a currently focused normal field.
    // recent_field may be unfocused so this remains useful after the user
    // switches to a terminal to ask for help. Its timestamp is bounded above,
    // and the report marks focus changes without identifying the application.
    if let Some(field) = recent_field {
        // A fresh no-match verdict is stronger than the general fact that
        // the field supports surrounding text. Keep this aligned with the
        // GUI's TriggerNotMatched state: a matcher preview can help inspect
        // the rule, but it does not test delivery into the application.
        if field.latest_match == telemetry::IbusLatestMatch::NoMatch {
            return Some(InputMethodGuidance {
                state: "trigger_not_matched",
                answer: "WayExpand received input in a recent IBus field, but no configured trigger matched its latest input.",
                action: "Replace `TRIGGER` with the exact trigger you typed, then run `wayexpand explain 'TRIGGER'` to check its enabled state, match mode, and application filters. Type it in the affected application and verify the visible replacement; a matcher result alone does not prove delivery. Use `wayexpand-gui --picker --copy-only` and paste if this field cannot be replaced safely.",
            });
        }
        let verdict = IbusFieldVerdict::of(field);
        let specific = match verdict {
            IbusFieldVerdict::NoSurroundingText => Some((
                "field_no_surrounding_text",
                "The most recently observed IBus field does not provide surrounding text, so WayExpand refuses automatic replacement there.",
            )),
            IbusFieldVerdict::Sensitive => Some((
                "sensitive_field",
                "WayExpand intentionally suppresses expansion in the most recently observed sensitive IBus field.",
            )),
            IbusFieldVerdict::OutputEmittedUnverified => Some((
                "output_submitted_unverified",
                "WayExpand submitted replacement output in the most recently observed IBus field, but cannot confirm that the application changed its text.",
            )),
            IbusFieldVerdict::Mismatch => Some((
                "surrounding_text_mismatch",
                "The most recently observed IBus field's surrounding text did not match the trigger context WayExpand expected.",
            )),
            IbusFieldVerdict::Policy => Some((
                "replacement_blocked_by_policy",
                "Policy blocked replacement in the most recently observed IBus field.",
            )),
            IbusFieldVerdict::NoContentType | IbusFieldVerdict::Idle => Some((
                "field_input_not_observed",
                "The most recently observed IBus field has not supplied enough normal-field input to diagnose expansion.",
            )),
            IbusFieldVerdict::ReplacementContextObserved => Some((
                "replacement_context_observed",
                "The most recently observed field provides replacement context, but this does not prove the trigger matched or the application changed its text.",
            )),
        };
        if let Some((state, answer)) = specific {
            return Some(InputMethodGuidance {
                state,
                answer,
                action: ibus_field_action(verdict, field.latest_match),
            });
        }
    }
    Some(InputMethodGuidance {
        state: "input_observed_delivery_unverified",
        answer: "WayExpand observed input from a normal IBus field in this session, but telemetry does not identify the affected application. This does not prove a trigger matched or that the application applied replacement text.",
        action: "Run a harmless trigger once in the affected application and inspect the visible text. A matcher preview checks only the pattern; use the real-application test in Getting started to verify delivery. Do not retry output whose result is uncertain. For a reliable manual path, use `wayexpand-gui --picker --copy-only` and paste.",
    })
}

fn report_json(
    daemon: Option<&DaemonRoute>,
    ibus_field: Option<&IbusFieldEvidence>,
    now: u64,
    daemon_status_schema: Option<u32>,
    daemon_status_compatible: Option<bool>,
    input_method: Option<&InputMethodGuidance>,
) -> serde_json::Value {
    let daemon_json = daemon.map(|route| {
        let diagnosis = &route.diagnosis;
        let verdict = diagnosis.verdict(now);
        let scope_guidance = terminal_scope_guidance(route);
        serde_json::json!({
            "source": route.source,
            "backend": route.backend,
        "verdict": scope_guidance.as_ref().map_or_else(
            || {
                if route.enabled_expansions == Some(0)
                    && verdict == Verdict::NothingMatched
                {
                    "no_enabled_expansions"
                } else {
                    verdict.code()
                }
            },
            |guidance| guidance.state,
        ),
            "matching_block": scope_guidance.as_ref().map(|guidance| guidance.state).or_else(|| diagnosis.matching_block.map(MatchingBlock::code)),
            "matching_unavailable_reason": route.matching_unavailable_reason.as_deref(),
            "capture_app_scope_state": route.capture_app_scope_state.as_deref(),
            "raw_capture_fallback_enabled": route.raw_capture_fallback_enabled,
            "raw_capture_app_count": route.raw_capture_app_count,
            "window_process_identity_state": route.window_process_identity_state.as_deref(),
            "last_outcome": match verdict {
                Verdict::LastStopped(kind) => Some(kind.code()),
                Verdict::LastExpanded => diagnosis.recent.first().map(|outcome| outcome.kind.code()),
                _ => None,
            },
            "terminal_apps_only": diagnosis.terminal_apps_only,
            "terminal_paste_last_failure": route.terminal_paste_last_failure,
            "input_events_total": diagnosis.input_events,
            "matches_found_total": diagnosis.matches_found,
            "stopped_total": diagnosis.stopped_total,
            "recent": diagnosis.recent.iter().map(|outcome| serde_json::json!({
                "at_unix": outcome.at_unix,
                "outcome": outcome.kind.code(),
                "completion": outcome.completion,
                "pattern_match": recent_pattern_match_status(outcome.kind),
                "delivery_status": recent_delivery_status(outcome.kind),
                "application_delivery_verified": false,
            })).collect::<Vec<_>>(),
        })
    });
    let ibus = ibus_field.map(|field| {
        let recent_typed = field.recent_typed.as_ref();
        let focus_state = recent_typed.map_or("none", |field| {
            if field.focused { "current" } else { "changed_since_input" }
        });
        let (pattern_match, delivery_status) = recent_typed
            .map(ibus_stage_status)
            .unwrap_or(("unknown", "not_attempted"));
        serde_json::json!({
            "verdict": recent_typed.map(|field| IbusFieldVerdict::of(field).label()),
            "field_focus_state": focus_state,
            "target_application_identified": false,
            "next_action": recent_typed.map(|field| ibus_field_action(IbusFieldVerdict::of(field), field.latest_match))
                .or_else(|| Some(ibus_no_current_field_action(field.keys_seen))),
            "keys": recent_typed.map(|field| field.keys),
            "expansions": recent_typed.map(|field| field.expansions),
            "keys_seen_this_session": field.keys_seen,
            "pattern_match": pattern_match,
            "delivery_status": delivery_status,
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
    } else if let Some(guidance) = daemon.and_then(terminal_scope_guidance) {
        serde_json::json!({
            "state": guidance.state,
            "answer": guidance.answer,
            "next_action": guidance.action,
            "application_delivery_verified": false,
            "pattern_match": "blocked_by_focus_identity",
            "delivery_status": "not_attempted",
        })
    } else if let Some(guidance) = input_method {
        serde_json::json!({
            "state": guidance.state,
            "answer": guidance.answer,
            "next_action": guidance.action,
            "application_delivery_verified": false,
            "pattern_match": "not_evaluated",
            "delivery_status": "not_attempted",
        })
    } else if let Some(route) = daemon {
        let verdict = route.diagnosis.verdict(now);
        if route.enabled_expansions == Some(0) && verdict == Verdict::NothingMatched {
            serde_json::json!({
                "state": "no_enabled_expansions",
                "answer": "WayExpand received input, but the running configuration has no enabled keyboard expansions.",
                "next_action": "Open WayExpand's snippet library, enable a keyboard expansion or create a text expansion, save, and run `wayexpand reload`. Then type its trigger in the affected application. A matcher preview does not verify delivery.",
                "application_delivery_verified": false,
            })
        } else {
            let (answer, next_action) = verdict_text(
                &route.diagnosis,
                now,
                route.terminal_paste_last_failure.as_deref(),
                route.raw_capture_app_count.unwrap_or_default(),
            );
            serde_json::json!({
                "state": verdict.code(),
                "answer": answer,
                "next_action": next_action,
                "application_delivery_verified": false,
            })
        }
    } else if let Some(field) = ibus_field {
        match field.recent_typed.as_ref() {
            Some(field) => {
                let verdict = IbusFieldVerdict::of(field);
                let (pattern_match, delivery_status) = ibus_stage_status(field);
                serde_json::json!({
                    "state": verdict.label(),
                    "answer": if field.focused {
                        sentence(verdict.explanation())
                    } else {
                        format!("{} Focus changed since this input; WayExpand cannot identify the application from this telemetry.", sentence(verdict.explanation()))
                    },
                    "next_action": ibus_field_action(verdict, field.latest_match),
                    "pattern_match": pattern_match,
                    "delivery_status": delivery_status,
                    "application_delivery_verified": false,
                })
            }
            None if field.keys_seen > 0 => serde_json::json!({
                "state": "no_recent_typed_field",
                "answer": "IBus received typing in this login session, but has no recent field-level input evidence.",
                "next_action": ibus_no_current_field_action(field.keys_seen),
                "keys_seen_this_session": field.keys_seen,
                "pattern_match": "unknown",
                "delivery_status": "unknown",
                "application_delivery_verified": false,
            }),
            None => serde_json::json!({
                "state": "no_input_observed",
                "answer": "No typing has reached the IBus engine in this login session.",
                "next_action": ibus_no_current_field_action(field.keys_seen),
                "keys_seen_this_session": 0,
                "pattern_match": "no_input_observed",
                "delivery_status": "not_attempted",
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
    let (pattern_match, delivery_status) = if daemon.and_then(terminal_scope_guidance).is_some() {
        ("blocked_by_focus_identity", "not_attempted")
    } else if let Some(guidance) = input_method {
        input_method_stage_status(guidance, ibus_field)
    } else if let Some(route) = daemon {
        if route.enabled_expansions == Some(0)
            && route.diagnosis.verdict(now) == Verdict::NothingMatched
        {
            ("not_matched", "not_attempted")
        } else {
            daemon_stage_status(route.diagnosis.verdict(now))
        }
    } else if let Some(field) = ibus_field.and_then(|evidence| evidence.recent_typed.as_ref()) {
        ibus_stage_status(field)
    } else if ibus_field.is_some_and(|evidence| evidence.keys_seen == 0) {
        ("no_input_observed", "not_attempted")
    } else {
        ("unknown", "unknown")
    };
    if let Some(object) = diagnosis.as_object_mut() {
        object.insert(
            "pattern_match".into(),
            serde_json::Value::String(pattern_match.into()),
        );
        object.insert(
            "delivery_status".into(),
            serde_json::Value::String(delivery_status.into()),
        );
        object.insert(
            "matcher_preview_proves_application_delivery".into(),
            serde_json::Value::Bool(false),
        );
        if let Some(field) = ibus_field.and_then(|evidence| evidence.recent_typed.as_ref()) {
            object.insert(
                "ibus_field_focus_state".into(),
                serde_json::Value::String(
                    if field.focused {
                        "current"
                    } else {
                        "changed_since_input"
                    }
                    .into(),
                ),
            );
            if !field.focused {
                if let Some(answer) = object
                    .get("answer")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                {
                    object.insert(
                        "answer".into(),
                        serde_json::Value::String(format!(
                            "{answer} Focus changed since this input; the application is not identified by this telemetry."
                        )),
                    );
                }
            }
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

fn make_report_actions_runnable(report: &mut serde_json::Value) {
    for section in ["diagnosis", "desktop_input_route"] {
        let Some(action) = report
            .get_mut(section)
            .and_then(|section| section.get_mut("next_action"))
            .and_then(|action| action.as_str())
            .map(str::to_owned)
        else {
            continue;
        };
        if let Some(target) = report
            .get_mut(section)
            .and_then(|section| section.get_mut("next_action"))
        {
            *target = serde_json::Value::String(runnable_cli_suggestion(&action));
        }
    }
}

fn runnable_cli_suggestion(suggestion: &str) -> String {
    let cli = crate::commands::install::make_cli_command_runnable("wayexpand".into());
    let gui = crate::commands::install::make_cli_command_runnable("wayexpand-gui".into());
    suggestion
        .replace("`wayexpand-gui ", &format!("`{gui} "))
        .replace("`wayexpand-gui`", &format!("`{gui}`"))
        .replace("`wayexpand ", &format!("`{cli} "))
        .replace("`wayexpand`", &format!("`{cli}`"))
}

/// Point a build-mismatch diagnosis at the CLI that can actually inspect the
/// active payload. A checkout or stale CLI cannot safely tell the user to
/// apply a restart based on its own install identity.
fn daemon_build_recovery_action(state: &str, fallback: &str) -> String {
    let install = crate::doctor::status::cli_install_identity();
    let Some(active_cli) = install
        .active_user_cli
        .filter(|path| install.cli_matches_active_user_install == Some(false) && path.is_file())
    else {
        return runnable_cli_suggestion(fallback);
    };
    let cli = crate::commands::shell_single_quote(&active_cli.to_string_lossy());
    let check = format!("{cli} upgrade --check");
    let why = format!("{cli} why");
    if state == "restart_required" {
        format!(
            "Run `{check}` with the active installation, follow its reported action, then run `{why}` again."
        )
    } else {
        format!(
            "Run `{check}` with the active installation and resolve its build identity warning, then run `{why}` again."
        )
    }
}

fn print_report(
    daemon: Option<&DaemonRoute>,
    ibus_field: Option<&IbusFieldEvidence>,
    now: u64,
    daemon_status_schema: Option<u32>,
    daemon_status_compatible: Option<bool>,
    input_method: Option<&InputMethodGuidance>,
) {
    let scope_guidance = daemon.and_then(terminal_scope_guidance);
    if daemon_status_compatible == Some(false) {
        println!("Keyboard route");
        println!(
            "  Cannot diagnose: daemon status schema {} is incompatible with this CLI (expected {CONTROL_STATUS_SCHEMA}).",
            daemon_status_schema
                .map(|schema| schema.to_string())
                .unwrap_or_else(|| "unknown".into())
        );
        println!(
            "  Fix: {}",
            runnable_cli_suggestion(
                "run `wayexpand upgrade --check` before trusting route diagnostics."
            )
        );
        if ibus_field.is_some() {
            println!();
        } else {
            return;
        }
    }
    if daemon.is_none()
        && ibus_field.is_none()
        && input_method.is_none()
        && scope_guidance.is_none()
        && daemon_status_compatible != Some(false)
    {
        println!("No WayExpand typing route is running, so nothing can expand.");
        println!(
            "Next: {}",
            runnable_cli_suggestion("run `wayexpand setup`, then `wayexpand doctor`.")
        );
        println!(
            "For manual insertion meanwhile: open {} and paste the selected snippet.",
            runnable_cli_suggestion("`wayexpand-gui --picker --copy-only`")
        );
        return;
    }
    if let Some(guidance) = scope_guidance.as_ref().or(input_method) {
        let (pattern_match, delivery_status) = if scope_guidance.is_some() {
            ("blocked_by_focus_identity", "not_attempted")
        } else {
            input_method_stage_status(guidance, ibus_field)
        };
        println!(
            "Why expansion did not work: {}",
            guidance.state.replace('_', " ")
        );
        println!("  {}", guidance.answer);
        println!("  Next: {}", runnable_cli_suggestion(guidance.action));
        println!(
            "  Trigger match: {}. Text delivery: {}.",
            pattern_match_text(pattern_match),
            delivery_status_text(delivery_status)
        );
        println!("  A matcher preview checks the rule only; it does not verify insertion into this application.");
        println!();
    }
    // The prioritized route guidance above already includes focused-field
    // capability when the route is otherwise ready. Showing another field
    // action here would give users two competing things to do; the JSON keeps
    // the separate field evidence for diagnostics and automation.
    if input_method.is_none() && scope_guidance.is_none() {
        if let Some(field) = ibus_field {
            println!("Recent input through IBus");
            match field.recent_typed.as_ref() {
                Some(field) => {
                    let verdict = IbusFieldVerdict::of(field);
                    let (pattern_match, delivery_status) = ibus_stage_status(field);
                    println!(
                        "  Recent field totals: {} key(s), {} expansion(s).",
                        field.keys, field.expansions
                    );
                    if !field.focused {
                        println!("  Focus changed since this input; the application is not identified by this telemetry.");
                    }
                    println!(
                        "  Trigger match: {}. Text delivery: {}.",
                        pattern_match_text(pattern_match),
                        delivery_status_text(delivery_status)
                    );
                    println!("  {}", sentence(verdict.explanation()));
                    println!(
                        "  Next: {}",
                        runnable_cli_suggestion(ibus_field_action(verdict, field.latest_match))
                    );
                }
                None if field.keys_seen > 0 => {
                    println!("  IBus received typing in this login session, but has no recent field-level input evidence.");
                    println!(
                        "  Next: {}",
                        runnable_cli_suggestion(ibus_no_current_field_action(field.keys_seen))
                    );
                }
                None => {
                    println!("  No typing has reached the IBus engine in this login session.");
                    println!(
                        "  Next: {}",
                        runnable_cli_suggestion(ibus_no_current_field_action(0))
                    );
                }
            }
            println!();
        }
    }
    if let Some(route) = daemon {
        let diagnosis = &route.diagnosis;
        print!("Keyboard route ({} → {}", route.source, route.backend);
        if diagnosis.terminal_apps_only {
            if route.raw_capture_fallback_enabled
                && route.raw_capture_app_count.is_some_and(|count| count > 0)
            {
                print!(
                    ", terminal apps plus app-scoped raw capture ({} apps)",
                    route.raw_capture_app_count.unwrap_or_default()
                );
            } else {
                print!(", terminal apps only");
            }
        }
        println!(")");
        if input_method.is_some() || scope_guidance.is_some() {
            // The IBus finding is the primary diagnosis when the other route
            // is control-only or terminal-scoped. Keep its scope visible, but
            // do not present a second diagnosis/action for unrelated events.
            if scope_guidance.is_some() {
                println!("  This terminal-only route is blocked until focused-app identity is available.");
            } else {
                println!("  This daemon route does not cover browser or editor fields.");
            }
        } else {
            let empty_library = route.enabled_expansions == Some(0)
                && diagnosis.verdict(now) == Verdict::NothingMatched;
            let (answer, fix) = if empty_library {
                (
                    "WayExpand received input, but the running configuration has no enabled keyboard expansions.".to_owned(),
                    Some("Open WayExpand's snippet library, enable a keyboard expansion or create a text expansion, save, and run `wayexpand reload`. Then type its trigger in the affected application.".to_owned()),
                )
            } else {
                verdict_text(
                    diagnosis,
                    now,
                    route.terminal_paste_last_failure.as_deref(),
                    route.raw_capture_app_count.unwrap_or_default(),
                )
            };
            let (pattern_match, delivery_status) = if empty_library {
                ("not_matched", "not_attempted")
            } else {
                daemon_stage_status(diagnosis.verdict(now))
            };
            println!(
                "  Trigger match: {}. Text delivery: {}.",
                pattern_match_text(pattern_match),
                delivery_status_text(delivery_status)
            );
            println!("  {answer}");
            if let Some(fix) = fix {
                println!("  Fix: {}", runnable_cli_suggestion(&fix));
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
        }
        println!();
    }
    // A terminal-scoped evdev route and desktop-managed IBus are independent
    // capture paths. When the current terminal scope is unknown, that explains
    // terminal matching only; do not hide IBus health for a user investigating
    // a browser or editor field. Label the second diagnosis by route so the
    // two actions cannot be mistaken for competing explanations of one event.
    if scope_guidance.is_some() {
        if let Some(guidance) = input_method {
            println!("Other text fields (IBus route)");
            println!("  {}", guidance.answer);
            if ibus_field
                .and_then(|evidence| evidence.recent_typed.as_ref())
                .is_some_and(|field| !field.focused)
            {
                println!("  Focus changed since this input; the application is not identified by this telemetry.");
            }
            println!("  Next: {}", runnable_cli_suggestion(guidance.action));
            println!("  Trigger match: not established. Text delivery: not established.");
            println!("  A matcher preview checks the rule only; it does not verify insertion into the field.");
        }
    }
}

/// The answer for the keyboard route and, when there is one, what to do.
fn verdict_text(
    diagnosis: &ExpansionDiagnosis,
    now: u64,
    terminal_paste_failure: Option<&str>,
    raw_capture_app_count: usize,
) -> (String, Option<String>) {
    use MatchingBlock as B;
    let verdict = diagnosis.verdict(now);
    if matches!(verdict, Verdict::LastStopped(OutcomeKind::NotApplied)) {
        match terminal_paste_failure {
            Some("trigger_erase_focus_changed") => return (
                "WayExpand matched the trigger, but focus changed while it was trying to erase it for terminal paste. The field may be unchanged or partly changed.".into(),
                Some("Inspect the field before typing again; do not replay uncertain output. Restore focus to the intended terminal and run a harmless real-application test.".into()),
            ),
            Some("trigger_erase_focus_unconfirmed") => return (
                "WayExpand matched the trigger, but could not confirm the target before erasing it for terminal paste. The field may be unchanged or partly changed.".into(),
                Some("Inspect the field before typing again; do not replay uncertain output. Run `wayexpand doctor` to restore focus tracking, then use Quick Picker in copy-only mode meanwhile.".into()),
            ),
            Some("trigger_erase_input_interrupted") => return (
                "WayExpand matched the trigger, but new input interrupted its deletion before terminal paste. The field may be unchanged or partly changed.".into(),
                Some("Inspect the field before continuing; do not replay uncertain output. After input stops, run one harmless real-application test.".into()),
            ),
            Some("trigger_erase_output_cancelled") => return (
                "WayExpand matched the trigger, but cancellation stopped its deletion before terminal paste. The field may be unchanged or partly changed.".into(),
                Some("Inspect the field before continuing; do not replay uncertain output. Refresh diagnostics and retry only after the target state is clear.".into()),
            ),
            Some("trigger_erase_event_backlog") => return (
                "WayExpand matched the trigger, but the EIS keyboard event queue did not settle before terminal paste. Trigger deletion may be incomplete.".into(),
                Some("Inspect the field before continuing; do not replay uncertain output. Run `wayexpand doctor` and retest after the input route is stable.".into()),
            ),
            Some("trigger_erase_device_unavailable") => return (
                "WayExpand matched the trigger, but the compositor's EIS keyboard device became unavailable during terminal paste. Trigger deletion may be incomplete.".into(),
                Some("Inspect the field before continuing; do not replay uncertain output. Refresh `wayexpand doctor` and reconnect the desktop input route before testing again.".into()),
            ),
            Some("trigger_erase_disconnected") => return (
                "WayExpand matched the trigger, but the EIS connection disconnected during terminal paste. Trigger deletion may be incomplete.".into(),
                Some("Inspect the field before continuing; do not replay uncertain output. Reconnect the desktop input route with `wayexpand doctor`, then run one harmless real-application test.".into()),
            ),
            Some("trigger_erase_flush_failed") => return (
                "WayExpand matched the trigger, but the compositor did not acknowledge all keyboard events needed to erase it. The field may be unchanged or partly changed.".into(),
                Some("Inspect the field before continuing; do not replay uncertain output. Reconnect the output route and test once in a harmless field.".into()),
            ),
            Some("trigger_erase_other") => return (
                "WayExpand matched the trigger, but could not confirm trigger deletion before the terminal paste. The field may be unchanged or partly changed.".into(),
                Some("Inspect the field before typing again; do not replay uncertain output. Run `wayexpand doctor`, then use Quick Picker in copy-only mode for manual paste meanwhile.".into()),
            ),
            Some("focus_changed") => return (
                "WayExpand matched the trigger and sent its deletion, but the focused window changed before the terminal paste key. The replacement was not submitted.".into(),
                Some("Inspect the original field and the newly focused window before continuing; do not replay this output. Restore focus to the intended terminal and run one harmless real-application test.".into()),
            ),
            Some("focus_unconfirmed") => return (
                "WayExpand matched the trigger and sent its deletion, but could not verify the focused target before the terminal paste key. The replacement was not submitted.".into(),
                Some("Inspect the field before continuing; the trigger may be gone. Run `wayexpand doctor` to restore focus tracking, then use Quick Picker in copy-only mode meanwhile.".into()),
            ),
            Some("input_interrupted") => return (
                "WayExpand matched the trigger and sent its deletion, but new input interrupted the transaction before the terminal paste key. The replacement was not submitted.".into(),
                Some("Inspect the field before continuing; the trigger may be gone and the new input may be nearby. Do not replay this output. Run one harmless real-application test after input stops.".into()),
            ),
            Some("output_cancelled") => return (
                "WayExpand matched the trigger and sent its deletion, but the output transaction was cancelled before the terminal paste key. The replacement was not submitted.".into(),
                Some("Inspect the field before continuing; the trigger may be gone. Refresh diagnostics and do not replay this output.".into()),
            ),
            Some("safety_checkpoint_failed") => return (
                "WayExpand matched the trigger and sent its deletion, but a safety check stopped the terminal paste before replacement output was submitted.".into(),
                Some("Inspect the field before continuing; the trigger may be gone. Run `wayexpand doctor` and do not replay this output until the target state is clear.".into()),
            ),
            Some("paste_key_failed") => return (
                "WayExpand matched the trigger and sent its deletion, but could not confirm delivery of the terminal paste key. The application may or may not have received the replacement.".into(),
                Some("Inspect the field before continuing; do not retry uncertain output. Then verify that this terminal accepts Ctrl+Shift+V. Use Quick Picker in copy-only mode meanwhile.".into()),
            ),
            _ => {}
        }
    }
    match verdict {
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
            if diagnosis.terminal_apps_only && raw_capture_app_count > 0 {
                "No typing has reached this route since it started. It handles configured terminal apps and explicitly allowlisted raw-capture apps."
            } else if diagnosis.terminal_apps_only {
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
                if diagnosis.terminal_apps_only && raw_capture_app_count > 0 {
                    ". This route skips apps outside the configured terminal and raw-capture app lists"
                } else if diagnosis.terminal_apps_only {
                    ". This route also skips every app that is not one of your terminal apps"
                } else {
                    ""
                }
            ),
            Some("Replace `TRIGGER` with the exact trigger you typed, then run `wayexpand explain 'TRIGGER'` to check the pattern and app filter; this does not test desktop input or insertion. Then run `wayexpand debug --test` in the affected app and inspect the visible text.".to_owned()),
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
            Some("inspect the field; if the text is missing or wrong, run `wayexpand debug --test` for a harmless real-application test; do not retry uncertain output".to_owned()),
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
            Some("inspect the intended field and any field that may have received output; correct partial or misplaced text by hand, and do not retry until the result is clear"),
        ),
        K::Partial => (
            "The last expansion was only partly written.".into(),
            Some("inspect the intended field and any field that may have received output; correct partial or misplaced text by hand, and do not retry this expansion"),
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

fn ibus_field_action(
    verdict: IbusFieldVerdict,
    latest_match: telemetry::IbusLatestMatch,
) -> &'static str {
    use IbusFieldVerdict as V;
    if latest_match == telemetry::IbusLatestMatch::NoMatch {
        return "Replace `TRIGGER` with the exact trigger you typed, then run `wayexpand explain 'TRIGGER'` to check its enabled state, match mode, and application filters. Type it in the affected application and verify the visible replacement; a matcher result alone does not prove delivery. Use `wayexpand-gui --picker --copy-only` and paste if this field cannot be replaced safely.";
    }
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
            "Replace `TRIGGER` with the exact trigger you typed, then run `wayexpand explain 'TRIGGER'`; inspect the field before trying again."
        }
        V::Policy => {
            "Run `wayexpand policy check --json` and `wayexpand doctor --security`; ask the policy administrator before changing a restriction."
        }
        V::Idle => {
            "Focus this field, run `wayexpand ibus watch --seconds 30`, and type harmless text to check input delivery."
        }
    }
}

/// Report matching separately from delivery. IBus records a match indirectly
/// as either emitted output or a refusal that occurs only after a trigger
/// matched; an emitted signal still cannot prove that the application used it.
fn ibus_stage_status(field: &IbusFieldActivity) -> (&'static str, &'static str) {
    match field.latest_match {
        IbusLatestMatch::OutputSubmitted => return ("confirmed_recently", "submitted_unverified"),
        IbusLatestMatch::Refused => return ("confirmed_recently", "refused_before_output"),
        IbusLatestMatch::NoMatch => return ("no_match", "not_attempted"),
        IbusLatestMatch::Unknown => {}
    }
    if field.expansions > 0 {
        // Status created by older IBus builds has only cumulative counters;
        // preserve that evidence without claiming it describes the latest key.
        ("confirmed_in_field_history", "submitted_unverified")
    } else if field.last_refusal.is_some() {
        ("confirmed_in_field_history", "refused_before_output")
    } else if field.keys > 0
        && field.content_type == wayexpand_backend_ibus::telemetry::IbusContentTypeState::Normal
    {
        ("not_observed_recently", "not_attempted")
    } else if field.keys > 0 {
        ("not_evaluated", "not_attempted")
    } else {
        ("not_observed", "not_attempted")
    }
}

fn daemon_stage_status(verdict: Verdict) -> (&'static str, &'static str) {
    match verdict {
        Verdict::LastStopped(OutcomeKind::Partial) => {
            ("confirmed_recently", "partial_or_uncertain")
        }
        Verdict::LastStopped(OutcomeKind::Unknown) => ("confirmed_recently", "uncertain"),
        Verdict::LastStopped(OutcomeKind::NotApplied) => ("confirmed_recently", "not_submitted"),
        Verdict::LastStopped(OutcomeKind::BlockedByPolicy) => {
            ("confirmed_recently", "blocked_by_policy")
        }
        Verdict::LastStopped(OutcomeKind::InterruptedByTyping) => {
            ("confirmed_recently", "stopped_before_output")
        }
        Verdict::LastStopped(OutcomeKind::SnippetActionFailed) => {
            ("confirmed_recently", "snippet_action_failed")
        }
        Verdict::LastStopped(_) => ("confirmed_recently", "unknown"),
        Verdict::LastExpanded => ("confirmed_recently", "submitted_unverified"),
        Verdict::NothingMatched => ("not_observed_recently", "not_attempted"),
        Verdict::NoTypingSeen => ("no_input_observed", "not_attempted"),
        Verdict::MatchingOff(_) => ("blocked", "not_attempted"),
        Verdict::InputStatusUnknown => ("unknown", "unknown"),
    }
}

/// Per-transaction evidence for JSON consumers. A recorded output completion
/// necessarily follows a recognized trigger, but never proves the target
/// application applied the text.
fn recent_pattern_match_status(kind: OutcomeKind) -> &'static str {
    match kind {
        OutcomeKind::Other => "unknown",
        _ => "confirmed_for_transaction",
    }
}

fn recent_delivery_status(kind: OutcomeKind) -> &'static str {
    match kind {
        OutcomeKind::Expanded | OutcomeKind::ExpandedCursorMisplaced => "submitted_unverified",
        OutcomeKind::NotApplied => "not_submitted",
        OutcomeKind::Unknown => "uncertain",
        OutcomeKind::Partial => "partial_or_uncertain",
        OutcomeKind::BlockedByPolicy => "blocked_by_policy",
        OutcomeKind::InterruptedByTyping => "stopped_before_output",
        OutcomeKind::SnippetActionFailed => "snippet_action_failed",
        OutcomeKind::Other => "unknown",
    }
}

fn input_method_stage_status(
    guidance: &InputMethodGuidance,
    field: Option<&IbusFieldEvidence>,
) -> (&'static str, &'static str) {
    let field_specific = matches!(
        guidance.state,
        "trigger_not_matched"
            | "field_no_surrounding_text"
            | "sensitive_field"
            | "output_submitted_unverified"
            | "surrounding_text_mismatch"
            | "replacement_blocked_by_policy"
            | "replacement_not_allowed"
            | "replacement_consent_unknown"
            | "field_input_not_observed"
            | "replacement_context_observed"
    );
    if field_specific {
        field
            .and_then(|evidence| evidence.recent_typed.as_ref())
            .map(ibus_stage_status)
            .unwrap_or(("unknown", "unknown"))
    } else {
        ("not_evaluated", "not_attempted")
    }
}

fn pattern_match_text(status: &str) -> &'static str {
    match status {
        "confirmed_recently" => "confirmed recently",
        "confirmed_in_field_history" => "confirmed earlier in this field",
        "no_match" => "no trigger matched the latest input",
        "not_observed_recently" => "not observed recently",
        "no_input_observed" => "no input observed",
        "blocked" => "blocked before matching",
        "blocked_by_focus_identity" => "blocked because focused-app identity is unknown",
        "not_evaluated" => "not evaluated",
        "not_observed" => "not observed",
        _ => "unknown",
    }
}

fn delivery_status_text(status: &str) -> &'static str {
    match status {
        "submitted_unverified" => "submitted, application result unverified",
        "refused_before_output" => "refused before output",
        "partial_or_uncertain" => "possibly partial; inspect the field and do not retry",
        "uncertain" => "unknown; inspect the field and do not retry",
        "not_submitted" => "not submitted",
        "blocked_by_policy" => "blocked by policy",
        "stopped_before_output" => "stopped before output",
        "snippet_action_failed" => "snippet action failed before delivery",
        "not_attempted" => "not attempted",
        _ => "unknown",
    }
}

fn ibus_no_current_field_action(keys_seen: u64) -> &'static str {
    if keys_seen == 0 {
        "Check that WayExpand is the active input method with `wayexpand ibus status`, then type harmless text in the affected field while `wayexpand ibus watch --seconds 30` runs."
    } else {
        "Focus the affected field, type harmless text while `wayexpand ibus watch --seconds 30` runs, then rerun `wayexpand why`; earlier input from another field does not diagnose this one."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnosis(body: &str) -> ExpansionDiagnosis {
        ExpansionDiagnosis::from_status(&DaemonStatus::parse(&format!("running\n{body}")))
    }

    fn scoped_daemon_route(capture_app_scope_state: Option<&str>) -> DaemonRoute {
        DaemonRoute {
            source: "evdev".into(),
            backend: "libei".into(),
            diagnosis: diagnosis("terminal_apps_only=true\ninput_events_total=0\n"),
            capture_app_scope_state: capture_app_scope_state.map(str::to_owned),
            raw_capture_fallback_enabled: true,
            raw_capture_app_count: Some(15),
            matching_unavailable_reason: None,
            window_process_identity_state: Some("compositor-application-id".into()),
            terminal_paste_last_failure: None,
            enabled_expansions: Some(282),
        }
    }

    #[test]
    fn allowlisted_raw_capture_focus_is_diagnosed_as_the_active_route() {
        let route = scoped_daemon_route(Some("allowed"));
        assert!(!route_is_limited_for_focused_app(&route));
        assert!(!ibus_route_relevant(Some(&route)));
        assert!(terminal_scope_guidance(&route).is_none());
    }

    #[test]
    fn unknown_or_denied_terminal_focus_keeps_ibus_as_a_possible_route() {
        for scope in [None, Some("unknown"), Some("outside_allowlist")] {
            let route = scoped_daemon_route(scope);
            assert!(route_is_limited_for_focused_app(&route));
            assert!(ibus_route_relevant(Some(&route)));
        }
    }

    #[test]
    fn no_input_guidance_names_configured_raw_capture_apps() {
        let route = scoped_daemon_route(Some("allowed"));
        let (answer, _) = verdict_text(
            &route.diagnosis,
            1,
            None,
            route.raw_capture_app_count.unwrap_or_default(),
        );
        assert!(answer.contains("explicitly allowlisted raw-capture apps"));
        assert!(!answer.contains("only handles your configured terminal apps"));
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
            let (answer, fix) = verdict_text(&diagnosis(body), 1, None, 0);
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
            capture_app_scope_state: None,
            raw_capture_fallback_enabled: false,
            raw_capture_app_count: None,
            matching_unavailable_reason: None,
            window_process_identity_state: None,
            terminal_paste_last_failure: None,
            enabled_expansions: None,
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
        let no_field = report_json(
            None,
            Some(&IbusFieldEvidence {
                recent_typed: None,
                keys_seen: 0,
            }),
            5,
            None,
            None,
            None,
        );
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
        let guidance = diagnose_input_method_route(&report, None, None, false).unwrap();
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

    #[test]
    fn kde_unsafe_shared_ibus_registration_precedes_transient_helper_warning() {
        let report = IbusChainReport {
            desktop: IbusDesktop::KdeWayland,
            checks: vec![
                wayexpand_backend_ibus::chain::IbusCheck {
                    id: "registered",
                    title: "WayExpand engine is registered in IBus",
                    state: IbusCheckState::Pass,
                    problem_id: None,
                    detail: "IBus registry lists WayExpand".into(),
                    fix: None,
                },
                wayexpand_backend_ibus::chain::IbusCheck {
                    id: "desktop",
                    title: "KWin IBus helper",
                    state: IbusCheckState::Warn,
                    problem_id: Some("WX-KWIN-IBUS-002"),
                    detail: "helper crashed recently".into(),
                    fix: None,
                },
                wayexpand_backend_ibus::chain::IbusCheck {
                    id: "manifest-scope",
                    title: "Shared registration is protected",
                    state: IbusCheckState::Fail,
                    problem_id: Some("WX-IBUS-MULTIUSER-001"),
                    detail: "shared manifest points into a user home".into(),
                    fix: None,
                },
            ],
            activity: None,
        };

        let guidance = diagnose_input_method_route(&report, None, None, false).unwrap();
        assert_eq!(guidance.state, "ibus_manifest_scope_invalid");
        assert!(guidance.answer.contains("not safe for multi-user use"));
        assert!(guidance.action.contains("--install-system-integration"));
        assert!(guidance.action.contains("administrator authorization"));

        let json = report_json(None, None, 0, None, None, Some(&guidance));
        assert_eq!(
            json["desktop_input_route"]["state"],
            "ibus_manifest_scope_invalid"
        );
        assert!(json["desktop_input_route"]["next_action"]
            .as_str()
            .unwrap()
            .contains("--install-system-integration"));
    }

    #[test]
    fn ibus_match_and_emitted_output_never_claim_verified_application_delivery() {
        let field = IbusFieldActivity {
            focused: true,
            content_type: telemetry::IbusContentTypeState::Normal,
            surrounding_capability: true,
            surrounding_nonempty: true,
            keys: 8,
            latest_match: IbusLatestMatch::OutputSubmitted,
            expansions: 1,
            ..IbusFieldActivity::default()
        };
        let evidence = IbusFieldEvidence {
            recent_typed: Some(field),
            keys_seen: 8,
        };
        let guidance = InputMethodGuidance {
            state: "output_submitted_unverified",
            answer: "WayExpand submitted replacement output, but cannot confirm the application changed its text.",
            action: "Inspect the field before continuing; if the text is wrong, do not replay it.",
        };

        let report = report_json(None, Some(&evidence), 0, None, None, Some(&guidance));

        assert_eq!(report["diagnosis"]["pattern_match"], "confirmed_recently");
        assert_eq!(
            report["diagnosis"]["delivery_status"],
            "submitted_unverified"
        );
        assert_eq!(report["diagnosis"]["application_delivery_verified"], false);
        assert_eq!(report["matcher_preview_proves_application_delivery"], false);
        assert!(report["diagnosis"]["next_action"].is_string());
    }
}
