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
    let build_warning = (requested == "status")
        .then(|| daemon_build_warning(&response, build_info::COMMIT, build_info::VERSION))
        .flatten();
    if requested_json {
        let mut status = status_as_json(&response)?;
        if requested == "status" {
            status["client_commit"] = serde_json::json!(build_info::COMMIT);
            status["client_version"] = serde_json::json!(build_info::VERSION);
            status["daemon_build_matches_client"] = serde_json::json!(daemon_build_matches(
                &response,
                build_info::COMMIT,
                build_info::VERSION
            ));
            status["daemon_build_warning"] = serde_json::json!(build_warning);
        }
        println!("{status}");
    } else {
        print!("{response}");
        if let Some(warning) = build_warning {
            eprintln!("warning: {warning}");
        }
    }
    Ok(())
}

fn daemon_build_warning(
    response: &str,
    client_commit: &str,
    client_version: &str,
) -> Option<&'static str> {
    let status = wayexpand_core::DaemonStatus::parse(response);
    if status.status_schema() != Some(CONTROL_STATUS_SCHEMA) {
        Some("the running daemon uses an incompatible status protocol; restart it before relying on its behavior")
    } else {
        match daemon_build_matches(response, client_commit, client_version) {
            Some(true) => None,
            Some(false) => Some("the running daemon is from a different build; update and restart it before relying on its behavior"),
            None => Some("the running daemon build cannot be verified; update or rebuild it with a commit ID, then restart it"),
        }
    }
}

fn daemon_build_matches(response: &str, client_commit: &str, client_version: &str) -> Option<bool> {
    let status = wayexpand_core::DaemonStatus::parse(response);
    if status.status_schema() != Some(CONTROL_STATUS_SCHEMA) || client_commit == "unknown" {
        return None;
    }
    if client_version.contains(".dirty-unknown") {
        return None;
    }
    if let Some(daemon_version) = status.field("daemon_version") {
        if daemon_version == "unknown"
            || daemon_version.contains(".dirty-unknown")
            || status
                .field("daemon_commit")
                .is_none_or(|commit| commit == "unknown")
        {
            return None;
        }
        let daemon_commit = status.field("daemon_commit")?;
        return Some(daemon_commit == client_commit && daemon_version == client_version);
    }
    // Older daemons only publish the Git commit. That is adequate for a
    // clean build, but cannot prove identity for a source tree with edits.
    if client_version.contains(".dirty.") {
        return None;
    }
    status
        .field("daemon_commit")
        .filter(|commit| *commit != "unknown")
        .map(|daemon_commit| daemon_commit == client_commit)
}

pub(crate) fn insert_command(args: impl Iterator<Item = String>) -> Result<()> {
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

#[cfg(test)]
mod status_build_tests {
    use super::{daemon_build_matches, daemon_build_warning};
    use wayexpand_core::CONTROL_STATUS_SCHEMA;

    #[test]
    fn status_warns_when_daemon_and_client_builds_do_not_match() {
        let status =
            format!("running\nstatus_schema={CONTROL_STATUS_SCHEMA}\ndaemon_commit=old-build\n");
        assert!(daemon_build_warning(&status, "new-build", "1.3.3-dev+new").is_some());
        assert!(daemon_build_warning(&status, "old-build", "1.3.3-dev+old").is_none());
        assert_eq!(
            daemon_build_matches(&status, "new-build", "1.3.3-dev+new"),
            Some(false)
        );
        assert_eq!(
            daemon_build_matches(&status, "old-build", "1.3.3-dev+old"),
            Some(true)
        );

        let dirty_version = "1.3.3-dev+old-build.dirty.1234567890abcdef";
        let same_commit_different_tree = format!(
            "running\nstatus_schema={CONTROL_STATUS_SCHEMA}\ndaemon_commit=old-build\ndaemon_version=1.3.3-dev+old-build.dirty.fedcba0987654321\n"
        );
        assert_eq!(
            daemon_build_matches(&same_commit_different_tree, "old-build", dirty_version),
            Some(false)
        );
        let matching_dirty_tree = format!(
            "running\nstatus_schema={CONTROL_STATUS_SCHEMA}\ndaemon_commit=old-build\ndaemon_version={dirty_version}\n"
        );
        assert_eq!(
            daemon_build_matches(&matching_dirty_tree, "old-build", dirty_version),
            Some(true)
        );
    }

    #[test]
    fn status_warns_when_daemon_identity_is_unavailable_or_protocol_differs() {
        let missing_identity = format!("running\nstatus_schema={CONTROL_STATUS_SCHEMA}\n");
        assert!(daemon_build_warning(&missing_identity, "new-build", "1.3.3-dev+new").is_some());
        assert_eq!(
            daemon_build_matches(&missing_identity, "new-build", "1.3.3-dev+new"),
            None
        );

        for (daemon_commit, daemon_version) in [
            ("old-build", "unknown"),
            ("old-build", "1.3.3-dev+old-build.dirty-unknown"),
            ("unknown", "1.3.3-dev+old-build"),
        ] {
            let status = format!(
                "running\nstatus_schema={CONTROL_STATUS_SCHEMA}\ndaemon_commit={daemon_commit}\ndaemon_version={daemon_version}\n"
            );
            assert!(daemon_build_warning(&status, "old-build", "1.3.3-dev+old").is_some());
            assert_eq!(
                daemon_build_matches(&status, "old-build", "1.3.3-dev+old"),
                None,
                "{daemon_version} must be unverified, not mismatched"
            );
        }

        let legacy_same_commit =
            format!("running\nstatus_schema={CONTROL_STATUS_SCHEMA}\ndaemon_commit=old-build\n");
        assert_eq!(
            daemon_build_matches(
                &legacy_same_commit,
                "old-build",
                "1.3.3-dev+old-build.dirty.1234567890abcdef"
            ),
            None
        );

        let old_protocol = "running\nstatus_schema=1\ndaemon_commit=old-build\n";
        assert!(daemon_build_warning(old_protocol, "old-build", "1.3.3-dev+old").is_some());
        assert_eq!(
            daemon_build_matches(old_protocol, "old-build", "1.3.3-dev+old"),
            None
        );
    }
}
