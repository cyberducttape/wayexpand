//! GNOME-specific IBus health conclusions from the shared route checks.

use anyhow::Result;
use wayexpand_backend_ibus::chain::{self, IbusCheck, IbusCheckState, IbusDesktop};

const CHECKS: [&str; 10] = [
    "gnome-session",
    "daemon",
    "component",
    "registered",
    "gnome-source-registered",
    "gnome-source-active",
    "consent",
    "engine",
    "field-input",
    "target-insertion",
];

pub(crate) fn run(json: bool) -> Result<()> {
    let report = chain::diagnose();
    let checks: Vec<&IbusCheck> = CHECKS.iter().filter_map(|id| report.check(id)).collect();
    let (conclusion, problem) = conclusion(report.desktop, &checks);
    if json {
        let value = serde_json::json!({
            "schema": 1,
            "command": "doctor --gnome",
            "desktop": if report.desktop == IbusDesktop::Gnome { "gnome" } else { "other_or_unknown" },
            "conclusion": conclusion,
            "healthy": conclusion == "operational_evidence_available",
            "problem_id": problem,
            "checks": checks.iter().map(|check| serde_json::json!({
                "id": check.id,
                "state": check.state.label(),
                "problem_id": problem_for(check),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        if conclusion != "operational_evidence_available" {
            anyhow::bail!(
                "GNOME readiness is {conclusion}; inspect the structured checks and problem_id"
            );
        }
        return Ok(());
    }

    println!("WayExpand GNOME Health");
    println!("Conclusion: {}", conclusion.replace('_', " "));
    for check in &checks {
        println!(
            "{} {} — {}",
            check.state.symbol(),
            check.title,
            check.state.label()
        );
    }
    if let Some(problem) = problem {
        println!("\nProblem: {problem}");
    }
    println!("\nSuggested diagnostic commands (content-free):");
    println!("  gsettings get org.gnome.desktop.input-sources sources");
    println!("  gdbus call --session --dest org.gnome.Shell --object-path /org/wayexpand/ShellWindowTracker --method org.wayexpand.ShellWindowTracker1.GetActiveInputSource");
    println!(
        "  gsettings get org.gnome.desktop.input-sources mru-sources  # persistence evidence only"
    );
    println!("  ibus list-engine --name-only");
    println!("  ibus engine");
    println!("  wayexpand ibus status --json");
    println!("\nA passing engine connection does not prove insertion in an application. Use `wayexpand debug --gnome --interactive` for a user-verified test.");
    if conclusion != "operational_evidence_available" {
        anyhow::bail!("GNOME readiness is {conclusion}; follow the reported problem and retest");
    }
    Ok(())
}

fn conclusion(desktop: IbusDesktop, checks: &[&IbusCheck]) -> (&'static str, Option<&'static str>) {
    if desktop != IbusDesktop::Gnome {
        return ("not_gnome_or_unknown", Some("WX-GNOME-SESSION-001"));
    }
    let get = |id: &str| {
        checks
            .iter()
            .find(|check| check.id == id)
            .map(|check| check.state)
    };
    for (id, problem) in [
        ("gnome-session", "WX-GNOME-SESSION-001"),
        ("daemon", "WX-GNOME-IBUS-001"),
        ("component", "WX-IBUS-001"),
        ("registered", "WX-IBUS-001"),
        ("gnome-source-registered", "WX-GNOME-IM-001"),
        ("gnome-source-active", "WX-GNOME-IM-002"),
        ("consent", "WX-POLICY-001"),
        ("engine", "WX-GNOME-IBUS-002"),
        ("field-input", "WX-GNOME-IM-003"),
        ("target-insertion", "WX-GNOME-IM-004"),
    ] {
        if get(id) != Some(IbusCheckState::Pass) {
            let state = get(id);
            return if state == Some(IbusCheckState::Fail) || state == Some(IbusCheckState::Warn) {
                ("incomplete", Some(problem))
            } else {
                ("unknown", Some(problem))
            };
        }
    }
    ("operational_evidence_available", None)
}

fn problem_for(check: &IbusCheck) -> Option<&'static str> {
    if matches!(check.state, IbusCheckState::Pass | IbusCheckState::Skip) {
        return None;
    }
    Some(match check.id {
        "gnome-session" => "WX-GNOME-SESSION-001",
        "daemon" => "WX-GNOME-IBUS-001",
        "component" | "registered" => "WX-IBUS-001",
        "gnome-source-registered" => "WX-GNOME-IM-001",
        "gnome-source-active" => "WX-GNOME-IM-002",
        "consent" => "WX-POLICY-001",
        "field-input" => "WX-GNOME-IM-003",
        "target-insertion" => "WX-GNOME-IM-004",
        _ => check.problem_id.unwrap_or("WX-GNOME-IM-001"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            conclusion(IbusDesktop::Gnome, &checks.iter().collect::<Vec<_>>()),
            ("unknown", Some("WX-GNOME-IM-001"))
        );
    }
}
