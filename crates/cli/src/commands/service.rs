//! Explicit inspection and restart of the active managed daemon service.

use super::Args;
use crate::*;

fn active_service() -> Option<crate::services::UserService> {
    let snapshot = crate::doctor::status::read_daemon_status()
        .ok()
        .and_then(|raw| crate::doctor::status::status_as_json(&raw).ok())?;
    let source = snapshot.get("source")?.as_str()?;
    crate::services::UserService::for_source(source)
}

pub(crate) fn service_command(mut args: Args) -> Result<()> {
    let action = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand service status [--json] | restart --yes"))?;
    let mut json = false;
    let mut yes = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--yes" => yes = true,
            _ => usage_bail!("usage: wayexpand service status [--json] | restart --yes"),
        }
    }
    let inventory = crate::services::ServiceInventory::probe();
    match action.as_str() {
        "status" => {
            let active = active_service();
            let rows = crate::services::UserService::ALL
                .iter()
                .map(|service| {
                    let state = inventory.state(*service);
                    serde_json::json!({
                        "unit":service.unit(), "installed":state.installed,
                        "enabled":state.enabled, "active":state.active, "failed":state.failed,
                        "selected_route":Some(*service) == active,
                    })
                })
                .collect::<Vec<_>>();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"schema_version":1,"services":rows})
                    )?
                );
            } else {
                for row in rows {
                    println!(
                        "{}: installed={} enabled={} active={} failed={} selected_route={}",
                        row["unit"].as_str().unwrap_or("unknown"),
                        row["installed"],
                        row["enabled"],
                        row["active"],
                        row["failed"],
                        row["selected_route"]
                    );
                }
            }
        }
        "restart" => {
            if json {
                usage_bail!("usage: wayexpand service restart --yes");
            }
            if !yes {
                bail!("restarting the active WayExpand service interrupts capture; review `wayexpand service status` and repeat with --yes");
            }
            let service = active_service().ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot identify the active managed route; refusing to guess a service"
                )
            })?;
            let state = inventory.state(service);
            if !state.installed || !state.active {
                bail!(
                    "{} is not both installed and active; refusing to start or switch routes",
                    service.unit()
                );
            }
            crate::setup_ibus::command_output("systemctl", &["--user", "restart", service.unit()])
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "systemd could not restart {}; inspect `journalctl --user -u {}`",
                        service.unit(),
                        service.unit()
                    )
                })?;
            println!("Restarted active service {}", service.unit());
        }
        _ => usage_bail!("usage: wayexpand service status [--json] | restart --yes"),
    }
    Ok(())
}
