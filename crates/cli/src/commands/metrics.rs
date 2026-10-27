//! `wayexpand metrics`: content-free Prometheus text for node_exporter's
//! textfile collector.
//!
//! Every value comes from the daemon status, the upgrade check, or the merged
//! configuration's counts. Triggers, replacements, command lines, window
//! titles and paths never appear, so the file is safe to scrape centrally.

use super::Args;
use crate::*;
use std::{fmt::Write as _, os::unix::fs::OpenOptionsExt};

const USAGE: &str = "usage: wayexpand metrics [--output FILE] [config]";

/// Daemon status gauges exported as-is: (status field, metric suffix, help).
const STATUS_GAUGES: &[(&str, &str, &str)] = &[
    (
        "command_queue_depth",
        "command_queue_depth",
        "Commands waiting for a worker.",
    ),
    (
        "command_in_flight",
        "command_in_flight",
        "Commands currently running.",
    ),
    (
        "output_stalled_ms",
        "output_stalled_milliseconds",
        "How long the output backend has been stalled.",
    ),
    (
        "output_retiring_workers",
        "output_retiring_workers",
        "Output workers still shutting down after a failure.",
    ),
    (
        "memory_rss_kib",
        "memory_rss_kibibytes",
        "Daemon resident memory.",
    ),
    (
        "matcher_latency_p95_us",
        "matcher_latency_p95_microseconds",
        "95th percentile matcher latency over the recent window.",
    ),
    (
        "injection_latency_p95_us",
        "injection_latency_p95_microseconds",
        "95th percentile injection latency over the recent window.",
    ),
];

/// Daemon status booleans exported as 0/1: (status field, metric suffix, help).
const STATUS_FLAGS: &[(&str, &str, &str)] = &[
    ("paused", "paused", "Whether expansion is paused."),
    (
        "window_tracker_connected",
        "window_tracker_connected",
        "Whether the focused-window tracker is connected.",
    ),
    (
        "capture_sensitive_focus",
        "sensitive_field_detection",
        "Whether the capture route can detect password fields.",
    ),
];

/// Counts from the merged configuration, or why it could not be loaded.
pub(crate) enum ConfigMetrics {
    Valid {
        files_loaded: usize,
        expansions: usize,
        hotkeys: usize,
        policy_violations: usize,
    },
    Invalid,
}

pub(crate) fn metrics_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let output = take_option(&mut rest, "--output")?.map(PathBuf::from);
    if rest.len() > 1
        || rest
            .first()
            .is_some_and(|argument| argument.starts_with('-'))
    {
        usage_bail!("{USAGE}");
    }
    let config_path = rest
        .into_iter()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    // A stopped daemon or a broken configuration is a metric value, not a
    // command failure: the collector must keep reporting exactly then.
    let snapshot = crate::doctor::status::read_daemon_status()
        .and_then(|response| crate::doctor::status::status_as_json(&response))
        .ok();
    let upgrade = match &snapshot {
        Some(snapshot) => crate::doctor::status::upgrade_status_from_snapshot(snapshot),
        None => crate::doctor::status::upgrade_status_unavailable(),
    };
    let text = render_metrics(snapshot.as_ref(), &upgrade, &config_metrics(&config_path));
    match output {
        Some(path) => write_atomically(&path, &text),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

fn config_metrics(path: &Path) -> ConfigMetrics {
    let Ok(base) = Config::load(path) else {
        return ConfigMetrics::Invalid;
    };
    let Ok(policy) = load_policy() else {
        return ConfigMetrics::Invalid;
    };
    match FleetConfig::load_standard_with_base_and_policy(base, &policy) {
        Ok(fleet) => ConfigMetrics::Valid {
            files_loaded: fleet.stats.total_files_loaded,
            expansions: fleet.stats.total_expansions,
            hotkeys: fleet.stats.total_hotkeys,
            policy_violations: fleet.policy_violations.len(),
        },
        Err(_) => ConfigMetrics::Invalid,
    }
}

/// The textfile collector may read at any moment, so the file is replaced
/// by a rename from a sibling temporary file and is never seen half-written.
fn write_atomically(path: &Path, text: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| usage_error("--output requires a file path"))?;
    // The output directory may be shared with a collector or other local
    // processes. Never follow a planted symlink at the temporary path.
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).context("could not obtain randomness for metrics output")?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".{suffix}.tmp"));
    let temporary = path.with_file_name(temporary_name);
    let mut created_temporary = false;
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&temporary)?;
        created_temporary = true;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::File::open(parent)?.sync_all()
    })();
    if let Err(error) = result {
        if created_temporary {
            let _ = fs::remove_file(&temporary);
        }
        return Err(error)
            .with_context(|| format!("could not atomically write {}", path.display()));
    }
    Ok(())
}

/// Escape a label value as the Prometheus text format requires.
fn label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn metric(out: &mut String, name: &str, kind: &str, help: &str, samples: &[(String, f64)]) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
    for (labels, value) in samples {
        let _ = writeln!(out, "{name}{labels} {value}");
    }
}

fn flag(value: bool) -> f64 {
    if value {
        1.0
    } else {
        0.0
    }
}

pub(crate) fn render_metrics(
    snapshot: Option<&serde_json::Value>,
    upgrade: &serde_json::Value,
    config: &ConfigMetrics,
) -> String {
    let mut out = String::new();
    metric(
        &mut out,
        "wayexpand_build_info",
        "gauge",
        "Version and commit of the CLI that wrote these metrics.",
        &[(
            format!(
                "{{version=\"{}\",commit=\"{}\"}}",
                label(build_info::VERSION),
                label(build_info::COMMIT)
            ),
            1.0,
        )],
    );
    let running = snapshot.is_some_and(|status| status["response"] == "running");
    metric(
        &mut out,
        "wayexpand_daemon_up",
        "gauge",
        "Whether the daemon answered on its control socket.",
        &[(String::new(), flag(running))],
    );
    metric(
        &mut out,
        "wayexpand_daemon_build_matches",
        "gauge",
        "Whether the running daemon is the same build as this CLI.",
        &[(
            String::new(),
            flag(upgrade["commit_matches"].as_bool() == Some(true)),
        )],
    );
    if let Some(status) = snapshot.filter(|_| running) {
        let text = |key: &str| label(status[key].as_str().unwrap_or("unknown"));
        metric(
            &mut out,
            "wayexpand_daemon_route_info",
            "gauge",
            "The daemon's active capture and output route.",
            &[(
                format!(
                    "{{route_id=\"{}\",source=\"{}\",backend=\"{}\",state=\"{}\"}}",
                    text("route_id"),
                    text("source"),
                    text("backend"),
                    text("state")
                ),
                1.0,
            )],
        );
        metric(
            &mut out,
            "wayexpand_daemon_config_ok",
            "gauge",
            "Whether the daemon's last configuration load or reload succeeded.",
            &[(String::new(), flag(status["config_state"] == "ok"))],
        );
        metric(
            &mut out,
            "wayexpand_daemon_output_healthy",
            "gauge",
            "Whether the output backend reports itself healthy.",
            &[(String::new(), flag(status["output_health"] == "ok"))],
        );
        for (field, suffix, help) in STATUS_FLAGS {
            if let Some(value) = status[*field].as_bool() {
                metric(
                    &mut out,
                    &format!("wayexpand_daemon_{suffix}"),
                    "gauge",
                    help,
                    &[(String::new(), flag(value))],
                );
            }
        }
        for (field, suffix, help) in STATUS_GAUGES {
            if let Some(value) = number(&status[*field]) {
                metric(
                    &mut out,
                    &format!("wayexpand_daemon_{suffix}"),
                    "gauge",
                    help,
                    &[(String::new(), value)],
                );
            }
        }
        // Every `*_total` status field is a monotonic counter; exporting
        // them generically keeps new daemon counters visible without a CLI
        // change. The field names are part of the status contract.
        if let Some(fields) = status.as_object() {
            for (field, value) in fields {
                let Some(value) = number(value) else { continue };
                if field.ends_with("_total") && is_metric_name(field) {
                    metric(
                        &mut out,
                        &format!("wayexpand_daemon_{field}"),
                        "counter",
                        &format!("Daemon status counter {field}."),
                        &[(String::new(), value)],
                    );
                }
            }
        }
    }
    let (valid, counts) = match config {
        ConfigMetrics::Valid {
            files_loaded,
            expansions,
            hotkeys,
            policy_violations,
        } => (
            true,
            Some([
                (
                    "files_loaded",
                    "Configuration files merged, including fleet layers.",
                    *files_loaded,
                ),
                (
                    "expansions",
                    "Expansions in the merged configuration.",
                    *expansions,
                ),
                ("hotkeys", "Hotkeys in the merged configuration.", *hotkeys),
                (
                    "policy_violations",
                    "Organization policy violations in the configuration.",
                    *policy_violations,
                ),
            ]),
        ),
        ConfigMetrics::Invalid => (false, None),
    };
    metric(
        &mut out,
        "wayexpand_config_valid",
        "gauge",
        "Whether the merged configuration on disk loads and validates.",
        &[(String::new(), flag(valid))],
    );
    for (suffix, help, value) in counts.into_iter().flatten() {
        metric(
            &mut out,
            &format!("wayexpand_config_{suffix}"),
            "gauge",
            help,
            &[(String::new(), value as f64)],
        );
    }
    out
}

/// Status values are typed numbers or, for older daemons, numeric strings.
fn number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .filter(|number: &f64| number.is_finite())
}

fn is_metric_name(name: &str) -> bool {
    name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn atomic_output_replaces_symlink_without_writing_through_it() {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).unwrap();
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let directory = std::env::temp_dir().join(format!("wayexpand-metrics-{suffix}"));
        fs::create_dir(&directory).unwrap();
        let target = directory.join("target");
        let output = directory.join("metrics.prom");
        fs::write(&target, "keep me").unwrap();
        std::os::unix::fs::symlink(&target, &output).unwrap();

        write_atomically(&output, "wayexpand_daemon_up 1\n").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "keep me");
        assert_eq!(
            fs::read_to_string(&output).unwrap(),
            "wayexpand_daemon_up 1\n"
        );
        assert_eq!(
            fs::symlink_metadata(&output).unwrap().permissions().mode() & 0o600,
            0o600
        );
        assert_eq!(
            fs::symlink_metadata(&output).unwrap().permissions().mode() & 0o022,
            0
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
