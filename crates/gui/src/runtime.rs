//! Bounded background runtime for daemon control and desktop probes.

use crate::{diagnostics, status::Status};
use anyhow::Context;
use std::{
    env,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::Duration,
};
use wayexpand_core::{discover_backends, Config, FleetConfig};

const REQUEST_CAPACITY: usize = 8;
const RESULT_CAPACITY: usize = 16;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTROL_RESPONSE_BYTES: usize = 4096;

pub(crate) enum Request {
    Diagnostics {
        config: Config,
        announce: bool,
    },
    Control {
        command: String,
        operation: Operation,
    },
}

pub(crate) enum Operation {
    Reload(Status),
    Pause { paused: bool },
    Status,
}

pub(crate) enum Completion {
    Diagnostics(DiagnosticsSnapshot),
    Control {
        operation: Operation,
        result: anyhow::Result<String>,
    },
}

pub(crate) struct DiagnosticsSnapshot {
    pub backend_status: Vec<wayexpand_core::BackendStatus>,
    pub fleet_status: String,
    pub protocol_probes: Vec<(String, String)>,
    pub daemon_status: String,
    pub daemon_connected: Option<bool>,
    pub paused: Option<bool>,
    pub announce: bool,
}

pub(crate) fn start() -> std::io::Result<(SyncSender<Request>, Receiver<Completion>)> {
    let (request_sender, request_receiver) = mpsc::sync_channel(REQUEST_CAPACITY);
    let (completion_sender, completion_receiver) = mpsc::sync_channel(RESULT_CAPACITY);
    thread::Builder::new()
        .name("wayexpand-gui-runtime".into())
        .spawn(move || {
            while let Ok(request) = request_receiver.recv() {
                let completion = match request {
                    Request::Diagnostics { config, announce } => {
                        Completion::Diagnostics(run_diagnostics(config, announce))
                    }
                    Request::Control { command, operation } => Completion::Control {
                        operation,
                        result: control_command(&command),
                    },
                };
                if completion_sender.send(completion).is_err() {
                    break;
                }
            }
        })?;
    Ok((request_sender, completion_receiver))
}

fn run_diagnostics(config: Config, announce: bool) -> DiagnosticsSnapshot {
    let backend_status = discover_backends();
    let fleet_status = match wayexpand_core::load_organization_policy().and_then(|policy| {
        FleetConfig::load_standard_with_base_and_policy(config, &policy)
            .map_err(|error| error.to_string())
    }) {
        Ok(fleet) => {
            let mut status = format!(
                "active · {} files · {} expansions · {} hotkeys",
                fleet.stats.total_files_loaded,
                fleet.stats.total_expansions,
                fleet.stats.total_hotkeys
            );
            if !fleet.policy_violations.is_empty() {
                status.push_str(" · policy: ");
                status.push_str(&fleet.policy_violations.join("; "));
            }
            status
        }
        Err(error) => format!("invalid: {error}"),
    };
    let protocol_probes = diagnostics::probe_protocols();
    let daemon_response = control_command("status");
    let daemon_connected = Some(
        daemon_response
            .as_ref()
            .ok()
            .and_then(|response| parse_connected(response))
            .unwrap_or(false),
    );
    let (daemon_status, paused) = match daemon_response {
        Ok(response) => (
            response.trim().replace('\n', " · "),
            parse_paused(&response),
        ),
        Err(error) => (format!("Unavailable: {error}"), None),
    };
    DiagnosticsSnapshot {
        backend_status,
        fleet_status,
        protocol_probes,
        daemon_status,
        daemon_connected,
        paused,
        announce,
    }
}

pub(crate) fn parse_connected(response: &str) -> Option<bool> {
    response.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key == "state").then(|| value == "connected")
    })
}

pub(crate) fn parse_paused(response: &str) -> Option<bool> {
    response.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key == "paused").then(|| value == "true")
    })
}

fn control_command(command: &str) -> anyhow::Result<String> {
    let path = env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        })
        .context("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required")?;
    let mut stream =
        UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))?;
    stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
    writeln!(stream, "{command}")?;
    let mut response = Vec::new();
    stream
        .take((MAX_CONTROL_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)?;
    if response.len() > MAX_CONTROL_RESPONSE_BYTES {
        anyhow::bail!("daemon control response exceeded {MAX_CONTROL_RESPONSE_BYTES} bytes");
    }
    String::from_utf8(response).context("daemon returned a non-UTF-8 control response")
}

#[cfg(test)]
mod tests {
    #[test]
    fn paused_state_parser_ignores_unrelated_status_lines() {
        assert_eq!(
            super::parse_paused("state=running\npaused=true\n"),
            Some(true)
        );
        assert_eq!(super::parse_paused("paused=false\n"), Some(false));
        assert_eq!(super::parse_paused("state=stopped\n"), None);
    }

    #[test]
    fn connection_parser_requires_the_daemon_state_field() {
        assert_eq!(super::parse_connected("state=connected\n"), Some(true));
        assert_eq!(super::parse_connected("state=stopped\n"), Some(false));
        assert_eq!(super::parse_connected("running\npaused=false\n"), None);
    }
}
