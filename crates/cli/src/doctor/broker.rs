//! Action Broker health: socket, service, and audit status.

use super::files::existing_control_socket_is_healthy;
use crate::setup::run_setup_command;
use crate::*;

pub(crate) fn broker_socket_path() -> Option<PathBuf> {
    std::env::var_os("WAYEXPAND_ACTION_BROKER_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(|dir| PathBuf::from(dir).join("wayexpand-broker.sock"))
        })
}

pub(crate) fn broker_health_path(socket: &Path) -> PathBuf {
    socket
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("wayexpand-broker-health.json")
}

pub(crate) fn broker_health_status(socket: Option<&Path>) -> Option<serde_json::Value> {
    let path = socket.map(broker_health_path)?;
    let contents = fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

pub(crate) fn broker_service_active() -> bool {
    run_setup_command(
        "systemctl",
        &[
            "--user",
            "is-active",
            "--quiet",
            "wayexpand-action-broker.service",
        ],
    )
    .is_ok_and(|status| status.success())
}

pub(crate) fn broker_diagnostics(config: Option<&Config>) -> serde_json::Value {
    let named_actions = config.map_or(0, |config| {
        config
            .expansion
            .iter()
            .filter(|expansion| {
                expansion
                    .command
                    .as_ref()
                    .is_some_and(|command| command.action.is_some())
            })
            .count()
            + config
                .hotkey
                .iter()
                .filter(|hotkey| hotkey.command.action.is_some())
                .count()
    });
    let socket = broker_socket_path();
    let socket_exists = socket.as_ref().is_some_and(|path| path.exists());
    let socket_valid = socket
        .as_deref()
        .is_some_and(existing_control_socket_is_healthy);
    let service_active = broker_service_active();
    let audit_status = broker_health_status(socket.as_deref());
    let audit_enabled = audit_status
        .as_ref()
        .and_then(|status| status.get("audit_enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let audit_healthy = !audit_enabled
        || audit_status
            .as_ref()
            .and_then(|status| status.get("audit_healthy"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    let required = named_actions > 0;
    serde_json::json!({
        "required": required,
        "named_action_count": named_actions,
        "socket": {
            "path": socket,
            "exists": socket_exists,
            "valid": socket_valid,
        },
        "service": {
            "unit": "wayexpand-action-broker.service",
            "active": service_active,
        },
        "audit": audit_status.unwrap_or_else(|| serde_json::json!({
            "available": false,
            "audit_enabled": false,
            "audit_healthy": true,
        })),
        "healthy": !required || (socket_exists && socket_valid && service_active && audit_healthy),
    })
}

pub(crate) fn print_broker_diagnostics(config: Option<&Config>) -> bool {
    let broker = broker_diagnostics(config);
    if !broker["required"].as_bool().unwrap_or(false) {
        println!("Action Broker: not required (no named actions configured)");
        return true;
    }
    let socket = broker["socket"]["path"].as_str().unwrap_or("unconfigured");
    let active = broker["service"]["active"].as_bool().unwrap_or(false);
    let exists = broker["socket"]["exists"].as_bool().unwrap_or(false);
    let valid = broker["socket"]["valid"].as_bool().unwrap_or(false);
    println!("Action Broker: {} named action(s), service_active={active}, socket={socket}, socket_exists={exists}, socket_valid={valid}", broker["named_action_count"]);
    if broker["audit"]["audit_enabled"].as_bool().unwrap_or(false) {
        let dropped = broker["audit"]["audit_queue_dropped_total"]
            .as_u64()
            .unwrap_or(0);
        let failures = broker["audit"]["audit_write_failures_total"]
            .as_u64()
            .unwrap_or(0);
        let healthy = broker["audit"]["audit_healthy"].as_bool().unwrap_or(false);
        println!(
            "  Action audit: healthy={healthy}, queue_dropped_total={dropped}, write_failures_total={failures}"
        );
    }
    if !active || !exists || !valid {
        println!("  Enable it with: systemctl --user enable --now wayexpand-action-broker.service");
        return false;
    }
    true
}
