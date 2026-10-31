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
    if let Some(message) = response
        .strip_prefix("error=policy\nmessage=")
        .map(str::trim)
    {
        return Err(daemon_error(message.to_owned()));
    }
    if requested_json {
        println!("{}", status_as_json(&response)?);
    } else {
        print!("{response}");
    }
    Ok(())
}

pub(crate) fn insert_command(args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand insert [--verified] <trigger>";
    let (verified, trigger) = match args.collect::<Vec<_>>().as_slice() {
        [trigger] if trigger != "--verified" => (false, trigger.clone()),
        [flag, trigger] if flag == "--verified" => (true, trigger.clone()),
        _ => usage_bail!("{USAGE}"),
    };
    if trigger.is_empty() || trigger.chars().any(char::is_control) {
        usage_bail!("a trigger cannot be empty or contain control characters");
    }
    let operation = if verified {
        // Bind the insert to the window focused now; the daemon refuses it
        // if focus has moved by the time it would type.
        let response = control_operation(wayexpand_core::DaemonOperation::Focus)?;
        let focus = wayexpand_core::DaemonFocus::parse(&response)
            .ok_or_else(|| daemon_error("daemon returned an unreadable focus response"))?;
        let (generation, token) = focus.verified_target().ok_or_else(|| {
            daemon_error(
                "cannot verify the insert target: the window tracker does not report an exact \
                 identity for the focused window",
            )
        })?;
        wayexpand_core::DaemonOperation::InsertTarget {
            generation,
            token: token.to_owned(),
            trigger,
        }
    } else {
        wayexpand_core::DaemonOperation::Insert { trigger }
    };
    let response = control_operation(operation)?;
    if response.trim_end() != "insert scheduled" {
        return Err(daemon_error(format!(
            "daemon refused the insert: {}",
            response.trim_end()
        )));
    }
    println!("insert scheduled");
    Ok(())
}
