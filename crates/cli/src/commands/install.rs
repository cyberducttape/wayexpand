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
fn data_home() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
}
fn config_home() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}
fn current() -> PathBuf {
    home().join(".local/lib/wayexpand/current")
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
pub(crate) fn checks() -> Vec<Check> {
    let mut out = Vec::new();
    for (id, relative, executable) in [
        ("cli", "bin/wayexpand", true),
        ("gui", "bin/wayexpand-gui", true),
        ("daemon", "bin/wayexpand-daemon", true),
        ("desktop", "desktop/wayexpand.desktop", false),
    ] {
        let path = if id == "desktop" {
            [
                current().join(relative),
                home().join(".local/share/applications/wayexpand.desktop"),
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
    let ibus_path = [
        data_home().join("ibus/component/wayexpand.xml"),
        current().join("ibus/wayexpand.xml"),
        PathBuf::from("/usr/share/ibus/component/wayexpand-ibus.xml"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    out.push(Check {
        component: "ibus",
        status: if ibus_path.is_some() { "pass" } else { "fail" },
        path: ibus_path.map_or_else(|| "IBus component XML".into(), |p| p.display().to_string()),
        detail: "component available; engine selection remains explicit",
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
    let default_config = [
        current().join("expansions.toml"),
        PathBuf::from("/etc/wayexpand/expansions.toml.example"),
        PathBuf::from("/usr/share/wayexpand/expansions.toml"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    out.push(Check {
        component: "default_config",
        status: if default_config.is_some() {
            "pass"
        } else {
            "fail"
        },
        path: default_config.map_or_else(
            || "packaged default expansions.toml".into(),
            |p| p.display().to_string(),
        ),
        detail: "default configuration template available",
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
        component: "configuration",
        status: if valid { "pass" } else { "fail" },
        path: config.display().to_string(),
        detail: "configuration exists and parses",
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
    if json {
        println!(
            "{}",
            serde_json::json!({"schema":1,"title":title,"ready":healthy,"checks":rows})
        );
    } else {
        println!("{title}");
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
    let ibus_readiness = wayexpand_backend_ibus::chain::diagnose().readiness();
    let installed = rows.iter().all(|r| r.status != "fail");
    let route_ready = recommendation.backend != "unavailable";
    let ready = installed && route_ready;
    if json {
        println!(
            "{}",
            serde_json::json!({
            "schema":1, "title":"WayExpand setup readiness", "ready":ready,
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
                "activation":{"required":true,"command":"wayexpand setup --apply","real_insertion_verified":false,"consent_granted":false}
            })
        );
    } else {
        println!("WayExpand Setup Check\n\nInstallation:");
        for r in &rows {
            println!(
                "  {:4} {} — {}",
                r.status.to_uppercase(),
                r.component,
                r.path
            );
        }
        println!("\nDesktop: {}\nRecommended: {} ({})\nRoute readiness: {}\n\nNext: wayexpand setup --apply\n\nPreflight does not verify insertion in a target application or grant raw-input/portal consent.", crate::doctor::backends::session_description(), recommendation.label, recommendation.detail, if route_ready {"available for setup"} else {"no safe automatic route available"});
        println!("IBus runtime stages: component={}, registered={}, running={}, selected={}, engine={}, text_received={}, expanded={}", ibus_readiness.component_installed, ibus_readiness.registered, ibus_readiness.ibus_running, ibus_readiness.selected, ibus_readiness.engine_running, ibus_readiness.text_received, ibus_readiness.expanded);
    }
    if !ready {
        bail!("setup prerequisites are incomplete; see the failed checks")
    }
    Ok(())
}
pub(crate) fn repair(args: Args) -> Result<()> {
    let mut check = false;
    let mut apply = false;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--check" => check = true,
            "--apply" => apply = true,
            "--json" => json = true,
            _ => bail!("usage: wayexpand repair --check|--apply [--json]"),
        }
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
    fn consent_gated_capture_is_optional_in_the_contract() {
        let contract: toml::Value =
            toml::from_str(include_str!("../../../../installer-components.toml")).unwrap();
        let evdev = &contract["components"]["evdev_service"];
        assert_eq!(evdev["required"].as_bool(), Some(false));
        assert_eq!(evdev["requires_consent"].as_bool(), Some(true));
    }
}
