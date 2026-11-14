//! Redacted, shareable support diagnostics.

use crate::commands::Args;
use crate::*;
use std::{
    io::Read,
    os::fd::AsRawFd,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const MAX_REPORT_BYTES: u64 = 1024 * 1024;
const REPORT_TIMEOUT: Duration = Duration::from_secs(10);

/// Produce a privacy-preserving support report from the same doctor and
/// certification reports operators can run independently. Every nested
/// structure is projected field-by-field; doctor JSON subtrees are never
/// copied wholesale.
pub(crate) fn support_bundle(args: Args) -> Result<()> {
    let mut output_path = None;
    let mut gnome = false;
    let mut args = args;
    while let Some(argument) = args.next() {
        if argument == "--gnome" && !gnome {
            gnome = true;
        } else if argument == "--output" && output_path.is_none() {
            output_path =
                Some(PathBuf::from(args.next().ok_or_else(|| {
                    usage_error("--output requires a new file path")
                })?));
        } else {
            usage_bail!("usage: wayexpand support [--gnome] [--output FILE]");
        }
    }

    let doctor = read_report("doctor")?;
    let certification = read_report("certify")?;
    let status = crate::doctor::status::read_daemon_status()
        .ok()
        .and_then(|raw| crate::doctor::status::status_as_json(&raw).ok());
    let services = service_health();
    let mut report = summarize_reports(&doctor, &certification, status.as_ref(), &services);
    if gnome {
        report["gnome"] = gnome_support_section(&doctor);
    }
    let bytes = serde_json::to_vec_pretty(&report).context("serializing support report")?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        bail!("support report exceeded the 1 MiB output limit");
    }

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

fn gnome_support_section(doctor: &serde_json::Value) -> serde_json::Value {
    use wayexpand_backend_ibus::chain::{IbusDaemonProcess, IbusManifest};

    let desktop = wayexpand_backend_ibus::chain::IbusDesktop::detect();
    let route = wayexpand_backend_ibus::chain::diagnose();
    let checks = [
        "gnome-session",
        "daemon",
        "component",
        "registered",
        "gnome-source-registered",
        "gnome-source-active",
        "consent",
        "field-input",
        "surrounding-text",
        "target-insertion",
    ];
    let source_setting = crate::setup_ibus::command_output(
        "gsettings",
        &["get", "org.gnome.desktop.input-sources", "sources"],
    );
    let parsed_sources = source_setting
        .as_deref()
        .and_then(|raw| wayexpand_backend_ibus::gnome_input_sources::parse_sources(raw).ok());
    let wayexpand_registered = parsed_sources.as_ref().is_some_and(|sources| {
        wayexpand_backend_ibus::gnome_input_sources::wayexpand_index(sources).is_some()
    });
    let active_source = wayexpand_backend_gnome_window::active_input_source().ok();
    // Use the same evidence contract as `wayexpand ibus status`: the source
    // must be registered and GNOME Shell's live InputSourceManager bridge
    // must report it active. Settings such as `current` and `mru-sources`
    // describe configuration/history; neither can prove activation.
    let wayexpand_is_active = support_activation_result(
        parsed_sources.as_ref().map(|sources| {
            wayexpand_backend_ibus::gnome_input_sources::wayexpand_index(sources).is_some()
        }),
        active_source.as_ref(),
    );
    let active_source_summary = active_source.as_ref().map(|(kind, id)| {
        if kind == "ibus" && id == "wayexpand" {
            "wayexpand"
        } else {
            "other"
        }
    });
    let source_summaries = parsed_sources.as_ref().map(|sources| {
        sources
            .iter()
            .map(|source| {
                let (kind, id) = match source.kind.as_str() {
                    "xkb" => ("xkb", Some("<redacted>".to_owned())),
                    "ibus" if source.id == "wayexpand" => ("ibus", Some("wayexpand".to_owned())),
                    "ibus" => ("ibus", Some("<other-engine>".to_owned())),
                    _ => ("other", Some("<redacted>".to_owned())),
                };
                serde_json::json!({"kind":kind, "id":id})
            })
            .collect::<Vec<_>>()
    });
    let component_dirs = IbusDaemonProcess::find()
        .map(|daemon| daemon.component_directories().0)
        .unwrap_or_else(|| vec![wayexpand_backend_ibus::chain::component_install_directory()]);
    let locations = component_dirs
        .iter()
        .map(|path| {
            if path == std::path::Path::new("/usr/share/ibus/component")
                || path == std::path::Path::new("/usr/local/share/ibus/component")
            {
                serde_json::json!({"kind":"system_component_directory","path":path})
            } else {
                serde_json::json!({"kind":"custom_component_directory","path":"<redacted>"})
            }
        })
        .collect::<Vec<_>>();
    let manifest = IbusManifest::find_in(&component_dirs);
    let current_engine = crate::setup_ibus::command_output_with_status("ibus", &["engine"])
        .and_then(|(_, value)| safe_engine_name(&value));
    let version = crate::setup_ibus::command_output("gnome-shell", &["--version"])
        .and_then(|value| value.split_whitespace().last().and_then(safe_version));
    let os = fs::read_to_string("/etc/os-release")
        .ok()
        .map(|content| {
            let get = |key: &str| {
                content
                    .lines()
                    .find_map(|line| line.strip_prefix(&format!("{key}=")))
                    .map(|value| value.trim_matches('"'))
                    .and_then(safe_engine_name)
            };
            serde_json::json!({"id": get("ID"), "version_id": get("VERSION_ID")})
        })
        .unwrap_or(serde_json::Value::Null);
    let check_map = checks
        .iter()
        .filter_map(|id| {
            route.check(id).map(|check| {
                (
                    (*id).to_owned(),
                    serde_json::json!({"state":check.state.label(), "problem_id":gnome_problem_for(check)}),
                )
            })
        })
        .collect::<serde_json::Map<_, _>>();
    let errors = check_map
        .values()
        .filter_map(|check| check["problem_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    serde_json::json!({
        "session": {
            "desktop": if desktop == wayexpand_backend_ibus::chain::IbusDesktop::Gnome { "gnome" } else { "other_or_unknown" },
            "wayland": std::env::var("XDG_SESSION_TYPE").ok().is_some_and(|value| value.eq_ignore_ascii_case("wayland")),
            "shell_version": version,
            "os": os,
        },
        "services": {
            "ibus_daemon_detected": check_map.get("daemon").is_some_and(|check| check["state"] == "pass"),
            "wayexpand_control": doctor["daemon"]["available"].as_bool(),
            "wayexpand_build": {"version": safe_token(doctor.get("wayexpand_version")), "commit": safe_token(doctor.get("wayexpand_commit"))},
        },
        "input_sources": {
            "readable": parsed_sources.is_some(),
            "sources": source_summaries,
            "wayexpand_registered": wayexpand_registered,
            "active_source": active_source_summary,
            "wayexpand_active": wayexpand_is_active,
        },
        "ibus": {
            "current_engine": current_engine,
            "component_directories": locations,
            "wayexpand_component_present": manifest.is_some(),
            "component_path": manifest.as_ref().map(|found| if found.path.starts_with("/usr/share/") || found.path.starts_with("/usr/local/share/") { "system_component" } else { "custom_path_redacted" }),
        },
        "policy": {
            "valid": doctor["policy"]["policy"]["valid"].as_bool(),
            "enforcement_enabled": doctor["policy"]["policy"]["safe_mode"].as_bool(),
            "replacement_authorized": check_map.get("consent").is_some_and(|check| check["state"] == "pass"),
        },
        "test_outcomes": check_map,
        "error_ids": errors,
        "privacy": {"keystrokes": "excluded", "snippet_contents": "excluded", "clipboard": "excluded", "environment_secrets": "excluded"},
    })
}

fn support_activation_result(
    registered: Option<bool>,
    live_source: Option<&(String, String)>,
) -> Option<bool> {
    match registered {
        Some(false) => Some(false),
        Some(true) => live_source.map(|(kind, id)| kind == "ibus" && id == "wayexpand"),
        None => None,
    }
}

fn gnome_problem_for(check: &wayexpand_backend_ibus::chain::IbusCheck) -> Option<&'static str> {
    use wayexpand_backend_ibus::chain::IbusCheckState;
    if matches!(check.state, IbusCheckState::Pass | IbusCheckState::Skip) {
        return None;
    }
    if check.id == "surrounding-text" && check.state != IbusCheckState::Fail {
        return check.problem_id;
    }
    Some(match check.id {
        "gnome-session" => "WX-GNOME-SESSION-001",
        "daemon" => "WX-GNOME-IBUS-001",
        "component" | "registered" => "WX-IBUS-001",
        "manifest-scope" => "WX-IBUS-MULTIUSER-001",
        "gnome-source-registered" => "WX-GNOME-IM-001",
        "gnome-source-active" => "WX-GNOME-IM-002",
        "consent" => "WX-POLICY-001",
        "field-input" => "WX-GNOME-IM-003",
        "surrounding-text" => "WX-IBUS-008",
        "target-insertion" => "WX-GNOME-IM-004",
        _ => check.problem_id.unwrap_or("WX-GNOME-IM-001"),
    })
}

fn safe_engine_name(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"');
    (!value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte)))
    .then(|| value.to_owned())
}

fn safe_version(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte)))
    .then(|| value.to_owned())
}

#[cfg(test)]
mod gnome_redaction_tests {
    use super::{safe_engine_name, safe_version, support_activation_result};

    #[test]
    fn gnome_support_tokens_reject_paths_and_unbounded_values() {
        assert_eq!(safe_engine_name("wayexpand"), Some("wayexpand".into()));
        assert!(safe_engine_name("/home/alice/custom-engine").is_none());
        assert!(safe_engine_name(&"x".repeat(81)).is_none());
        assert_eq!(safe_version("49.2"), Some("49.2".into()));
        assert!(safe_version("49.2 secret=token").is_none());
    }

    #[test]
    fn deprecated_current_value_cannot_confirm_gnome_activation() {
        let live_source_unavailable = support_activation_result(Some(true), None);
        assert_eq!(live_source_unavailable, None);

        // Registration plus an independently observed live Shell source is
        // required for PASS; the deprecated `current` readback is not input
        // to this decision.
        assert_eq!(
            support_activation_result(Some(true), Some(&("ibus".into(), "wayexpand".into()))),
            Some(true)
        );
        assert_eq!(
            support_activation_result(Some(true), Some(&("xkb".into(), "de".into()))),
            Some(false)
        );
    }
}

fn read_report(command_name: &str) -> Result<serde_json::Value> {
    let executable = std::env::current_exe().context("locating the WayExpand executable")?;
    let mut command = Command::new(executable);
    configure_process_group(&mut command);
    let mut child = command
        .args([command_name, "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting {command_name} diagnostics"))?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        bail!("could not capture {command_name} diagnostic output");
    };
    let mut supervisor = ChildSupervisor::new(child);
    let fd = stdout.as_raw_fd();
    // SAFETY: fcntl changes status flags on the owned stdout descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error())
            .context("making diagnostic output nonblocking");
    }
    let mut bytes = Vec::new();
    let deadline = Instant::now() + REPORT_TIMEOUT;
    let mut eof = false;
    let status = loop {
        if Instant::now() >= deadline {
            supervisor.kill_group();
            let _ = supervisor.reap();
            bail!("{command_name} diagnostics exceeded the time limit");
        }
        if !eof {
            let mut buffer = [0_u8; 8192];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => {
                        eof = true;
                        break;
                    }
                    Ok(count) => {
                        bytes.extend_from_slice(&buffer[..count]);
                        if bytes.len() as u64 > MAX_REPORT_BYTES {
                            supervisor.kill_group();
                            let _ = supervisor.reap();
                            bail!(
                                "{command_name} diagnostics exceeded the support-report size limit"
                            );
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        supervisor.kill_group();
                        let _ = supervisor.reap();
                        return Err(error).context("reading diagnostic output");
                    }
                }
            }
        }
        match supervisor.has_exited() {
            Ok(true) if eof => {
                supervisor.kill_group();
                break supervisor
                    .reap()
                    .context("waiting for diagnostic command")?;
            }
            Ok(_) => thread::sleep(Duration::from_millis(5)),
            Err(error) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return Err(error).context("checking diagnostic command status");
            }
        }
    };
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
    status: Option<&serde_json::Value>,
    services: &serde_json::Value,
) -> serde_json::Value {
    let backends = doctor["backends"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|backend| {
            serde_json::json!({
                "kind": safe_token(backend.get("kind")),
                "state": safe_token(backend.get("state")),
                "availability": safe_token(backend.get("availability")),
                "permission": safe_token(backend.get("permission")),
                "policy_allowed": copy(backend, "policy_allowed"),
            })
        })
        .collect::<Vec<_>>();

    let mut scenario_status_counts = serde_json::Map::new();
    for check in certification["checks"].as_array().into_iter().flatten() {
        let Some(status) = check["status"].as_str().filter(|status| {
            [
                "pass",
                "fail",
                "not-run",
                "unsupported",
                "warning",
                "skipped",
            ]
            .contains(status)
        }) else {
            continue;
        };
        let count = scenario_status_counts
            .entry(status.to_owned())
            .or_insert_with(|| serde_json::Value::from(0));
        *count = serde_json::Value::from(count.as_u64().unwrap_or(0) + 1);
    }

    let daemon = &doctor["daemon"];
    // IBus route: check identifiers and states only. Details can contain
    // home-directory paths, and fields are summarized as counts and verdicts.
    let ibus_route = &doctor["ibus"]["route"];
    let ibus_checks = ibus_route["checks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|check| {
            serde_json::json!({
                "id": safe_token(check.get("id")),
                "problem_id": safe_token(check.get("problem_id")),
                "state": safe_token(check.get("state")),
            })
        })
        .collect::<Vec<_>>();
    let ibus_fields = ibus_route["activity"]["recent_fields"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|field| {
            serde_json::json!({
                "verdict": safe_token(field.get("verdict")),
                "content_type": safe_token(field.get("content_type")),
                "surrounding_capability": copy(field, "surrounding_capability"),
                "surrounding_nonempty": copy(field, "surrounding_nonempty"),
                "keys": copy(field, "keys"),
                "expansions": copy(field, "expansions"),
                "refusals": refusal_counts(&field["refusals"]),
            })
        })
        .collect::<Vec<_>>();
    let route_plan = doctor["route_plan"]["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|route| serde_json::json!({
            "id": safe_token(route.get("id")),
            "standing": safe_token(route.get("standing")),
            "capture": safe_token(route.get("capture")),
            "injection": safe_token(route.get("injection")),
            "certification": safe_token(route.get("certification")),
            "sensitive_field_protection": route.get("sensitive_field_protection").and_then(serde_json::Value::as_bool),
            "assessment": {
                "installed": safe_token(route["assessment"].get("installed")),
                "supported": safe_token(route["assessment"].get("supported")),
                "authorized": safe_token(route["assessment"].get("authorized")),
                "connected": safe_token(route["assessment"].get("connected")),
                "policy_allowed": route["assessment"].get("policy_allowed").and_then(serde_json::Value::as_bool),
                "certified": route["assessment"].get("certified").and_then(serde_json::Value::as_bool),
                "requires_user_action": route["assessment"].get("requires_user_action").and_then(serde_json::Value::as_bool),
                "limitations": route["assessment"]["limitations"].as_array().into_iter().flatten().filter_map(|value| safe_token(Some(value))).collect::<Vec<_>>(),
                "portal": route["assessment"].get("portal").filter(|value| value.is_object()).map(|portal| serde_json::json!({
                    "provider_candidate": portal.get("provider_candidate").and_then(serde_json::Value::as_bool),
                    "implementation_discovered": safe_token(portal.get("implementation_discovered")),
                    "interface_available": safe_token(portal.get("interface_available")),
                    "authorization": safe_token(portal.get("authorization")),
                    "direct_eis_socket_configured": portal.get("direct_eis_socket_configured").and_then(serde_json::Value::as_bool),
                    "eis_connected": safe_token(portal.get("eis_connected")),
                    "injection_negotiated": safe_token(portal.get("injection_negotiated")),
                    "injection_verified": safe_token(portal.get("injection_verified")),
                })),
            },
        }))
        .collect::<Vec<_>>();
    let protocol_probes = [
        "input_method_v2",
        "wlroots_virtual_keyboard",
        "kwin_window_tracker",
    ]
    .into_iter()
    .filter_map(|name| {
        let probe = doctor.get("protocol_probes")?.get(name)?;
        Some((
            name.to_owned(),
            serde_json::json!({"status": safe_token(probe.get("status"))}),
        ))
    })
    .collect::<serde_json::Map<_, _>>();
    let recent_events = status.map_or_else(
        || serde_json::json!({"available": false}),
        |status| serde_json::json!({
            "available": true,
            "input_backend_restarts": number(status, "input_reconnect_total"),
            "output_backend_restarts": number(status, "output_reconnect_total"),
            "uncertain_injections": number(status, "output_uncertain_total"),
            "partial_injections": number(status, "partial_write_total"),
            "recent_output_acknowledgements": status.get("recent_expansion_outcomes").and_then(serde_json::Value::as_array)
                .map_or(0, |outcomes| outcomes.iter().filter(|item| {
                    match item.get("application_result").and_then(serde_json::Value::as_str) {
                        Some("unverified") => true,
                        Some(_) => false,
                        None => item.get("completion").and_then(serde_json::Value::as_str) == Some("applied"),
                    }
                }).count() as u64),
            "suppressed_expansions": number(status, "suppressed_expansions_total"),
        }),
    );
    let diagnostics = diagnostic_summaries(doctor, services);
    serde_json::json!({
        "schema": 4,
        "generated_at_unix": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "application": {
            "version": safe_token(doctor.get("wayexpand_version")),
            "commit": safe_token(doctor.get("wayexpand_commit")),
            "desktop": safe_desktop(copy(doctor, "desktop")),
            "wayland": doctor.get("wayland").and_then(serde_json::Value::as_bool),
            "healthy": copy(doctor, "healthy"),
        },
        "configuration": {
            "valid": copy(&doctor["config"], "valid"),
            "policy_valid": copy(&doctor["policy"]["policy"], "valid"),
            "enforcement_enabled": copy(&doctor["policy"]["policy"], "safe_mode"),
        },
        "selection": {
            "automatic": {
                "source": safe_token(doctor["automatic_selection"].get("source")),
                "backend": safe_token(doctor["automatic_selection"].get("backend")),
                "ready": copy(&doctor["automatic_selection"], "ready"),
            },
            "recommended": {
                "mode": copy(&doctor["setup_recommendation"], "mode"),
                "backend": safe_token(doctor["setup_recommendation"].get("backend")),
                "ready": copy(&doctor["setup_recommendation"], "ready"),
            },
            "route_plan": route_plan,
        },
        "daemon": {
            "available": copy(daemon, "available"),
            "compatible": copy(daemon, "compatible"),
            "commit_matches": copy(daemon, "commit_matches"),
            "state": safe_token(daemon.get("state")),
            "source": safe_token(daemon.get("source")),
            "backend": safe_token(daemon.get("backend")),
            "capabilities": {
                "exclusive_capture": daemon["capabilities"]["exclusive_capture"].as_bool(),
                "sensitive_field_detection": daemon["capabilities"]["sensitive_field_detection"].as_bool(),
                "composition_awareness": daemon["capabilities"]["composition_awareness"].as_bool(),
                "layout_awareness": daemon["capabilities"]["layout_awareness"].as_bool(),
                "atomic_replacement": daemon["capabilities"]["atomic_replacement"].as_bool(),
                "full_unicode": daemon["capabilities"]["full_unicode"].as_bool(),
            },
        },
        "backends": backends,
        "ibus": {
            "installed": copy(&doctor["ibus"], "installed"),
            "schema": copy(ibus_route, "schema"),
            "ready": copy(ibus_route, "ready"),
            "configuration_readiness": safe_token(ibus_route.get("configuration_readiness")),
            "operational_readiness": safe_token(ibus_route.get("operational_readiness")),
            "desktop": safe_desktop(copy(ibus_route, "desktop")),
            "first_failure": safe_token(ibus_route.get("first_failure")),
            "checks": ibus_checks,
            "other_route": {
                "scope": safe_token(ibus_route["other_route"].get("scope")),
                "state": safe_token(ibus_route["other_route"].get("state")),
            },
            "engine_live": copy(&ibus_route["activity"], "live"),
            "config_generation_diagnostics_available": copy(&ibus_route["activity"], "config_generation_diagnostics_available"),
            "config_generation": number(&ibus_route["activity"], "config_generation"),
            "config_generation_requested": number(&ibus_route["activity"], "config_generation_requested"),
            "config_generation_mismatches": number(&ibus_route["activity"], "config_generation_mismatches"),
            "config_reload_failures": number(&ibus_route["activity"], "config_reload_failures"),
            "config_reload_error": safe_token(ibus_route["activity"].get("config_reload_error")),
            "totals": {
                "focus_ins": number(&ibus_route["activity"]["totals"], "focus_ins"),
                "keys": number(&ibus_route["activity"]["totals"], "keys"),
                "expansions": number(&ibus_route["activity"]["totals"], "expansions"),
                "output_failures": number(&ibus_route["activity"]["totals"], "output_failures"),
                "refusals": refusal_counts(&ibus_route["activity"]["totals"]["refusals"]),
            },
            "recent_fields": ibus_fields,
        },
        "protocol_probes": protocol_probes,
        "services": services,
        "diagnostics": diagnostics,
        "recent_events": recent_events,
        "certification": {
            "certified": copy(certification, "certified"),
            "desktop": safe_desktop(copy(certification, "desktop")),
            "selected_mode": safe_token(certification.get("selected_mode")),
            "required_scenario_count": certification["required_scenarios"]
                .as_array().map_or(0, Vec::len),
            "check_status_counts": scenario_status_counts,
        },
    })
}

fn safe_token(value: Option<&serde_json::Value>) -> Option<&str> {
    value.and_then(serde_json::Value::as_str).filter(|value| {
        !value.is_empty()
            && value.len() <= 80
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    })
}

fn safe_desktop(value: serde_json::Value) -> serde_json::Value {
    let Some(value) = value.as_str() else {
        return serde_json::Value::Null;
    };
    let desktop = ["GNOME", "KDE Plasma", "Sway", "Hyprland", "Xfce", "COSMIC"]
        .into_iter()
        .find(|known| value.eq_ignore_ascii_case(known));
    desktop.map_or(serde_json::Value::Null, serde_json::Value::from)
}

fn number(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

fn refusal_counts(value: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "no_surrounding_capability": number(value, "no_surrounding_capability"),
        "surrounding_text_unknown": number(value, "surrounding_text_unknown"),
        "surrounding_text_empty": number(value, "surrounding_text_empty"),
        "surrounding_text_mismatch": number(value, "surrounding_text_mismatch"),
        "policy": number(value, "policy"),
    })
}

fn service_health() -> serde_json::Value {
    let units = crate::services::UserService::ALL.map(crate::services::UserService::unit);
    let mut args = vec![
        "--user",
        "show",
        "-p",
        "Id,ActiveState,Result,ExecMainCode,ExecMainStatus,NRestarts",
    ];
    args.extend(units);
    let output = crate::setup_ibus::command_output("systemctl", &args).unwrap_or_default();
    parse_service_health(&output)
}

fn parse_service_health(output: &str) -> serde_json::Value {
    let mut entries = Vec::new();
    for block in output.split("\n\n") {
        let field = |name: &str| {
            block
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
                .unwrap_or("")
        };
        let Some(service) = crate::services::UserService::ALL
            .into_iter()
            .find(|service| service.unit() == field("Id"))
        else {
            continue;
        };
        let state = field("ActiveState");
        let result = field("Result");
        let exit_code = field("ExecMainStatus").parse::<i32>().ok();
        let failed =
            state == "failed" || (exit_code.is_some_and(|code| code != 0) && !result.is_empty());
        let mut error_ids = Vec::new();
        if failed {
            error_ids.push(if exit_code.is_some_and(|code| code != 0) {
                "WX-SERVICE-003"
            } else {
                "WX-SERVICE-002"
            });
        }
        entries.push(serde_json::json!({
            "unit": service.unit(),
            "state": safe_systemd_state(state),
            "result": safe_systemd_result(result),
            "exit_code": exit_code,
            "restart_count": field("NRestarts").parse::<u64>().ok(),
            "error_ids": error_ids,
        }));
    }
    serde_json::json!({"available": !entries.is_empty(), "units": entries})
}

fn safe_systemd_state(value: &str) -> Option<&str> {
    [
        "active",
        "reloading",
        "inactive",
        "failed",
        "activating",
        "deactivating",
        "maintenance",
    ]
    .into_iter()
    .find(|known| *known == value)
}

fn safe_systemd_result(value: &str) -> Option<&str> {
    [
        "success",
        "resources",
        "timeout",
        "exit-code",
        "signal",
        "core-dump",
        "watchdog",
        "protocol",
        "start-limit-hit",
        "unhealthy",
        "oom-kill",
        "exec-condition",
        "condition",
    ]
    .into_iter()
    .find(|known| *known == value)
    .filter(|_| !value.is_empty())
}

fn diagnostic_summaries(
    doctor: &serde_json::Value,
    services: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let mut findings = Vec::new();
    if doctor["config"]["valid"].as_bool() == Some(false) {
        findings.push(serde_json::json!({"id":"WX-CONFIG-001","severity":"error","message":"Configuration is invalid or unreadable"}));
    }
    if doctor["daemon"]["available"].as_bool() == Some(false) {
        findings.push(serde_json::json!({"id":"WX-SERVICE-002","severity":"warning","message":"No WayExpand daemon responded"}));
    }
    let ibus_route = &doctor["ibus"]["route"];
    let gnome_problem = ibus_route["checks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|check| check["problem_id"] == "WX-GNOME-IM-001");
    if gnome_problem.is_some() {
        findings.push(serde_json::json!({
            "id":"WX-GNOME-IM-001",
            "severity":"warning",
            "message":"GNOME IBus session, source registration, or activation could not be verified"
        }));
    } else if ibus_route["ready"].as_bool() == Some(false) {
        findings.push(serde_json::json!({"id":"WX-IBUS-001","severity":"warning","message":"The IBus route is configured or observed, but end-to-end insertion is not confirmed"}));
    }
    if let Some(units) = services["units"].as_array() {
        for id in units
            .iter()
            .flat_map(|unit| unit["error_ids"].as_array().into_iter().flatten())
            .filter_map(serde_json::Value::as_str)
        {
            let (severity, message) = match id {
                "WX-SERVICE-002" => ("warning", "A WayExpand service is in a failed state"),
                "WX-SERVICE-003" => ("error", "A WayExpand service exited unsuccessfully"),
                _ => continue,
            };
            findings.push(serde_json::json!({"id":id,"severity":severity,"message":message}));
        }
    }
    findings
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
            "ibus": { "route": {
                "ready": false,
                "schema": 2,
                "configuration_readiness": "pass",
                "operational_readiness": "unknown",
                "checks": [{"id":"gnome-source-registered","state":"fail","problem_id":"WX-GNOME-IM-001","detail":"/home/alice/private setting"}]
            }},
            "automatic_selection": { "source": "stdin", "backend": "libei", "ready": false },
            "backends": [{
                "kind": "libei", "state": "Implemented", "availability": "Unknown",
                "permission": "NotApplicable", "detail": "token=private",
                "policy_allowed": true
            }],
            "protocol_probes": {
                "input_method_v2": {
                    "status": "unavailable",
                    "detail": "input-method-v2 manager and Wayland seat probe /home/alice/private"
                }
            },
            "route_plan": {"routes": [{
                "id":"ibus", "standing":"available", "capture":"input_method",
                "injection":"commit", "certification":"uncertified",
                "sensitive_field_protection":false, "reason":"/home/alice/private reason"
            }]},
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
        let services = serde_json::json!({"available":true,"units":[{"unit":"wayexpand.service","state":"failed","exit_code":1,"restart_count":3,"error_ids":["WX-SERVICE-003"]}]});
        let status = serde_json::json!({
            "input_reconnect_total":2,"output_reconnect_total":3,"output_uncertain_total":1,"partial_write_total":1,
            "suppressed_expansions_total":4,
            "recent_expansion_outcomes":[{"completion":"applied","application_result":"unverified","trigger":"private"}]
        });
        let summary = summarize_reports(&doctor, &certification, Some(&status), &services);
        let serialized = summary.to_string();
        for forbidden in [
            "/home/alice",
            "secret-policy",
            "private path",
            "token=private",
            ":secret",
            "private snippet",
            "/home/alice/private reason",
            "input-method-v2 manager and Wayland seat probe",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        assert_eq!(summary["ibus"]["configuration_readiness"], "pass");
        assert_eq!(summary["ibus"]["schema"], 2);
        assert_eq!(summary["ibus"]["operational_readiness"], "unknown");
        assert!(summary["ibus"]["checks"]
            .to_string()
            .contains("WX-GNOME-IM-001"));
        assert!(summary["diagnostics"]
            .to_string()
            .contains("WX-GNOME-IM-001"));
        assert_eq!(
            summary["certification"]["check_status_counts"]["not-run"],
            1
        );
        assert_eq!(summary["configuration"]["enforcement_enabled"], true);
        assert_eq!(
            summary["protocol_probes"]["input_method_v2"]["status"],
            "unavailable"
        );
        assert_eq!(summary["schema"], 4);
        assert_eq!(summary["services"]["units"][0]["restart_count"], 3);
        assert_eq!(summary["services"]["units"][0]["exit_code"], 1);
        assert_eq!(summary["recent_events"]["input_backend_restarts"], 2);
        assert_eq!(summary["recent_events"]["output_backend_restarts"], 3);
        assert_eq!(
            summary["recent_events"]["recent_output_acknowledgements"],
            1
        );
        assert!(summary["recent_events"]
            .get("recent_successful_expansions")
            .is_none());
        assert!(summary["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| { finding["id"] == "WX-SERVICE-003" }));
        assert!(summary["route_plan"]["routes"][0].get("reason").is_none());
        assert!(summary["protocol_probes"]["input_method_v2"]
            .get("detail")
            .is_none());
    }

    #[test]
    fn service_history_parses_only_known_systemd_fields() {
        let parsed = super::parse_service_health(
            "Id=wayexpand-control.service\nActiveState=failed\nResult=exit-code\nExecMainStatus=7\nNRestarts=12\n",
        );
        assert_eq!(parsed["units"][0]["exit_code"], 7);
        assert_eq!(parsed["units"][0]["restart_count"], 12);
        assert_eq!(parsed["units"][0]["error_ids"][0], "WX-SERVICE-003");
        assert!(parsed.to_string().find("ExecMainStatus").is_none());
        let malformed = super::parse_service_health(
            "Id=wayexpand-control.service\nActiveState=failed\nResult=some private error\nExecMainStatus=private path\nNRestarts=unknown\n",
        );
        assert!(malformed.to_string().find("private").is_none());
    }
}
