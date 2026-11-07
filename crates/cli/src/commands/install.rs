//! Installation readiness and narrowly scoped, non-consent repairs.
use super::Args;
use anyhow::{bail, Result};
use serde::Serialize;
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize)]
pub(crate) struct Check {
    pub(crate) component: &'static str,
    pub(crate) status: &'static str,
    pub(crate) path: String,
    pub(crate) detail: &'static str,
}

fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}
fn resolve_xdg_home(value: Option<&std::ffi::OsStr>, fallback: PathBuf) -> PathBuf {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|value| value.is_absolute())
        .unwrap_or(fallback)
}
fn xdg_home_is_valid(value: Option<&std::ffi::OsStr>) -> bool {
    value
        .filter(|value| !value.is_empty())
        .is_none_or(|value| Path::new(value).is_absolute())
}
fn data_home() -> PathBuf {
    resolve_xdg_home(
        env::var_os("XDG_DATA_HOME").as_deref(),
        home().join(".local/share"),
    )
}
fn data_home_is_valid() -> bool {
    xdg_home_is_valid(env::var_os("XDG_DATA_HOME").as_deref())
}
fn config_home() -> PathBuf {
    resolve_xdg_home(
        env::var_os("XDG_CONFIG_HOME").as_deref(),
        home().join(".config"),
    )
}
fn config_home_is_valid() -> bool {
    xdg_home_is_valid(env::var_os("XDG_CONFIG_HOME").as_deref())
}
fn current() -> PathBuf {
    home().join(".local/lib/wayexpand/current")
}

fn component_contract_path(component: &str, field: &str) -> Option<PathBuf> {
    let contract: toml::Value =
        toml::from_str(include_str!("../../../../installer-components.toml")).ok()?;
    contract
        .get("components")?
        .get(component)?
        .get(field)?
        .as_str()
        .map(PathBuf::from)
}

fn ibus_component_candidates() -> Vec<PathBuf> {
    let system_directory = wayexpand_backend_ibus::chain::component_install_directory();
    let mut candidates = vec![
        current().join("ibus/wayexpand.xml"),
        system_directory.join("wayexpand.xml"),
        system_directory.join("wayexpand-ibus.xml"),
    ];
    if let Some(path) = component_contract_path("ibus", "package_path") {
        candidates.push(path);
    }
    candidates.push(PathBuf::from(
        "/usr/local/share/ibus/component/wayexpand-ibus.xml",
    ));
    candidates
}

fn ibus_engine_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![current().join("bin/wayexpand-ibus")];
    if let Some(path) = component_contract_path("ibus", "package_install_exec_path") {
        candidates.push(path);
    }
    candidates.extend(candidates_for_binary("wayexpand-ibus"));
    candidates
}

fn candidates_for_binary(name: &str) -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/usr/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
    ];
    if let Some(path) = env::var_os("PATH") {
        paths.extend(env::split_paths(&path).map(|dir| dir.join(name)));
    }
    paths
}

fn candidates(relative: &str) -> Vec<PathBuf> {
    let name = Path::new(relative).file_name().unwrap_or_default();
    let mut paths = vec![
        current().join(relative),
        PathBuf::from("/usr").join(relative),
        PathBuf::from("/usr/local").join(relative),
    ];
    if relative.starts_with("bin/") {
        if let Some(path) = env::var_os("PATH") {
            paths.extend(env::split_paths(&path).map(|dir| dir.join(name)));
        }
    }
    paths
}
fn find(relative: &str, executable: bool) -> Option<PathBuf> {
    candidates(relative).into_iter().find(|path| {
        fs::metadata(path)
            .is_ok_and(|m| m.is_file() && (!executable || m.permissions().mode() & 0o111 != 0))
    })
}
fn unit_available(name: &str) -> bool {
    [
        config_home().join("systemd/user"),
        PathBuf::from("/usr/lib/systemd/user"),
        PathBuf::from("/usr/local/lib/systemd/user"),
        PathBuf::from("/usr/share/wayexpand/systemd"),
    ]
    .iter()
    .any(|dir| dir.join(name).is_file())
        || current().join("systemd").join(name).is_file()
}

#[derive(Serialize)]
struct ReadinessCheck {
    component: String,
    status: String,
    detail: String,
    evidence: serde_json::Value,
}

fn readiness_check(
    component: impl Into<String>,
    status: impl Into<String>,
    detail: impl Into<String>,
    evidence: serde_json::Value,
) -> ReadinessCheck {
    ReadinessCheck {
        component: component.into(),
        status: status.into(),
        detail: detail.into(),
        evidence,
    }
}

fn systemd_property(unit: &str, property: &str) -> Option<String> {
    crate::setup_ibus::command_output(
        "systemctl",
        &["--user", "show", "--property", property, "--value", unit],
    )
    .map(|value| value.trim().to_owned())
}

fn exec_start_program(value: &str) -> Option<PathBuf> {
    let path = value.split_once("path=")?.1.split(';').next()?.trim();
    let path = path.strip_prefix('{').unwrap_or(path).trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn resolved_executable(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn package_managed_executable(path: &Path) -> bool {
    let path = path.to_string_lossy().into_owned();
    [("dpkg-query", "-S"), ("rpm", "-qf"), ("pacman", "-Qo")]
        .into_iter()
        .any(|(program, flag)| crate::setup_ibus::command_output(program, &[flag, &path]).is_some())
}

fn installation_method(path: Option<&Path>) -> &'static str {
    let Some(path) = path else {
        return "unknown";
    };
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy();
    if text.contains("/target/debug/") || text.contains("/target/release/") {
        return "developer-build";
    }
    if let Some(origin) = resolved
        .ancestors()
        .find(|ancestor| ancestor.ends_with("bin"))
        .and_then(|bin| bin.parent())
        .and_then(|root| fs::read_to_string(root.join("share/wayexpand/install-origin")).ok())
    {
        match origin.trim() {
            "distribution-package" => return "distribution-package",
            "user-local-release-archive" => return "user-local-release-archive",
            "source-installation" => return "source-installation",
            "developer-build" => return "developer-build",
            _ => {}
        }
    }
    if package_managed_executable(&resolved) {
        return "distribution-package";
    }
    if text.contains("/.local/lib/wayexpand/") {
        "unknown"
    } else if text.starts_with("/usr/local/") || text.contains("/.cargo/bin/") {
        "source-installation"
    } else {
        "unknown"
    }
}

fn path_executable(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|path| {
            fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

fn installation_identity() -> serde_json::Value {
    let cli = env::current_exe().ok();
    let path_cli = path_executable("wayexpand");
    let source = crate::doctor::status::read_daemon_status()
        .ok()
        .and_then(|response| {
            wayexpand_core::DaemonStatus::parse(&response)
                .field("source")
                .map(str::to_owned)
        });
    let service = source
        .as_deref()
        .and_then(crate::services::UserService::for_source);
    let daemon = service
        .and_then(|service| systemd_property(service.unit(), "MainPID"))
        .and_then(|pid| pid.parse::<u32>().ok())
        .filter(|pid| *pid > 0)
        .and_then(|pid| fs::read_link(format!("/proc/{pid}/exe")).ok());
    let cli_resolved = cli.as_deref().map(resolved_executable);
    let path_resolved = path_cli.as_deref().map(resolved_executable);
    let daemon_resolved = daemon.as_deref().map(|path| path.display().to_string());
    let cli_method = installation_method(cli.as_deref());
    let path_method = installation_method(path_cli.as_deref());
    let daemon_method = installation_method(daemon.as_deref());
    let root = |value: &str| {
        let path = Path::new(value);
        let text = path.to_string_lossy();
        if text.contains("/.local/lib/wayexpand/") {
            path.ancestors()
                .find(|ancestor| ancestor.ends_with(".local/lib/wayexpand"))
                .map(Path::to_path_buf)
        } else if text.starts_with("/usr/local/") {
            Some(PathBuf::from("/usr/local"))
        } else if text.starts_with("/usr/") {
            Some(PathBuf::from("/usr"))
        } else if text.contains("/.local/") {
            path.ancestors()
                .find(|ancestor| ancestor.ends_with(".local"))
                .map(Path::to_path_buf)
        } else {
            path.parent().map(Path::to_path_buf)
        }
    };
    let path_shadow = cli_resolved
        .as_deref()
        .zip(path_resolved.as_deref())
        .is_some_and(|(running, resolved)| running != resolved);
    let mixed_cli_daemon = cli_resolved
        .as_deref()
        .zip(daemon_resolved.as_deref())
        .is_some_and(|(cli, daemon)| root(cli) != root(daemon));
    serde_json::json!({
        "cli_executable": cli_resolved,
        "path_executable": path_resolved,
        "daemon_executable": daemon_resolved,
        "daemon_service": service.map(crate::services::UserService::unit),
        "cli_method": cli_method,
        "path_method": path_method,
        "daemon_method": daemon_method,
        "path_shadow_detected": path_shadow,
        "mixed_cli_daemon_installation": mixed_cli_daemon,
    })
}

fn integrated_checks(ibus: &wayexpand_backend_ibus::chain::IbusChainReport) -> Vec<ReadinessCheck> {
    let mut checks = Vec::new();
    for unit in [
        "wayexpand-control.service",
        "wayexpand-input-method.service",
        "wayexpand-evdev.service",
    ] {
        let state = systemd_property(unit, "LoadState").unwrap_or_else(|| "unavailable".into());
        checks.push(readiness_check(
            format!("systemd:{unit}"),
            if state == "loaded" {
                "pass"
            } else {
                "unavailable"
            },
            if state == "loaded" {
                "recognized by the user systemd manager"
            } else {
                "not loaded by the user systemd manager"
            },
            serde_json::json!({"load_state":state}),
        ));
        let exec_start = systemd_property(unit, "ExecStart");
        let program = exec_start.as_deref().and_then(exec_start_program);
        let executable_ok = program.as_ref().is_some_and(|path| {
            fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        });
        checks.push(readiness_check(
            format!("systemd_exec:{unit}"),
            if executable_ok { "pass" } else { "fail" },
            if executable_ok { "loaded unit ExecStart resolves to an executable" } else { "loaded unit ExecStart is missing or not executable" },
            serde_json::json!({"program":program.map(|path| path.display().to_string()),"exec_start":exec_start}),
        ));
    }
    let readiness = ibus.readiness();
    checks.push(readiness_check(
        "ibus_registration",
        if readiness.registered { "pass" } else { "unavailable" },
        "IBus live registry state and component executable contract",
        serde_json::json!({
            "component_installed":readiness.component_installed,
            "registered":readiness.registered,
            "manifest":ibus.check("manifest").map(|check| serde_json::json!({"state":format!("{:?}",check.state).to_lowercase(),"detail":check.detail})),
        }),
    ));
    let identity = installation_identity();
    let mismatch = identity["path_shadow_detected"] == true
        || identity["mixed_cli_daemon_installation"] == true;
    checks.push(readiness_check(
        "installation_path_identity",
        if mismatch { "warning" } else { "pass" },
        if mismatch { "PATH, CLI, and running daemon resolve to different installations; PATH may be selecting an older executable" } else { "CLI PATH resolution and running daemon installation are consistent" },
        identity,
    ));
    checks
}

fn operational_checks(
    ibus: &wayexpand_backend_ibus::chain::IbusReadiness,
    daemon: Option<&serde_json::Value>,
) -> Vec<ReadinessCheck> {
    let mut checks = Vec::new();
    let ibus_active = ibus.selected && ibus.engine_running;
    checks.push(readiness_check(
        "ibus_engine",
        if ibus_active { "pass" } else { "not_tested" },
        if ibus_active { "selected engine process is running; target text receipt/insertion remains a separate test" } else { "IBus engine activation has not been verified" },
        serde_json::json!({"selected":ibus.selected,"engine_running":ibus.engine_running,"text_received":ibus.text_received,"expanded":ibus.expanded}),
    ));
    let daemon_ready = daemon.is_some_and(|status| {
        crate::doctor::status::status_schema_compatible(status)
            && crate::doctor::status::daemon_build_matches(
                status
                    .get("daemon_commit")
                    .and_then(serde_json::Value::as_str),
                status
                    .get("daemon_version")
                    .and_then(serde_json::Value::as_str),
            )
            && status
                .get("config_state")
                .and_then(serde_json::Value::as_str)
                == Some("ok")
            && status
                .get("config_generation")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|generation| generation > 0)
            && status
                .get("output_connection_state")
                .and_then(serde_json::Value::as_str)
                == Some("connected")
            && status
                .get("output_health")
                .and_then(serde_json::Value::as_str)
                == Some("ok")
    });
    let evidence = daemon.map_or(serde_json::Value::Null, |status| {
        serde_json::json!({
            "daemon_commit":status.get("daemon_commit"),
            "status_schema":status.get("status_schema"),
            "source":status.get("source"),
            "backend":status.get("backend"),
            "config_state":status.get("config_state"),
            "config_generation":status.get("config_generation"),
            "route_available":status.get("route_available"),
            "matching_available":status.get("matching_available"),
            "output_connection_state":status.get("output_connection_state"),
            "output_health":status.get("output_health"),
        })
    });
    checks.push(readiness_check(
        "daemon_runtime",
        if daemon_ready { "pass" } else { "unavailable" },
        if daemon_ready {
            "daemon status protocol, build, configuration, and output health verified"
        } else {
            "no healthy, version-compatible daemon status is available"
        },
        evidence,
    ));
    checks.push(readiness_check(
        "target_insertion",
        "not_tested",
        "setup preflight never claims successful insertion; run the interactive desktop integration test",
        serde_json::json!({"verified":false}),
    ));
    checks
}
pub(crate) fn checks() -> Vec<Check> {
    let mut out = Vec::new();
    out.push(Check {
        component: "xdg_config_home",
        status: if config_home_is_valid() {
            "pass"
        } else {
            "fail"
        },
        path: config_home().display().to_string(),
        detail: "XDG_CONFIG_HOME is absolute or defaults to ~/.config",
    });
    out.push(Check {
        component: "xdg_data_home",
        status: if data_home_is_valid() { "pass" } else { "fail" },
        path: data_home().display().to_string(),
        detail: "XDG_DATA_HOME is absolute or defaults to ~/.local/share",
    });
    for (id, relative, executable) in [
        ("cli", "bin/wayexpand", true),
        ("gui", "bin/wayexpand-gui", true),
        ("daemon", "bin/wayexpand-daemon", true),
        ("desktop", "desktop/wayexpand.desktop", false),
    ] {
        let path = if id == "desktop" {
            [
                current().join(relative),
                data_home().join("applications/wayexpand.desktop"),
                PathBuf::from("/usr/share/applications/wayexpand.desktop"),
                PathBuf::from("/usr/local/share/applications/wayexpand.desktop"),
            ]
            .into_iter()
            .find(|p| p.is_file())
        } else {
            find(relative, executable)
        };
        out.push(Check {
            component: id,
            status: if path.is_some() { "pass" } else { "fail" },
            path: path.map_or_else(|| relative.into(), |p| p.display().to_string()),
            detail: if executable {
                "installed executable"
            } else {
                "desktop launcher"
            },
        });
    }
    let ibus_path = ibus_component_candidates()
        .into_iter()
        .find(|p| p.is_file());
    out.push(Check {
        component: "ibus",
        status: if ibus_path.is_some() { "pass" } else { "fail" },
        path: ibus_path.map_or_else(|| "IBus component XML".into(), |p| p.display().to_string()),
        detail: "staged or shared system component available; XDG per-user component paths are not treated as registered",
    });
    let engine = ibus_engine_candidates().into_iter().find(|path| {
        fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    });
    out.push(Check {
        component: "ibus_engine_executable",
        status: if engine.is_some() { "pass" } else { "fail" },
        path: engine.map_or_else(
            || "bin/wayexpand-ibus".into(),
            |path| path.display().to_string(),
        ),
        detail: "IBus engine executable is installed",
    });
    let contract_path = [
        current().join("share/wayexpand/installer-components.toml"),
        PathBuf::from("/usr/share/wayexpand/installer-components.toml"),
        PathBuf::from("/usr/local/share/wayexpand/installer-components.toml"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    out.push(Check {
        component: "installer_contract",
        status: if contract_path.is_some() {
            "pass"
        } else {
            "fail"
        },
        path: contract_path.map_or_else(
            || "installer-components.toml".into(),
            |p| p.display().to_string(),
        ),
        detail: "shared component contract installed",
    });
    let icon = [
        current().join("icons/hicolor/128x128/apps/wayexpand.png"),
        data_home().join("icons/hicolor/128x128/apps/wayexpand.png"),
        PathBuf::from("/usr/share/icons/hicolor/128x128/apps/wayexpand.png"),
        PathBuf::from("/usr/local/share/icons/hicolor/128x128/apps/wayexpand.png"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    out.push(Check {
        component: "icon",
        status: if icon.is_some() { "pass" } else { "fail" },
        path: icon.map_or_else(
            || "hicolor/128x128/apps/wayexpand.png".into(),
            |p| p.display().to_string(),
        ),
        detail: "application icon",
    });
    let mut default_config_candidates = component_contract_path("default_config_template", "path")
        .map(|path| vec![current().join(path)])
        .unwrap_or_else(|| vec![current().join("expansions.toml")]);
    if let Some(path) = component_contract_path("default_config_template", "package_path") {
        default_config_candidates.push(path);
    }
    default_config_candidates.extend([
        PathBuf::from("/usr/share/wayexpand/expansions.toml"),
        PathBuf::from("/usr/local/share/wayexpand/expansions.toml"),
    ]);
    let default_config_template = default_config_candidates.into_iter().find(|p| p.is_file());
    out.push(Check {
        component: "default_config_template",
        status: if default_config_template.is_some() {
            "pass"
        } else {
            "fail"
        },
        path: default_config_template.map_or_else(
            || "packaged default expansions.toml".into(),
            |p| p.display().to_string(),
        ),
        detail: "bundled starter configuration template available",
    });
    for (id, unit) in [
        ("control_service", "wayexpand-control.service"),
        ("input_method_service", "wayexpand-input-method.service"),
        ("evdev_service", "wayexpand-evdev.service"),
    ] {
        let found = unit_available(unit);
        out.push(Check {
            component: id,
            status: if found {
                "pass"
            } else if id == "evdev_service" {
                "optional"
            } else {
                "fail"
            },
            path: unit.into(),
            detail: if id == "evdev_service" {
                "optional; global capture requires explicit consent"
            } else {
                "user service unit available"
            },
        });
    }
    let config = wayexpand_core::default_config_path();
    let valid = config.is_file() && wayexpand_core::Config::load(&config).is_ok();
    out.push(Check {
        component: "active_configuration",
        status: if valid { "pass" } else { "fail" },
        path: config.display().to_string(),
        detail: "user's active configuration exists and parses",
    });
    let contract: toml::Value =
        toml::from_str(include_str!("../../../../installer-components.toml"))
            .expect("checked-in installer component contract must be valid TOML");
    let known: std::collections::BTreeSet<&str> = out.iter().map(|c| c.component).collect();
    if let Some(components) = contract.get("components").and_then(toml::Value::as_table) {
        for (id, declaration) in components {
            if declaration.get("required").and_then(toml::Value::as_bool) == Some(true)
                && !known.contains(id.as_str())
            {
                out.push(Check {
                    component: "contract_implementation",
                    status: "fail",
                    path: id.clone(),
                    detail: "required manifest component has no install-check implementation",
                });
            }
        }
    }
    out
}
fn print_report(title: &str, json: bool) -> Result<bool> {
    let rows = checks();
    let healthy = rows.iter().all(|r| r.status != "fail");
    let identity = installation_identity();
    if json {
        println!(
            "{}",
            serde_json::json!({"schema":2,"level":"installed","title":title,"ready":healthy,"installation_identity":identity,"checks":rows})
        );
    } else {
        println!("{title}");
        println!(
            "Installation method: CLI={} PATH={} daemon={}",
            identity["cli_method"].as_str().unwrap_or("unknown"),
            identity["path_method"].as_str().unwrap_or("unknown"),
            identity["daemon_method"].as_str().unwrap_or("unknown"),
        );
        if identity["path_shadow_detected"] == true {
            println!("WARNING: PATH resolves a different wayexpand executable than the one running this command.");
            println!(
                "  CLI:  {}",
                identity["cli_executable"].as_str().unwrap_or("unknown")
            );
            println!(
                "  PATH: {}",
                identity["path_executable"].as_str().unwrap_or("not found")
            );
        }
        if identity["mixed_cli_daemon_installation"] == true {
            println!("WARNING: CLI and running daemon belong to different installation roots.");
        }
        for r in &rows {
            println!(
                "{:4} {:<22} {} — {}",
                r.status.to_uppercase(),
                r.component,
                r.path,
                r.detail
            );
        }
    }
    Ok(healthy)
}
pub(crate) fn install_check(args: Args) -> Result<()> {
    let mut json = false;
    for arg in args {
        if arg == "--json" {
            json = true
        } else {
            bail!("usage: wayexpand install-check [--json]")
        }
    }
    if !print_report("WayExpand installation audit", json)? {
        bail!("installation audit found missing required components")
    }
    Ok(())
}
pub(crate) fn setup_check(json: bool) -> Result<()> {
    let rows = checks();
    let caps = wayexpand_backend_selection::probe_capabilities();
    let policy = crate::doctor::policy::load_policy()?;
    let ibus_available =
        wayexpand_backend_ibus::engine_available() || wayexpand_backend_ibus::engine_installed();
    let recommendation =
        crate::setup::recommended_setup_backend_with_ibus(&caps, ibus_available, &policy);
    let route_plan = wayexpand_backend_selection::plan_routes(&caps, ibus_available, |route| {
        wayexpand_backend_selection::route_allowed_by_policy(&policy, route)
    });
    let ibus_report = wayexpand_backend_ibus::chain::diagnose();
    let ibus_readiness = ibus_report.readiness();
    let first_run_test_available =
        wayexpand_core::Config::load(wayexpand_core::default_config_path())
            .ok()
            .is_some_and(|config| crate::commands::system::setup_test_trigger(&config).is_some());
    let daemon_status = crate::doctor::status::read_daemon_status()
        .ok()
        .and_then(|response| crate::doctor::status::status_as_json(&response).ok());
    let terminal_only_runtime = daemon_status
        .as_ref()
        .and_then(|status| status.get("terminal_apps_only"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let terminal_match = terminal_match_self_test(daemon_status.as_ref(), terminal_only_runtime);
    let integration = integrated_checks(&ibus_report);
    let operational = operational_checks(&ibus_readiness, daemon_status.as_ref());
    let installed = rows.iter().all(|r| r.status != "fail");
    let route_ready = recommendation.backend != "unavailable";
    let terminal_match_required = terminal_only_runtime;
    let terminal_match_ok = terminal_match
        .get("status")
        .and_then(|value| value.as_str())
        == Some("pass");
    let ready = installed && route_ready && (!terminal_match_required || terminal_match_ok);
    if json {
        println!(
            "{}",
            serde_json::json!({
            "schema":2, "title":"WayExpand setup readiness", "ready":ready,
                "readiness_levels":{
                    "installed":{"status":if installed {"pass"} else {"fail"},"checks":rows.iter().map(|row| serde_json::json!({"component":row.component,"status":row.status,"path":row.path,"detail":row.detail})).collect::<Vec<_>>()},
                    "integrated":{"status":if integration.iter().all(|check| check.status == "pass" || check.status == "warning") {"pass"} else {"incomplete"},"checks":integration},
                    "operational":{"status":if operational.iter().any(|check| check.component == "daemon_runtime" && check.status == "pass") || operational.iter().any(|check| check.component == "ibus_engine" && check.status == "pass") {"partial"} else {"not_ready"},"checks":operational},
                    "target_insertion":{"status":"not_tested","verified":false},
                    "terminal_matching_self_test":&terminal_match,
                },
            "installation_ready":installed, "route_ready":route_ready,
                "installation_checks":rows,
                "desktop":{"session":crate::doctor::backends::session_description(),"recommended_input":recommendation.label,"recommended_backend":recommendation.backend,"reason":recommendation.detail},
                "route_assessments":route_plan.routes.iter().map(|planned| serde_json::json!({
                    "id":planned.contract.id,
                    "standing":planned.standing.as_str(),
                    "reason":planned.reason,
                    "assessment":&planned.assessment,
                })).collect::<Vec<_>>(),
            "ibus_runtime":ibus_readiness,
                "terminal_matching_self_test":&terminal_match,
                "activation":{"required":true,"command":"wayexpand setup --apply","real_insertion_verified":false,"consent_granted":false},
                "first_run_test":{"command":"wayexpand setup --apply","manual_command":"wayexpand debug --test <trigger>","selection":"prefer a testable ;;hello sample, otherwise an enabled static snippet","available":first_run_test_available,"real_application_confirmation_required":true,"verified":false}
            })
        );
    } else {
        println!("WayExpand Setup Check\n\nInstalled artifacts:");
        for r in &rows {
            println!(
                "  {:4} {} — {}",
                r.status.to_uppercase(),
                r.component,
                r.path
            );
        }
        println!("\nIntegration:");
        for check in &integration {
            println!(
                "  {:<12} {} — {}",
                check.status.to_uppercase(),
                check.component,
                check.detail
            );
            if check.component == "installation_path_identity" {
                println!(
                    "    CLI installation method: {}",
                    check.evidence["cli_method"].as_str().unwrap_or("unknown")
                );
                println!(
                    "    PATH installation method: {}",
                    check.evidence["path_method"].as_str().unwrap_or("unknown")
                );
                println!(
                    "    Daemon installation method: {}",
                    check.evidence["daemon_method"]
                        .as_str()
                        .unwrap_or("unknown")
                );
                println!(
                    "    CLI: {}",
                    check.evidence["cli_executable"]
                        .as_str()
                        .unwrap_or("unknown")
                );
                println!(
                    "    PATH: {}",
                    check.evidence["path_executable"]
                        .as_str()
                        .unwrap_or("not found")
                );
                println!(
                    "    Daemon: {}",
                    check.evidence["daemon_executable"]
                        .as_str()
                        .unwrap_or("not running")
                );
            }
        }
        println!("\nOperational:");
        for check in &operational {
            println!(
                "  {:<12} {} — {}",
                check.status.to_uppercase(),
                check.component,
                check.detail
            );
        }
        println!("\nTerminal matching self-test:");
        println!(
            "  {} — {}",
            terminal_match["status"]
                .as_str()
                .unwrap_or("unknown")
                .to_uppercase(),
            terminal_match["detail"]
                .as_str()
                .unwrap_or("could not evaluate terminal matching")
        );
        println!("\nDesktop: {}\nRecommended: {} ({})\nSetup preflight: {}\nTarget insertion: NOT TESTED\nFirst-run test: {}\n\nNext: wayexpand setup --apply\n\nPreflight does not grant raw-input/portal consent or verify target-app insertion.", crate::doctor::backends::session_description(), recommendation.label, recommendation.detail, if ready {"ready to configure"} else {"prerequisites incomplete"}, if first_run_test_available {"`wayexpand setup --apply` can offer a real-app test using an enabled static snippet (requires confirmation)"} else {"no enabled static snippet is configured; add a harmless one for an optional test"});
    }
    if !ready {
        bail!("setup prerequisites are incomplete; see the failed checks")
    }
    Ok(())
}

/// Exercise the actual matching engine under terminal-only admission. This
/// catches regressions where the daemon is alive but terminal scoping or the
/// configured terminal application list prevents every expansion from
/// matching. No command-backed snippets are executed and no output is sent.
fn terminal_match_self_test(
    daemon_status: Option<&serde_json::Value>,
    terminal_only_runtime: bool,
) -> serde_json::Value {
    if terminal_only_runtime {
        let response =
            crate::control_operation(wayexpand_core::DaemonOperation::TerminalMatchSelfTest);
        return parse_terminal_match_self_test(response, daemon_status);
    }
    let config_path = wayexpand_core::default_config_path();
    let configuration_source = "cli_default_path";
    let config = match wayexpand_core::Config::load(&config_path) {
        Ok(config) => config,
        Err(error) => {
            return serde_json::json!({
                "status":"fail",
                "detail":format!("configuration could not be loaded: {}", error.safe_summary()),
                "configuration_path":config_path,
                "configuration_source":configuration_source,
            })
        }
    };
    let mut report = terminal_match_self_test_for(&config);
    report["configuration_path"] = serde_json::json!(config_path);
    report["configuration_source"] = serde_json::json!(configuration_source);
    report
}

fn parse_terminal_match_self_test(
    response: Result<String>,
    daemon_status: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut report = match response {
        Ok(response) => serde_json::from_str::<serde_json::Value>(&response).unwrap_or_else(|_| {
            serde_json::json!({
                "status":"fail",
                "detail":"the daemon returned an invalid terminal matching self-test response"
            })
        }),
        Err(error) => serde_json::json!({
            "status":"fail",
            "detail":format!("the live daemon matcher could not be checked: {error}")
        }),
    };
    if !matches!(
        report.get("status").and_then(serde_json::Value::as_str),
        Some("pass" | "fail")
    ) {
        report = serde_json::json!({
            "status":"fail",
            "detail":"the daemon returned an invalid terminal matching self-test status"
        });
    }
    report["configuration_source"] = serde_json::json!("daemon_live_matcher");
    if let Some(path) = daemon_status.and_then(|status| status.get("config_active_path")) {
        report["configuration_path"] = path.clone();
    }
    report
}

fn terminal_match_self_test_for(config: &wayexpand_core::Config) -> serde_json::Value {
    if config.settings.terminal_apps.is_empty() {
        return serde_json::json!({"status":"not_applicable","detail":"no terminal application identifiers are configured"});
    }
    let mut engine = match wayexpand_core::ExpansionEngine::new(config.clone()) {
        Ok(engine) => engine,
        Err(error) => {
            return serde_json::json!({"status":"fail","detail":format!("matcher could not be built: {error}")})
        }
    };
    engine.set_terminal_apps_only(true);
    for terminal_filter in &config.settings.terminal_apps {
        let app_id = terminal_filter
            .strip_prefix("app_id_exact:")
            .or_else(|| terminal_filter.strip_prefix("app_id_glob:"))
            .unwrap_or(terminal_filter);
        let app_id = if terminal_filter.starts_with("app_id_glob:") {
            app_id.replace('*', "terminal").replace('?', "x")
        } else {
            app_id.to_owned()
        };
        for expansion in config
            .expansion
            .iter()
            .filter(|item| item.enabled && item.command.is_none())
        {
            engine.process(wayexpand_core::InputEvent::Reset);
            engine.process(wayexpand_core::InputEvent::WindowChanged(Some(
                wayexpand_core::WindowContext {
                    app_id: Some(app_id.clone()),
                    ..Default::default()
                },
            )));
            let mut matches = Vec::new();
            for character in expansion.trigger.chars() {
                matches.extend(
                    engine.process(wayexpand_core::InputEvent::Text(character.to_string())),
                );
            }
            matches.extend(engine.process(wayexpand_core::InputEvent::Delimiter(' ')));
            if matches
                .iter()
                .any(|matched| matched.trigger.as_ref() == expansion.trigger)
            {
                return serde_json::json!({"status":"pass","detail":"a configured static expansion matched through terminal-only admission","terminal_application_count":config.settings.terminal_apps.len()});
            }
        }
    }
    serde_json::json!({"status":"fail","detail":"no enabled static expansion matched for any configured terminal application under terminal-only admission","terminal_application_count":config.settings.terminal_apps.len()})
}
pub(crate) fn repair(args: Args) -> Result<()> {
    let mut check = false;
    let mut apply = false;
    let mut json = false;
    let mut gnome_input_source = false;
    for arg in args {
        match arg.as_str() {
            "--check" => check = true,
            "--apply" => apply = true,
            "--json" => json = true,
            "--gnome-input-source" => gnome_input_source = true,
            _ => bail!(
                "usage: wayexpand repair --check|--apply [--json] | --gnome-input-source [--json]"
            ),
        }
    }
    if gnome_input_source {
        if check || apply {
            bail!("`--gnome-input-source` is a direct repair action; do not combine it with --check or --apply");
        }
        crate::setup_ibus::repair_gnome_input_source()?;
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "schema": 1,
                    "repair": "gnome-input-source",
                    "state": "pass",
                    "source_configuration": "pass",
                    "operational_readiness": "unknown",
                    "detail": "GNOME source is registered and active; real-field input still requires verification"
                })
            );
        } else {
            println!("WayExpand is registered and selected as GNOME's current IBus input source.");
            println!("Input from a normal text field has not been tested by this repair.");
        }
        return Ok(());
    }
    if check == apply {
        bail!("choose exactly one of --check or --apply")
    }
    let unit_dir = config_home().join("systemd/user");
    let source = current().join("systemd");
    let mut repairs = Vec::new();
    for unit in [
        "wayexpand-control.service",
        "wayexpand-input-method.service",
    ] {
        let dst = unit_dir.join(unit);
        let src = source.join(unit);
        if !dst.exists() && src.is_file() {
            repairs.push((src, dst));
        }
    }
    if apply {
        fs::create_dir_all(&unit_dir)?;
        for (src, dst) in &repairs {
            std::os::unix::fs::symlink(src, dst)?;
        }
        if !repairs.is_empty() {
            let status =
                crate::setup::run_setup_command("systemctl", &["--user", "daemon-reload"])?;
            if !status.success() {
                bail!("systemd user manager rejected daemon-reload; created unit links remain available for inspection")
            }
        }
    }
    if json {
        println!(
            "{}",
            serde_json::json!({"schema":1,"mode":if apply{"apply"}else{"check"},"repairs":repairs.iter().map(|(_,dst)|serde_json::json!({"component":dst.file_name().unwrap_or_default().to_string_lossy(),"path":dst,"status":if apply{"repaired"}else{"available"}})).collect::<Vec<_>>(),"consent_required": ["raw evdev access","remote desktop portal access","IBus engine selection"]})
        );
    } else if repairs.is_empty() {
        println!("No automatic repairs are available.");
    } else {
        for (_, dst) in &repairs {
            println!(
                "{} {}",
                if apply {
                    "Repaired"
                } else {
                    "Repair available:"
                },
                dst.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_matching_self_test_exercises_the_terminal_only_engine_gate() {
        let config = wayexpand_core::Config::parse(
            r#"
            [settings]
            terminal_apps = ["org.gnome.Terminal"]

            [[expansion]]
            trigger = ";;health"
            replacement = "read only check"
            "#,
        )
        .unwrap();
        let report = terminal_match_self_test_for(&config);
        assert_eq!(report["status"], "pass", "{report}");

        let blocked = wayexpand_core::Config::parse(
            r#"
            [settings]
            terminal_apps = ["org.gnome.Terminal"]

            [[expansion]]
            trigger = ";;health"
            replacement = "read only check"
            app_filter = ["app_id_exact:org.kde.konsole"]
            "#,
        )
        .unwrap();
        assert_eq!(terminal_match_self_test_for(&blocked)["status"], "fail");
    }

    #[test]
    fn terminal_matching_self_test_uses_the_live_daemon_matcher_result() {
        let path = "/home/operator/.config/wayexpand/expansions.toml";
        let daemon_status = serde_json::json!({"config_active_path":path});
        let report = parse_terminal_match_self_test(
            Ok(r#"{"status":"pass","detail":"live match succeeded"}"#.into()),
            Some(&daemon_status),
        );
        assert_eq!(report["status"], "pass");
        assert_eq!(report["configuration_source"], "daemon_live_matcher");
        assert_eq!(report["configuration_path"], path);

        let invalid = parse_terminal_match_self_test(Ok("{}".into()), None);
        assert_eq!(invalid["status"], "fail");
        let unavailable = parse_terminal_match_self_test(Err(anyhow::anyhow!("offline")), None);
        assert_eq!(unavailable["status"], "fail");
    }

    #[test]
    fn xdg_home_resolution_matches_installers_for_empty_and_custom_values() {
        let fallback = PathBuf::from("/home/operator/.config");
        assert_eq!(resolve_xdg_home(None, fallback.clone()), fallback);
        assert_eq!(
            resolve_xdg_home(Some(std::ffi::OsStr::new("")), PathBuf::from("/fallback")),
            PathBuf::from("/fallback")
        );
        assert_eq!(
            resolve_xdg_home(
                Some(std::ffi::OsStr::new("relative/config")),
                PathBuf::from("/fallback")
            ),
            PathBuf::from("/fallback")
        );
        assert_eq!(
            resolve_xdg_home(
                Some(std::ffi::OsStr::new("/srv/operator/config")),
                PathBuf::from("/fallback")
            ),
            PathBuf::from("/srv/operator/config")
        );
        assert!(xdg_home_is_valid(None));
        assert!(xdg_home_is_valid(Some(std::ffi::OsStr::new(""))));
        assert!(!xdg_home_is_valid(Some(std::ffi::OsStr::new(
            "relative/config"
        ))));
        assert!(xdg_home_is_valid(Some(std::ffi::OsStr::new(
            "/srv/operator/config"
        ))));
    }

    #[test]
    fn install_origin_marker_distinguishes_source_and_release_archive() {
        let root = env::temp_dir().join(format!(
            "wayexpand-install-origin-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let binary = root.join("bin/wayexpand");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::create_dir_all(root.join("share/wayexpand")).unwrap();
        fs::write(&binary, "test executable").unwrap();
        for (origin, expected) in [
            ("source-installation", "source-installation"),
            ("user-local-release-archive", "user-local-release-archive"),
        ] {
            fs::write(root.join("share/wayexpand/install-origin"), origin).unwrap();
            assert_eq!(installation_method(Some(&binary)), expected);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_required_manifest_component_has_a_readiness_check() {
        let contract: toml::Value =
            toml::from_str(include_str!("../../../../installer-components.toml")).unwrap();
        let known: std::collections::BTreeSet<_> = checks()
            .into_iter()
            .map(|check| check.component.to_owned())
            .collect();
        for (id, declaration) in contract["components"].as_table().unwrap() {
            if declaration["required"].as_bool() == Some(true) {
                assert!(
                    known.contains(id),
                    "required component {id} has no readiness check"
                );
            }
        }
    }

    #[test]
    fn installation_audit_separates_bundled_template_from_active_configuration() {
        let rows = checks();
        let template = rows
            .iter()
            .find(|check| check.component == "default_config_template")
            .expect("bundled template check is present");
        let active = rows
            .iter()
            .find(|check| check.component == "active_configuration")
            .expect("active configuration check is present");
        assert!(template.detail.contains("template"));
        assert!(active.detail.contains("active configuration"));
        assert_ne!(template.component, active.component);
    }

    #[test]
    fn ibus_contract_and_install_check_exclude_per_user_registration() {
        let contract: toml::Value =
            toml::from_str(include_str!("../../../../installer-components.toml")).unwrap();
        let ibus = &contract["components"]["ibus"];
        assert_eq!(ibus["user_registration"].as_bool(), Some(false));
        assert!(ibus["registration"]
            .as_str()
            .unwrap()
            .contains("system-wide"));
        assert_eq!(ibus["path"].as_str(), Some("ibus/wayexpand.xml"));
        assert_eq!(
            ibus["package_path"].as_str(),
            Some("/usr/share/ibus/component/wayexpand-ibus.xml")
        );
        assert_eq!(
            ibus["package_install_exec"].as_str(),
            Some("wayexpand-ibus")
        );
        assert_eq!(
            ibus["package_install_exec_path"].as_str(),
            Some("/usr/bin/wayexpand-ibus")
        );
        assert!(ibus_engine_candidates().contains(&PathBuf::from("/usr/bin/wayexpand-ibus")));
        let candidates = ibus_component_candidates();
        assert!(!candidates.contains(&data_home().join("ibus/component/wayexpand.xml")));
        assert!(candidates.contains(
            &wayexpand_backend_ibus::chain::component_install_directory().join("wayexpand.xml")
        ));
        let default_config = &contract["components"]["default_config_template"];
        assert_eq!(default_config["path"].as_str(), Some("expansions.toml"));
        assert_eq!(
            default_config["package_path"].as_str(),
            Some("/etc/wayexpand/expansions.toml.example")
        );
    }

    #[test]
    fn consent_gated_capture_is_optional_in_the_contract() {
        let contract: toml::Value =
            toml::from_str(include_str!("../../../../installer-components.toml")).unwrap();
        let evdev = &contract["components"]["evdev_service"];
        assert_eq!(evdev["required"].as_bool(), Some(false));
        assert_eq!(evdev["requires_consent"].as_bool(), Some(true));
    }

    #[test]
    fn systemd_exec_start_parser_reads_the_loaded_program_path() {
        assert_eq!(
            exec_start_program(
                "{ path=/home/operator/.local/lib/wayexpand/current/bin/wayexpand-daemon ; argv[]=/home/operator/.local/lib/wayexpand/current/bin/wayexpand-daemon --source=stdin ; }"
            ),
            Some(PathBuf::from(
                "/home/operator/.local/lib/wayexpand/current/bin/wayexpand-daemon"
            ))
        );
        assert_eq!(exec_start_program(""), None);
    }
}
