//! Redacted, shareable support diagnostics.

use crate::commands::Args;
use crate::*;
use std::{io::Read, process::Stdio};

const MAX_REPORT_BYTES: u64 = 1024 * 1024;

/// Produce a privacy-preserving summary from the same doctor and certification
/// reports operators can run independently. This is an explicit allowlist:
/// filesystem paths, detailed errors, policy contents, triggers, and snippet
/// replacements are never copied into the support report.
pub(crate) fn support_bundle(args: Args) -> Result<()> {
    let mut output_path = None;
    let mut args = args;
    while let Some(argument) = args.next() {
        if argument == "--output" && output_path.is_none() {
            output_path =
                Some(PathBuf::from(args.next().ok_or_else(|| {
                    usage_error("--output requires a new file path")
                })?));
        } else {
            usage_bail!("usage: wayexpand support-bundle [--output FILE]");
        }
    }

    let doctor = read_report("doctor")?;
    let certification = read_report("certify")?;
    let report = summarize_reports(&doctor, &certification);
    let bytes = serde_json::to_vec_pretty(&report).context("serializing support report")?;

    if let Some(path) = output_path {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("creating support report {}", path.display()))?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error).with_context(|| "writing support report".to_owned());
        }
        println!("Wrote redacted support report to {}", path.display());
    } else {
        println!("{}", String::from_utf8_lossy(&bytes));
    }
    Ok(())
}

fn read_report(command_name: &str) -> Result<serde_json::Value> {
    let executable = std::env::current_exe().context("locating the WayExpand executable")?;
    let mut child = Command::new(executable)
        .args([command_name, "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting {command_name} diagnostics"))?;
    let mut stdout = child.stdout.take().context("capturing diagnostic output")?;
    let mut bytes = Vec::new();
    stdout
        .by_ref()
        .take(MAX_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("reading diagnostic output")?;
    let status = child.wait().context("waiting for diagnostic command")?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        bail!("{command_name} diagnostics exceeded the support-report size limit");
    }
    // `doctor --json` intentionally exits unsuccessfully for unhealthy hosts,
    // but still emits the full machine-readable report. Certification also
    // emits a useful report when no live scenario has been run.
    if !status.success() && command_name != "doctor" {
        bail!("{command_name} diagnostics could not be collected");
    }
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {command_name} diagnostic output"))
}

fn copy(value: &serde_json::Value, key: &str) -> serde_json::Value {
    value.get(key).cloned().unwrap_or(serde_json::Value::Null)
}

pub(crate) fn summarize_reports(
    doctor: &serde_json::Value,
    certification: &serde_json::Value,
) -> serde_json::Value {
    let backends = doctor["backends"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|backend| {
            serde_json::json!({
                "kind": copy(backend, "kind"),
                "state": copy(backend, "state"),
                "availability": copy(backend, "availability"),
                "permission": copy(backend, "permission"),
                "policy_allowed": copy(backend, "policy_allowed"),
            })
        })
        .collect::<Vec<_>>();

    let mut scenario_status_counts = serde_json::Map::new();
    for check in certification["checks"].as_array().into_iter().flatten() {
        let Some(status) = check["status"].as_str() else {
            continue;
        };
        let count = scenario_status_counts
            .entry(status.to_owned())
            .or_insert_with(|| serde_json::Value::from(0));
        *count = serde_json::Value::from(count.as_u64().unwrap_or(0) + 1);
    }

    let daemon = &doctor["daemon"];
    serde_json::json!({
        "schema": 1,
        "generated_at_unix": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "application": {
            "version": copy(doctor, "wayexpand_version"),
            "commit": copy(doctor, "wayexpand_commit"),
            "desktop": copy(doctor, "desktop"),
            "wayland": copy(doctor, "wayland"),
            "healthy": copy(doctor, "healthy"),
        },
        "configuration": {
            "valid": copy(&doctor["config"], "valid"),
            "policy_valid": copy(&doctor["policy"]["policy"], "valid"),
            "enforcement_enabled": copy(&doctor["policy"]["policy"], "safe_mode"),
        },
        "selection": {
            "automatic": {
                "source": copy(&doctor["automatic_selection"], "source"),
                "backend": copy(&doctor["automatic_selection"], "backend"),
                "ready": copy(&doctor["automatic_selection"], "ready"),
            },
            "recommended": {
                "mode": copy(&doctor["setup_recommendation"], "mode"),
                "backend": copy(&doctor["setup_recommendation"], "backend"),
                "ready": copy(&doctor["setup_recommendation"], "ready"),
            },
        },
        "daemon": {
            "available": copy(daemon, "available"),
            "compatible": copy(daemon, "compatible"),
            "commit_matches": copy(daemon, "commit_matches"),
            "state": copy(daemon, "state"),
            "source": copy(daemon, "source"),
            "backend": copy(daemon, "backend"),
            "capabilities": copy(daemon, "capabilities"),
        },
        "backends": backends,
        "certification": {
            "certified": copy(certification, "certified"),
            "desktop": copy(certification, "desktop"),
            "selected_mode": copy(certification, "selected_mode"),
            "required_scenario_count": certification["required_scenarios"]
                .as_array().map_or(0, Vec::len),
            "check_status_counts": scenario_status_counts,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::summarize_reports;

    #[test]
    fn support_summary_excludes_paths_snippets_and_credentials() {
        let doctor = serde_json::json!({
            "wayexpand_version": "1.3.3-dev",
            "desktop": "KDE Plasma",
            "healthy": false,
            "config": { "valid": true, "path": "/home/alice/private.toml" },
            "policy": { "policy": { "valid": true, "safe_mode": true, "token": "secret-policy" } },
            "daemon": { "available": false, "error": "private path and credential" },
            "automatic_selection": { "source": "stdin", "backend": "libei", "ready": false },
            "backends": [{
                "kind": "libei", "state": "Implemented", "availability": "Unknown",
                "permission": "NotApplicable", "detail": "token=private",
                "policy_allowed": true
            }],
            "expansions": [{ "trigger": ":secret", "replacement": "private snippet" }]
        });
        let certification = serde_json::json!({
            "certified": false,
            "desktop": "KDE Plasma",
            "config_path": "/home/alice/private.toml",
            "selected_mode": "stdin",
            "required_scenarios": ["password-field"],
            "checks": [{ "name": "password-field", "status": "not-run", "detail": "private" }]
        });
        let summary = summarize_reports(&doctor, &certification);
        let serialized = summary.to_string();
        for forbidden in [
            "/home/alice",
            "secret-policy",
            "private path",
            "token=private",
            ":secret",
            "private snippet",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        assert_eq!(
            summary["certification"]["check_status_counts"]["not-run"],
            1
        );
        assert_eq!(summary["configuration"]["enforcement_enabled"], true);
    }
}
