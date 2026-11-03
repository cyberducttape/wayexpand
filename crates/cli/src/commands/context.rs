//! Opt-in terminal context reporting and shell integrations.

use super::Args;
use crate::*;
use std::{
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt},
};

const BASH_INTEGRATION: &str = include_str!("../../integrations/wayexpand.bash");
const ZSH_INTEGRATION: &str = include_str!("../../integrations/wayexpand.zsh");
const FISH_INTEGRATION: &str = include_str!("../../integrations/wayexpand.fish");

pub(crate) fn context_command(mut args: Args) -> Result<()> {
    match args.next().as_deref() {
        Some("integration") => {
            let shell = args.next().unwrap_or_default();
            if args.next().is_some() {
                usage_bail!("usage: wayexpand context integration bash|zsh|fish");
            }
            let script = match shell.as_str() {
                "bash" => BASH_INTEGRATION,
                "zsh" => ZSH_INTEGRATION,
                "fish" => FISH_INTEGRATION,
                _ => usage_bail!("usage: wayexpand context integration bash|zsh|fish"),
            };
            print!("{script}");
        }
        Some("session-id") => {
            if args.next().is_some() {
                usage_bail!("usage: wayexpand context session-id");
            }
            let mut bytes = [0u8; 16];
            getrandom::fill(&mut bytes).context("could not generate a shell session ID")?;
            for byte in bytes {
                print!("{byte:02x}");
            }
            println!();
        }
        Some("report") => report_context(args.collect())?,
        Some("status") => show_context(args.collect())?,
        Some("clear") => clear_context(args.collect())?,
        _ => usage_bail!(
            "usage: wayexpand context integration bash|zsh|fish | session-id | report [options] | status [--json] | clear"
        ),
    }
    Ok(())
}

fn report_context(args: Vec<String>) -> Result<()> {
    const USAGE: &str = "usage: wayexpand context report [--session ID] [--shell NAME] [--remote-host HOST] [--environment NAME] [--project-root PATH] [--kubernetes-context NAME] [--container NAME] [--ttl SECONDS] [--quiet]";
    let mut session_id = None;
    let mut shell = None;
    let mut remote_host = None;
    let mut environment = None;
    let mut project_root = None;
    let mut kubernetes_context = None;
    let mut container = None;
    let mut ttl_seconds = 60u32;
    let mut quiet = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--quiet" {
            quiet = true;
            index += 1;
            continue;
        }
        let (name, inline_value) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(name, value)| {
                (name, Some(value))
            });
        let value = match inline_value {
            Some(value) => value.to_owned(),
            None => {
                index += 1;
                args.get(index)
                    .cloned()
                    .ok_or_else(|| usage_error(format!("{name} requires a value")))?
            }
        };
        match name {
            "--session" => session_id = Some(value),
            "--shell" => shell = Some(value),
            "--remote-host" => remote_host = nonempty(value),
            "--environment" => environment = nonempty(value),
            "--project-root" => project_root = nonempty(value),
            "--kubernetes-context" => kubernetes_context = nonempty(value),
            "--container" => container = nonempty(value),
            "--ttl" => {
                ttl_seconds = value
                    .parse()
                    .map_err(|_| usage_error("--ttl must be 1 through 300 seconds"))?;
                if !(1..=300).contains(&ttl_seconds) {
                    usage_bail!("--ttl must be 1 through 300 seconds");
                }
            }
            _ => usage_bail!("{USAGE}"),
        }
        index += 1;
    }
    let session_id = session_id
        .or_else(|| env::var("WAYEXPAND_CONTEXT_SESSION_ID").ok())
        .ok_or_else(|| usage_error("shell integration must set WAYEXPAND_CONTEXT_SESSION_ID"))?;
    let report = wayexpand_core::SessionContextReport {
        terminal_id: terminal_id()?,
        session_id,
        shell,
        remote_host,
        environment,
        project_root,
        kubernetes_context,
        container,
    };
    report
        .validate()
        .map_err(|reason| usage_error(format!("invalid context report: {reason}")))?;
    let response = control_operation(wayexpand_core::DaemonOperation::ContextSet {
        ttl_seconds,
        report,
    })?;
    if response.trim_end() != "context updated" {
        return Err(daemon_error(response.trim().to_owned()));
    }
    if !quiet {
        println!("terminal context updated (expires in {ttl_seconds}s)");
    }
    Ok(())
}

fn show_context(args: Vec<String>) -> Result<()> {
    let json = match args.as_slice() {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => usage_bail!("usage: wayexpand context status [--json]"),
    };
    let response = context_operation(wayexpand_core::DaemonOperation::ContextGet {
        terminal_id: terminal_id()?,
        session_id: current_session_id()?,
    })?;
    let value: serde_json::Value = serde_json::from_str(&response)
        .map_err(|_| daemon_error("daemon returned an unreadable context response"))?;
    if json {
        println!("{}", serde_json::to_string(&value)?);
    } else if value.get("active").and_then(serde_json::Value::as_bool) == Some(true) {
        let context = &value["context"];
        println!(
            "terminal_id={}",
            context["terminal_id"].as_str().unwrap_or("")
        );
        for (label, key) in [
            ("shell", "shell"),
            ("remote_host", "remote_host"),
            ("environment", "environment"),
            ("project_root", "project_root"),
            ("kubernetes_context", "kubernetes_context"),
            ("container", "container"),
        ] {
            if let Some(value) = context[key].as_str() {
                println!("{label}={value}");
            }
        }
        println!(
            "expires_in_seconds={}",
            value["expires_in_seconds"].as_u64().unwrap_or_default()
        );
    } else {
        println!("no fresh context report for this terminal session");
    }
    Ok(())
}

fn clear_context(args: Vec<String>) -> Result<()> {
    if !args.is_empty() {
        usage_bail!("usage: wayexpand context clear");
    }
    let response = context_operation(wayexpand_core::DaemonOperation::ContextClear {
        terminal_id: terminal_id()?,
        session_id: current_session_id()?,
    })?;
    if response.trim_end() != "context cleared" {
        return Err(daemon_error(response.trim().to_owned()));
    }
    println!("terminal context cleared");
    Ok(())
}

fn context_operation(operation: wayexpand_core::DaemonOperation) -> Result<String> {
    let response = control_operation(operation)?;
    if response.starts_with("context refused:") || response.starts_with("context unavailable:") {
        return Err(daemon_error(response.trim().to_owned()));
    }
    Ok(response)
}

fn current_session_id() -> Result<String> {
    env::var("WAYEXPAND_CONTEXT_SESSION_ID")
        .map_err(|_| usage_error("shell integration must set WAYEXPAND_CONTEXT_SESSION_ID"))
}

fn terminal_id() -> Result<String> {
    // Use the process's controlling terminal rather than stdin: shell prompt
    // hooks and the SSH heartbeat can run with stdin redirected or detached.
    let metadata = fs::metadata("/dev/tty")
        .map_err(|_| usage_error("could not identify the local terminal device"))?;
    if !metadata.file_type().is_char_device() {
        return Err(usage_error("stdin is not a terminal device"));
    }
    Ok(format!("tty-{}-{}", metadata.dev(), metadata.rdev()))
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}
