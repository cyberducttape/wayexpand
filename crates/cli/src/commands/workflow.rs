//! Explicit safety-aware workflows for insertion, artifact generation, and
//! broker-authorized operations.

use super::{daemon::insert_command, new::new_command};
use crate::{args::take_option, *};
use std::{
    collections::BTreeMap,
    io::IsTerminal,
    time::{Duration, Instant},
};
use wayexpand_broker_client::{ActionRequest, ActionResponse, BrokerClient};

const USAGE: &str = "usage: wayexpand workflow insert <trigger> | generate <template> [--set NAME=VALUE]... [-o PATH] [--force] [--config FILE] | run <ACTION_ID> [--set NAME=VALUE]... [--approve-action ACTION_ID] [--config FILE]";

pub(crate) fn workflow_command(args: impl Iterator<Item = String>) -> Result<()> {
    let mut args: Vec<String> = args.collect();
    let Some(mode) = (!args.is_empty()).then(|| args.remove(0)) else {
        usage_bail!("{USAGE}");
    };
    match mode.as_str() {
        "insert" => {
            if args.len() != 1 {
                usage_bail!("{USAGE}");
            }
            println!(
                "Mode: Insert\nTarget: the currently focused window (verified at insertion time)"
            );
            insert_command(["--verified".to_owned(), args.remove(0)].into_iter())
        }
        "generate" => generate(args),
        "run" => run_approved_action(args),
        _ => usage_bail!("unknown workflow mode {mode:?}; {USAGE}"),
    }
}

fn generate(mut args: Vec<String>) -> Result<()> {
    if args.is_empty() || args.iter().any(|arg| arg == "--stdout") {
        usage_bail!("Generate File requires a template and a file destination; {USAGE}");
    }
    println!("Mode: Generate File\nReview: rendered diff and destination, validate with the registered tool, then confirm writing");
    if !args.iter().any(|arg| arg == "--validate") {
        args.push("--validate".into());
    }
    args.push("--review".into());
    new_command(args.into_iter())
}

fn run_approved_action(mut args: Vec<String>) -> Result<()> {
    let approved = take_option(&mut args, "--approve-action")
        .map_err(|error| usage_error(error.to_string()))?;
    let config_path = take_option(&mut args, "--config")
        .map_err(|error| usage_error(error.to_string()))?
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let mut parameters = BTreeMap::new();
    while let Some(value) =
        take_option(&mut args, "--set").map_err(|error| usage_error(error.to_string()))?
    {
        let Some((key, value)) = value.split_once('=') else {
            usage_bail!("--set needs NAME=VALUE");
        };
        let key = key.trim();
        if key.is_empty()
            || key.chars().any(char::is_control)
            || parameters
                .insert(key.to_owned(), value.to_owned())
                .is_some()
        {
            usage_bail!("action parameter names must be non-empty and unique");
        }
    }
    if args.len() != 1 || args[0].starts_with('-') {
        usage_bail!("{USAGE}");
    }
    let action_id = args.remove(0);
    if action_id.is_empty()
        || action_id.len() > wayexpand_broker_client::MAX_ACTION_ID_BYTES
        || action_id.chars().any(char::is_control)
    {
        usage_bail!("action IDs must be non-empty, bounded, and contain no control characters");
    }
    if approved.as_deref().is_some_and(|value| value != action_id) {
        usage_bail!("--approve-action must exactly match the requested action ID");
    }
    let config =
        Config::load(&config_path).map_err(|error| config_load_error(&config_path, error))?;
    let config_policy = config.organization.effective_enforcement_policy();
    let system_policy = crate::doctor::policy::load_policy()?.effective_enforcement_policy();
    if config_policy.disable_commands || system_policy.disable_commands {
        bail!("organization policy disables command actions");
    }
    let broker_health = crate::doctor::broker::broker_diagnostics(Some(&config));
    let audit = &broker_health["audit"];
    if !mandatory_audit_ready(audit) {
        bail!("Run Approved Action requires a healthy Action Broker with mandatory audit enabled; inspect `wayexpand doctor --json` and configure audit_path plus audit_required = true");
    }
    let matches: Vec<_> = config
        .expansion
        .iter()
        .filter(|snippet| snippet.enabled)
        .filter_map(|snippet| {
            snippet
                .command
                .as_ref()
                .filter(|command| command.action.as_deref() == Some(action_id.as_str()))
                .map(|command| (snippet, command))
        })
        .collect();
    let [(snippet, command)] = matches.as_slice() else {
        bail!("action must match exactly one enabled, configured named Action Broker snippet");
    };
    if command.action.is_none() || !command.program.is_empty() {
        bail!("only named Action Broker actions can run in this workflow");
    }
    let trigger = escape_terminal(&snippet.trigger);
    let purpose = if snippet.description.trim().is_empty() {
        "(no description)".to_owned()
    } else {
        escape_terminal(snippet.description.trim())
    };
    print!("Mode: Run Approved Action\nAction ID: {action_id}\nSnippet: {trigger}\nPurpose: {purpose}\nBroker: configured Action Broker (policy validates permissions)\nParameters: {}\n",
        if parameters.is_empty() { "none".to_owned() } else { parameters.keys().cloned().collect::<Vec<_>>().join(", ") });
    println!("Execution is separate from text insertion. The broker validates the action and its parameter schema.");
    confirm_action(&action_id, approved.as_deref())?;

    let socket = std::env::var_os("WAYEXPAND_ACTION_BROKER_SOCKET")
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|dir| {
                PathBuf::from(dir)
                    .join("wayexpand-broker.sock")
                    .into_os_string()
            })
        })
        .context("Action Broker socket is not configured")?;
    let mut client = BrokerClient::connect(socket).context("could not connect to Action Broker")?;
    let timeout = Duration::from_millis(command.timeout_ms);
    client
        .set_io_timeout(timeout)
        .context("could not set Action Broker timeout")?;
    let request = ActionRequest {
        action_id: action_id.clone(),
        timeout_ms: command.timeout_ms,
        inherit_env: false,
        env_vars: Vec::new(),
        stdout_capture: true,
        parameters,
    };
    client
        .send_request(&request)
        .context("could not submit approved action")?;
    let deadline = Instant::now() + timeout;
    match client
        .recv_response_until(deadline, None)
        .context("Action Broker did not return an outcome")?
    {
        ActionResponse::Success(output) => {
            println!("Action outcome: success\nExit code: {}\nDuration: {} ms\nAudit: mandatory broker completion record confirmed.", output.exit_code, output.duration_ms);
            if output.stdout_truncated || output.stderr_truncated {
                println!("Output: truncated by broker limits");
            }
            if output.exit_code != 0 {
                bail!(
                    "approved action {action_id:?} returned exit code {}",
                    output.exit_code
                );
            }
        }
        ActionResponse::Error(error) => {
            bail!("Action Broker rejected or failed action {action_id:?}: {error:?}")
        }
    }
    Ok(())
}

fn escape_terminal(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn confirm_action(action_id: &str, approved: Option<&str>) -> Result<()> {
    if approved == Some(action_id) {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail!("non-interactive execution requires --approve-action {action_id}");
    }
    eprint!("To approve, type `run {action_id}`: ");
    std::io::Write::flush(&mut std::io::stderr())?;
    let mut answer = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut answer)?;
    if answer.trim() != format!("run {action_id}") {
        bail!("action cancelled before execution");
    }
    Ok(())
}

fn mandatory_audit_ready(audit: &serde_json::Value) -> bool {
    audit["audit_enabled"] == true
        && audit["audit_required"] == true
        && audit["audit_healthy"] == true
}

#[cfg(test)]
mod tests {
    use super::mandatory_audit_ready;

    #[test]
    fn approved_actions_require_enabled_required_and_healthy_audit() {
        assert!(mandatory_audit_ready(&serde_json::json!({
            "audit_enabled": true,
            "audit_required": true,
            "audit_healthy": true
        })));
        for audit in [
            serde_json::json!({"audit_enabled": false, "audit_required": true, "audit_healthy": true}),
            serde_json::json!({"audit_enabled": true, "audit_required": false, "audit_healthy": true}),
            serde_json::json!({"audit_enabled": true, "audit_required": true, "audit_healthy": false}),
            serde_json::json!({}),
        ] {
            assert!(!mandatory_audit_ready(&audit));
        }
    }
}
