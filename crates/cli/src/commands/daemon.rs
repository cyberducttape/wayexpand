//! Commands that talk to the running daemon over its control socket.

use super::Args;
use crate::*;

pub(crate) fn control_command(requested: &str, mut args: Args) -> Result<()> {
    let status_argument = args.next();
    let requested_json = status_argument.as_deref() == Some("--json");
    if status_argument.is_some() && !requested_json {
        usage_bail!("usage: wayexpand {requested} [--json]");
    }
    if args.next().is_some() {
        usage_bail!("usage: wayexpand {requested} [--json]");
    }
    let operation = match requested {
        "status" => wayexpand_core::DaemonOperation::Status,
        "focus" => wayexpand_core::DaemonOperation::Focus,
        "reload" => wayexpand_core::DaemonOperation::Reload,
        "pause" => wayexpand_core::DaemonOperation::Pause,
        "resume" => wayexpand_core::DaemonOperation::Resume,
        "stop" => wayexpand_core::DaemonOperation::Stop,
        _ => return Err(daemon_error("unknown daemon operation")),
    };
    let response = control_operation(operation)?;
    if requested_json {
        println!("{}", status_as_json(&response)?);
    } else {
        print!("{response}");
    }
    Ok(())
}

pub(crate) fn insert_command(mut args: Args) -> Result<()> {
    let trigger = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand insert <trigger>"))?;
    if args.next().is_some() {
        usage_bail!("usage: wayexpand insert <trigger>");
    }
    if trigger.is_empty() || trigger.chars().any(char::is_control) {
        usage_bail!("a trigger cannot be empty or contain control characters");
    }
    let response = control_operation(wayexpand_core::DaemonOperation::Insert { trigger })?;
    if response.trim_end() != "insert scheduled" {
        return Err(daemon_error(format!(
            "daemon refused the insert: {}",
            response.trim_end()
        )));
    }
    println!("insert scheduled");
    Ok(())
}
