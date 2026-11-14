//! `wayexpand ibus status|watch`: check the IBus route link by link and
//! explain, per focused field, why triggers do or do not expand.

use super::Args;
use crate::args::take_flag;
use crate::doctor::status::{read_daemon_status, status_as_json};
use crate::errors::{classified_error, CliErrorKind};
use crate::*;
use std::time::{Duration, Instant};
use wayexpand_backend_ibus::{
    chain::{self, IbusChainReport, IbusCheckState},
    telemetry::{self, IbusFieldActivity},
    IbusFieldVerdict,
};

const USAGE: &str = "usage: wayexpand ibus [status [--json] | watch [--seconds N]]";

pub(crate) fn ibus_command(mut args: Args) -> Result<()> {
    match args.next().as_deref() {
        None | Some("status") => status_command(args),
        Some("watch") => watch_command(args),
        Some(_) => usage_bail!("{USAGE}"),
    }
}

/// How another WayExpand capture route relates to IBus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RouteOverlap {
    /// No daemon, or a daemon that does not capture keys (stdin harness).
    None,
    /// A raw-input daemon limited to its configured app allowlist.
    AppScoped { source: String },
    /// The control-only daemon: quick insert and hotkeys, no capture.
    ControlOnly,
    /// A capturing daemon that expands everywhere: IBus and it both expand.
    Everywhere { source: String },
    /// A capturing daemon too old to report its scope.
    Unknown { source: String },
}

impl RouteOverlap {
    pub(crate) fn from_status(status: Option<&serde_json::Value>) -> Self {
        let Some(status) = status else {
            return Self::None;
        };
        let source = status["source"].as_str().unwrap_or("").to_owned();
        if source == "stdin" {
            return Self::ControlOnly;
        }
        if !matches!(source.as_str(), "evdev" | "input-method") {
            return Self::None;
        }
        match status["terminal_apps_only"].as_bool() {
            Some(true) => Self::AppScoped { source },
            Some(false) => Self::Everywhere { source },
            None => Self::Unknown { source },
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::AppScoped { .. } => "app-scoped",
            Self::ControlOnly => "control-only",
            Self::Everywhere { .. } => "everywhere",
            Self::Unknown { .. } => "unknown",
        }
    }

    /// A finding to show next to the IBus checks, if any.
    pub(crate) fn finding(
        &self,
        ibus_active: bool,
        control_installed: bool,
    ) -> Option<(IbusCheckState, String, Option<String>)> {
        match self {
            // Beside IBus, a WayExpand service still runs quick insert and
            // hotkeys; without one those features are silently missing.
            Self::None if ibus_active => Some((
                IbusCheckState::Warn,
                "no WayExpand service is running, so quick insert and hotkeys are unavailable (IBus expansion still works)".into(),
                Some(if control_installed {
                    "`systemctl --user enable --now wayexpand-control.service`, or rerun `wayexpand setup --backend ibus`".into()
                } else {
                    "reinstall WayExpand to get wayexpand-control.service, then rerun `wayexpand setup --backend ibus`".into()
                }),
            )),
            Self::None => None,
            Self::ControlOnly => Some((
                IbusCheckState::Pass,
                "wayexpand-control.service provides quick insert and hotkeys beside IBus".into(),
                None,
            )),
            Self::AppScoped { source } if ibus_active => Some((
                IbusCheckState::Pass,
                format!("the WayExpand daemon ({source} capture) is limited to its configured app allowlist, beside IBus"),
                None,
            )),
            Self::AppScoped { .. } => None,
            Self::Everywhere { source } if ibus_active => Some((
                IbusCheckState::Fail,
                format!("the WayExpand daemon ({source} capture) also expands in every app, so snippets expand twice where IBus works"),
                Some("limit it to terminals with `wayexpand setup --backend ibus --terminals`, or stop it: `systemctl --user disable --now wayexpand-evdev.service wayexpand-input-method.service`".into()),
            )),
            Self::Everywhere { .. } => None,
            Self::Unknown { source } => Some((
                IbusCheckState::Warn,
                format!("the WayExpand daemon ({source} capture) is too old to report whether it is limited to terminals"),
                Some("restart it after updating: `systemctl --user restart wayexpand-evdev.service`".into()),
            )),
        }
    }
}

fn control_installed() -> bool {
    crate::services::ServiceInventory::probe().installed(crate::services::UserService::Control)
}

fn daemon_status_json() -> Option<serde_json::Value> {
    read_daemon_status()
        .ok()
        .and_then(|response| status_as_json(&response).ok())
}

fn ibus_active(report: &IbusChainReport) -> bool {
    report
        .check("selected")
        .is_some_and(|check| check.state == IbusCheckState::Pass)
}

fn field_json(field: &IbusFieldActivity) -> serde_json::Value {
    let verdict = IbusFieldVerdict::of(field);
    let mut value = serde_json::to_value(field).unwrap_or(serde_json::Value::Null);
    if let Some(object) = value.as_object_mut() {
        object.insert("verdict".into(), verdict.label().into());
        object.insert("explanation".into(), verdict.explanation().into());
        object.insert("target_application_verified".into(), false.into());
        object.insert(
            "output_emitted_unverified".into(),
            (verdict == IbusFieldVerdict::OutputEmittedUnverified).into(),
        );
    }
    value
}

/// The JSON shape shared by `wayexpand ibus status --json`, `doctor --json`
/// and the support bundle. It contains no typed text.
pub(crate) fn report_json(report: &IbusChainReport, overlap: &RouteOverlap) -> serde_json::Value {
    let checks: Vec<serde_json::Value> = report
        .checks
        .iter()
        .map(|check| {
            serde_json::json!({
                "id": check.id,
                "title": check.title,
                "state": check.state.label(),
                "problem_id": check.problem_id,
                "detail": check.detail,
                "fix": check.fix,
            })
        })
        .collect();
    let activity = report.activity.as_ref().map(|reading| {
        serde_json::json!({
            "live": reading.live,
            "pid": reading.status.pid,
            "version": reading.status.version,
            "engines": reading.status.engines,
            "config_generation_diagnostics_available": reading.status.config_generation_diagnostics_available,
            "config_generation": reading.status.config_generation,
            "config_generation_requested": reading.status.config_generation_requested,
            "config_generation_mismatches": reading.status.config_generation_mismatches,
            "config_reload_failures": reading.status.config_reload_failures,
            "config_reload_error": reading.status.config_reload_error,
            "totals": reading.status.totals,
            "last_transaction_id": reading.status.last_transaction_id,
            "recent_outputs": reading.status.recent_outputs,
            "recent_fields": reading.status.recent_fields.iter().map(field_json).collect::<Vec<_>>(),
        })
    });
    let finding = overlap.finding(ibus_active(report), control_installed());
    serde_json::json!({
        "schema": 4,
        "ready": report.ready() && !matches!(finding, Some((IbusCheckState::Fail, _, _))),
        "configuration_readiness": report.configuration_readiness().label(),
        "operational_readiness": report.operational_readiness().label(),
        "desktop": report.desktop.label(),
        "checks": checks,
        "first_failure": report.first_failure().map(|check| check.id),
        "readiness": report.readiness(),
        "activity": activity,
        "other_route": {
            "scope": overlap.label(),
            "state": finding.as_ref().map(|(state, _, _)| state.label()),
            "detail": finding.as_ref().map(|(_, detail, _)| detail.clone()),
            "fix": finding.and_then(|(_, _, fix)| fix),
        },
    })
}

fn print_check(state: IbusCheckState, title: &str, detail: &str, fix: Option<&str>) {
    println!("  {} {title}: {detail}", state.symbol());
    if let Some(fix) = fix {
        if state != IbusCheckState::Pass {
            println!("      Fix: {fix}");
        }
    }
}

fn describe_field(field: &IbusFieldActivity) -> String {
    let verdict = IbusFieldVerdict::of(field);
    format!(
        "field #{}{}: {} keys, {} output emission(s), unverified — {}: {}",
        field.sequence,
        if field.focused { " (focused)" } else { "" },
        field.keys,
        field.expansions,
        verdict.label(),
        verdict.explanation()
    )
}

/// Print the checks in human form. Returns whether the route is ready.
pub(crate) fn print_report(report: &IbusChainReport, overlap: &RouteOverlap) -> bool {
    println!("IBus route ({}):", report.desktop.label());
    for check in &report.checks {
        print_check(
            check.state,
            check.title,
            &check.detail,
            check.fix.as_deref(),
        );
    }
    let finding = overlap.finding(ibus_active(report), control_installed());
    if let Some((state, detail, fix)) = &finding {
        print_check(
            *state,
            "No second route expands the same keys",
            detail,
            fix.as_deref(),
        );
    }
    let ready = report.ready() && !matches!(finding, Some((IbusCheckState::Fail, _, _)));
    let stages: Vec<String> = report
        .readiness()
        .stages()
        .iter()
        .map(|(stage, ok)| {
            format!(
                "{} {}",
                if *ok { "✓" } else { "✗" },
                stage.replace('_', " ")
            )
        })
        .collect();
    println!(
        "Readiness: configuration={} · operational={} · {}",
        report.configuration_readiness().label(),
        report.operational_readiness().label(),
        stages.join(" · ")
    );
    match &report.activity {
        Some(reading) if !reading.status.recent_fields.is_empty() => {
            println!("Recent fields (newest last; no text is recorded):");
            for field in &reading.status.recent_fields {
                println!("  {}", describe_field(field));
            }
        }
        _ => {}
    }
    if let Some(output) = report
        .activity
        .as_ref()
        .and_then(|reading| reading.status.recent_outputs.last())
    {
        println!(
            "Latest output: {} · {} expected characters · {} · target identity {} · tx {}",
            output.outcome,
            output.expected_characters,
            output.duration_us.map_or_else(
                || "duration unavailable".to_owned(),
                |duration| format!("{duration} µs")
            ),
            output.target_identity_state,
            output.transaction_id,
        );
        if output.outcome == "emitted-unverified" {
            println!(
                "  IBus signal emission is not confirmation that the application changed its text."
            );
        } else if output.outcome == "partial-uncertain" {
            println!("  The target may be partially modified. Inspect it; WayExpand will not retry automatically.");
        }
    }
    ready
}

fn status_command(mut args: Args) -> Result<()> {
    let mut remaining: Vec<String> = args.by_ref().collect();
    let json = take_flag(&mut remaining, "--json");
    if !remaining.is_empty() {
        usage_bail!("{USAGE}");
    }
    let report = chain::diagnose();
    let overlap = RouteOverlap::from_status(daemon_status_json().as_ref());
    let ready = if json {
        let value = report_json(&report, &overlap);
        println!("{}", serde_json::to_string_pretty(&value)?);
        value["ready"].as_bool().unwrap_or(false)
    } else {
        let ready = print_report(&report, &overlap);
        println!();
        if ready {
            println!("Result: OPERATIONALLY VERIFIED. The IBus engine has received input from a normal field.");
        } else if report.configuration_readiness() == IbusCheckState::Pass
            && report.operational_readiness() == IbusCheckState::Fail
        {
            println!("Result: CONFIGURED, OPERATIONAL FAILURE. Resolve the failed check above and rerun `wayexpand ibus status`.");
        } else if report.configuration_readiness() == IbusCheckState::Pass {
            println!(
                "Result: CONFIGURED, OPERATIONAL STATUS {}. Target-app insertion has not been independently confirmed.",
                report.operational_readiness().label().to_ascii_uppercase()
            );
        } else {
            println!(
                "Result: CONFIGURATION {} · OPERATIONAL STATUS {}. Review the checks above.",
                report
                    .configuration_readiness()
                    .label()
                    .to_ascii_uppercase(),
                report.operational_readiness().label().to_ascii_uppercase()
            );
        }
        ready
    };
    if !ready {
        return Err(classified_error(
            CliErrorKind::Operational,
            "the IBus route is not ready",
        ));
    }
    Ok(())
}

fn watch_command(mut args: Args) -> Result<()> {
    let mut seconds = 60_u64;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--seconds" => {
                seconds = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| (1..=3600).contains(value))
                    .ok_or_else(|| usage_error("--seconds needs a number from 1 to 3600"))?;
            }
            _ => usage_bail!("{USAGE}"),
        }
    }
    let Some(path) = telemetry::status_path() else {
        bail!("XDG_RUNTIME_DIR is not set; the IBus engine cannot publish its activity");
    };
    println!("Watching the WayExpand IBus engine for {seconds} s (Ctrl+C to stop).");
    println!("Switch to the app you want to check and type a few characters, including a trigger.");
    println!("Only counts are shown; no typed text is recorded.");
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut last_seen: Option<telemetry::IbusStatus> = None;
    let mut warned_missing = false;
    while Instant::now() < deadline {
        match telemetry::read_status_at(&path) {
            Some(reading) if reading.live => {
                warned_missing = false;
                for field in &reading.status.recent_fields {
                    let previous = last_seen.as_ref().and_then(|status| {
                        status
                            .recent_fields
                            .iter()
                            .find(|seen| seen.sequence == field.sequence)
                    });
                    let changed = previous.is_none_or(|previous| {
                        IbusFieldVerdict::of(previous) != IbusFieldVerdict::of(field)
                            || previous.expansions != field.expansions
                            || previous.focused != field.focused
                    });
                    if changed && field.keys > 0 {
                        println!("{}", describe_field(field));
                    }
                }
                last_seen = Some(reading.status);
            }
            _ if !warned_missing => {
                println!("The IBus engine is not running or not publishing activity; run `wayexpand ibus status`.");
                warned_missing = true;
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_global_capture_daemon_beside_ibus_is_a_double_expansion_failure() {
        let everywhere = RouteOverlap::from_status(Some(&serde_json::json!({
            "source": "evdev",
            "terminal_apps_only": false,
        })));
        assert_eq!(
            everywhere,
            RouteOverlap::Everywhere {
                source: "evdev".into()
            }
        );
        assert!(matches!(
            everywhere.finding(true, true),
            Some((IbusCheckState::Fail, _, Some(_)))
        ));
        // Without IBus active there is nothing to double up with.
        assert!(everywhere.finding(false, true).is_none());

        let terminals = RouteOverlap::from_status(Some(&serde_json::json!({
            "source": "evdev",
            "terminal_apps_only": true,
        })));
        assert_eq!(terminals.label(), "app-scoped");
        assert!(matches!(
            terminals.finding(true, true),
            Some((IbusCheckState::Pass, _, None))
        ));

        let old = RouteOverlap::from_status(Some(&serde_json::json!({ "source": "evdev" })));
        assert!(matches!(
            old.finding(true, true),
            Some((IbusCheckState::Warn, _, _))
        ));

        // The control-only runtime uses the no-capture stdin route for output
        // selection, but does not spawn a stdin reader.
        let control = RouteOverlap::from_status(Some(&serde_json::json!({ "source": "stdin" })));
        assert_eq!(control, RouteOverlap::ControlOnly);
        assert!(matches!(
            control.finding(true, true),
            Some((IbusCheckState::Pass, _, None))
        ));
        assert_eq!(RouteOverlap::from_status(None), RouteOverlap::None);
    }

    #[test]
    fn ibus_without_any_service_explains_missing_quick_insert() {
        let none = RouteOverlap::None;
        let (state, detail, fix) = none.finding(true, true).unwrap();
        assert_eq!(state, IbusCheckState::Warn);
        assert!(detail.contains("quick insert"));
        assert!(fix.unwrap().contains("wayexpand-control.service"));
        let (_, _, fix) = none.finding(true, false).unwrap();
        assert!(fix.unwrap().contains("reinstall"));
        // Without IBus there is nothing to report about a companion service.
        assert!(none.finding(false, true).is_none());
    }

    #[test]
    fn report_json_is_content_free_and_flags_the_first_failure() {
        let report = IbusChainReport {
            desktop: chain::IbusDesktop::KdeWayland,
            checks: vec![
                chain::IbusCheck {
                    id: "component",
                    title: "IBus can find the WayExpand component",
                    state: IbusCheckState::Fail,
                    problem_id: None,
                    detail: "missing".into(),
                    fix: Some("run setup".into()),
                },
                chain::IbusCheck {
                    id: "gnome-source-registered",
                    title: "WayExpand registered in GNOME",
                    state: IbusCheckState::Fail,
                    problem_id: Some("WX-GNOME-IM-001"),
                    detail: "missing".into(),
                    fix: Some("wayexpand repair --gnome-input-source".into()),
                },
            ],
            activity: None,
        };
        let value = report_json(&report, &RouteOverlap::None);
        assert_eq!(value["ready"], false);
        assert_eq!(value["schema"], 4);
        assert_eq!(value["first_failure"], "component");
        assert_eq!(value["checks"][0]["fix"], "run setup");
        assert_eq!(value["checks"][1]["problem_id"], "WX-GNOME-IM-001");
        assert_eq!(value["other_route"]["scope"], "none");
        assert_eq!(value["operational_readiness"], "unknown");
    }
}
