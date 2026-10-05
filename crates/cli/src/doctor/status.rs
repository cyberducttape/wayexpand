//! Daemon status reading and its JSON representation.

use super::broker::broker_diagnostics;
use crate::*;

pub(crate) fn read_daemon_status() -> Result<String> {
    let path = env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        })
        .context("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required")?;
    let mut stream = UnixStream::connect(&path)
        .with_context(|| format!("connecting to daemon socket {}", path.display()))?;
    stream.set_read_timeout(Some(CONTROL_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_IO_TIMEOUT))?;
    writeln!(stream, "status")?;
    let mut response = Vec::with_capacity(MAX_CONTROL_RESPONSE_BYTES);
    stream
        .take((MAX_CONTROL_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)?;
    if response.len() > MAX_CONTROL_RESPONSE_BYTES {
        bail!("daemon status exceeded {MAX_CONTROL_RESPONSE_BYTES} bytes");
    }
    String::from_utf8(response).context("daemon status is not valid UTF-8")
}

pub(crate) fn status_as_json(response: &str) -> Result<serde_json::Value> {
    let mut object = serde_json::Map::new();
    let mut lines = response.lines();
    if let Some(state) = lines.next() {
        object.insert("response".into(), state.into());
    }
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = match value {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            value
                if matches!(
                    key,
                    "command_queue_depth"
                        | "command_in_flight"
                        | "expansion_command_queue_depth"
                        | "expansion_command_in_flight"
                        | "hotkey_queue_depth"
                        | "hotkey_in_flight"
                        | "command_queue_rejected_total"
                        | "command_timeout_total"
                        | "command_failure_total"
                        | "status_schema"
                        | "inject_max_text_chars"
                        | "inject_expected_throughput_chars_per_sec"
                        | "injection_latency_sample_count"
                        | "injection_latency_window_count"
                        | "injection_latency_p50_us"
                        | "injection_latency_p95_us"
                        | "injection_latency_p99_us"
                ) =>
            {
                match value.parse::<u64>() {
                    Ok(number) => serde_json::Value::Number(number.into()),
                    Err(_) => value.into(),
                }
            }
            _ => value.into(),
        };
        object.insert(key.to_owned(), value);
    }
    object.insert(
        "action_broker".into(),
        broker_diagnostics(Config::load(default_config_path()).ok().as_ref()),
    );
    Ok(serde_json::Value::Object(object))
}

pub(crate) fn runtime_capabilities_from_status(snapshot: &serde_json::Value) -> serde_json::Value {
    let value = |key: &str| {
        snapshot
            .get(key)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    serde_json::json!({
        "capture": {
            "sensitive_focus": value("capture_sensitive_focus"),
            "exclusive": value("capture_exclusive"),
            "reliable_key_state": value("capture_reliable_key_state"),
            "key_passthrough": value("capture_key_passthrough"),
            "composition_aware": value("capture_composition_aware"),
            "local_compose_aware": value("capture_local_compose_aware"),
            "layout_aware": value("capture_layout_aware"),
        },
        "window_context": {
            "tracker_connected": value("window_tracker_connected"),
        },
        "injection": {
            "insertion_mode": value("inject_insertion_mode"),
            "max_text_chars": value("inject_max_text_chars"),
            "expected_throughput_chars_per_sec": value("inject_expected_throughput_chars_per_sec"),
            "atomic_replace": value("inject_atomic_replace"),
            "full_unicode": value("inject_full_unicode"),
            "cursor_reposition": value("inject_cursor_reposition"),
            "key_passthrough": value("inject_key_passthrough"),
        },
    })
}

pub(crate) fn status_schema_compatible(snapshot: &serde_json::Value) -> bool {
    snapshot
        .get("status_schema")
        .and_then(serde_json::Value::as_u64)
        == Some(u64::from(CONTROL_STATUS_SCHEMA))
}
