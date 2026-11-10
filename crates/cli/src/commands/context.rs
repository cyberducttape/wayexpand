//! Opt-in terminal context reporting and shell integrations.

use super::Args;
use crate::*;
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt},
    sync::Arc,
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
        Some("render") => render_context(args.collect())?,
        Some("clear") => clear_context(args.collect())?,
        _ => usage_bail!(
            "usage: wayexpand context integration bash|zsh|fish | session-id | report [options] | status [--json] | render <trigger> [--set NAME=VALUE]... [--json] | clear"
        ),
    }
    Ok(())
}

/// Render one static snippet using fresh shell context or explicitly selected
/// context. This is an output-only preview: command-backed entries are
/// refused, and this command never types, pastes, or executes the result.
fn render_context(args: Vec<String>) -> Result<()> {
    const USAGE: &str = "usage: wayexpand context render <trigger> [--shell bash|zsh|fish] [--remote-host TARGET] [--environment NAME] [--kubernetes-context NAME] [--set NAME=VALUE]... [--config PATH] [--json]";
    let Some(trigger) = args.first() else {
        usage_bail!("{USAGE}");
    };
    let mut config_path = default_config_path();
    let mut supplied = HashMap::new();
    let mut json = false;
    let mut shell_override = None;
    let mut remote_host_override = None;
    let mut environment_override = None;
    let mut kubernetes_context_override = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--set" => {
                index += 1;
                let assignment = args
                    .get(index)
                    .ok_or_else(|| usage_error("--set requires NAME=VALUE"))?;
                let (name, value) = assignment
                    .split_once('=')
                    .ok_or_else(|| usage_error("--set requires NAME=VALUE"))?;
                if name.is_empty() || supplied.insert(name.to_owned(), value.to_owned()).is_some() {
                    usage_bail!("--set field names must be non-empty and unique");
                }
            }
            "--config" => {
                index += 1;
                config_path = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| usage_error("--config requires a path"))?,
                );
            }
            "--shell" => {
                index += 1;
                shell_override = Some(
                    args.get(index)
                        .ok_or_else(|| usage_error("--shell requires a value"))?
                        .clone(),
                );
            }
            "--remote-host" => {
                index += 1;
                remote_host_override = Some(
                    args.get(index)
                        .ok_or_else(|| usage_error("--remote-host requires a value"))?
                        .clone(),
                );
            }
            "--environment" => {
                index += 1;
                environment_override = Some(
                    args.get(index)
                        .ok_or_else(|| usage_error("--environment requires a value"))?
                        .clone(),
                );
            }
            "--kubernetes-context" => {
                index += 1;
                kubernetes_context_override = Some(
                    args.get(index)
                        .ok_or_else(|| usage_error("--kubernetes-context requires a value"))?
                        .clone(),
                );
            }
            option if option.starts_with("--set=") => {
                let assignment = &option["--set=".len()..];
                let (name, value) = assignment
                    .split_once('=')
                    .ok_or_else(|| usage_error("--set requires NAME=VALUE"))?;
                if name.is_empty() || supplied.insert(name.to_owned(), value.to_owned()).is_some() {
                    usage_bail!("--set field names must be non-empty and unique");
                }
            }
            option if option.starts_with("--config=") => {
                config_path = PathBuf::from(&option["--config=".len()..]);
            }
            option if option.starts_with("--shell=") => {
                shell_override = Some(option["--shell=".len()..].to_owned());
            }
            option if option.starts_with("--remote-host=") => {
                remote_host_override = Some(option["--remote-host=".len()..].to_owned());
            }
            option if option.starts_with("--environment=") => {
                environment_override = Some(option["--environment=".len()..].to_owned());
            }
            option if option.starts_with("--kubernetes-context=") => {
                kubernetes_context_override =
                    Some(option["--kubernetes-context=".len()..].to_owned());
            }
            _ => usage_bail!("{USAGE}"),
        }
        index += 1;
    }

    validate_context_selection(
        kubernetes_context_override.as_deref(),
        supplied.get("field:context").map(String::as_str),
    )?;
    if let Some(context) = kubernetes_context_override.as_deref() {
        validate_kubernetes_context(context)?;
    }

    let config =
        Config::load(&config_path).map_err(|error| config_load_error(&config_path, error))?;
    let expansion = config
        .expansion
        .iter()
        .find(|expansion| expansion.answers_to(trigger))
        .ok_or_else(|| usage_error("no snippet has that trigger or alias"))?;
    if !expansion.enabled {
        usage_bail!("the selected snippet is disabled");
    }
    if !expansion.is_keyboard_matchable() {
        usage_bail!(
            "the selected entry is generate-only; render it with `wayexpand new {}`",
            expansion.trigger
        );
    }
    if expansion.command.is_some() {
        usage_bail!(
            "context render only renders text snippets; command-backed snippets are never run"
        );
    }
    let fields = wayexpand_core::form_fields(&expansion.replacement)
        .map_err(|error| config_error(format!("snippet form is invalid: {error}")))?;
    let missing_context_form = fields.iter().any(|field| {
        field.key == "field:context"
            && !supplied.contains_key(&field.key)
            && kubernetes_context_override.is_none()
    });

    // Ask the daemon only for context variables or a Kubernetes context form
    // this template actually uses. Explicit values keep previews useful
    // without a running daemon and prevent unrelated shell/SSH context from
    // becoming a hidden dependency.
    let mut session = reported_context_if_available(context_report_needed(
        &expansion.replacement,
        shell_override.is_some(),
        remote_host_override.is_some(),
        environment_override.is_some(),
        kubernetes_context_override.is_some(),
        missing_context_form,
    ))?
    .unwrap_or_else(|| wayexpand_core::SessionContextReport {
        terminal_id: "tty-render".into(),
        session_id: "0123456789abcdef0123456789abcdef".into(),
        ..wayexpand_core::SessionContextReport::default()
    });
    if let Some(shell) = shell_override {
        session.shell = Some(shell);
    }
    if let Some(remote_host) = remote_host_override {
        session.remote_host = Some(remote_host);
    }
    if let Some(environment) = environment_override {
        session.environment = Some(environment);
    }
    if let Some(kubernetes_context) = kubernetes_context_override {
        session.kubernetes_context = Some(kubernetes_context);
    }
    session
        .validate()
        .map_err(|reason| usage_error(format!("invalid session context: {reason}")))?;
    let shell = session
        .shell
        .as_deref()
        .filter(|shell| matches!(*shell, "bash" | "zsh" | "fish"))
        .ok_or_else(|| {
            usage_error("context rendering supports the reported bash, zsh, or fish shell only")
        })?;

    let mut values = wayexpand_core::FormValues::default();
    for field in &fields {
        let value = supplied
            .remove(&field.key)
            .or_else(|| context_form_prefill(&field.key, &session))
            .or_else(|| match &field.kind {
                wayexpand_core::FormFieldKind::Text { default }
                | wayexpand_core::FormFieldKind::Date { default } => {
                    (!default.is_empty()).then(|| default.clone())
                }
                wayexpand_core::FormFieldKind::Choice { default, .. } => default.clone(),
            });
        let value = match value {
            Some(value) => value,
            None => prompt_context_field(field)?,
        };
        field
            .validate_value(&value)
            .map_err(|_| usage_error(format!("value for field {:?} is invalid", field.label)))?;
        values.insert(field.key.clone(), value);
    }
    if !supplied.is_empty() {
        usage_bail!("--set names a field that does not exist in the selected snippet");
    }
    if let Some(context) = values.get("field:context") {
        session.kubernetes_context = Some(context.clone());
    }
    if let Some(context) = session.kubernetes_context.as_deref() {
        validate_kubernetes_context(context)?;
    }
    let mut context = config.template_context(None);
    context.fields = Arc::new(values);
    context.session_context = Some(session.clone());
    let (rendered, _) =
        wayexpand_core::render_template_with_cursor(&expansion.replacement, &context).map_err(
            |error| {
                config_error(format!(
                    "snippet could not be rendered: {}",
                    error.safe_summary()
                ))
            },
        )?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "rendered": rendered.as_str(),
                "trigger": expansion.trigger,
                "shell": shell,
                "remote_host": session.remote_host,
                "environment": session.environment,
                "kubernetes_context": session.kubernetes_context,
                "target_verification": "unverified",
                "delivery": "manual-review-and-paste",
                "execution": "never",
            })
        );
    } else {
        eprintln!(
            "Preview only · shell={shell} · target={} · environment={} · Kubernetes target (unverified)={} · review before pasting; WayExpand will not execute this text",
            session.remote_host.as_deref().unwrap_or("local"),
            session.environment.as_deref().unwrap_or("unspecified"),
            session.kubernetes_context.as_deref().unwrap_or("unspecified"),
        );
        print!("{}", rendered.as_str());
        if !rendered.as_str().ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

fn context_variable_name(variable: &str) -> Option<&str> {
    variable
        .strip_prefix("shellq:")
        .unwrap_or(variable)
        .strip_prefix("context:")
}

fn context_form_prefill(
    field_key: &str,
    session: &wayexpand_core::SessionContextReport,
) -> Option<String> {
    (field_key == "field:context")
        .then(|| session.kubernetes_context.clone())
        .flatten()
}

fn validate_context_selection(
    option_context: Option<&str>,
    field_context: Option<&str>,
) -> Result<()> {
    if let (Some(option), Some(field)) = (option_context, field_context) {
        if option != field {
            bail!(
                "conflicting Kubernetes contexts: --kubernetes-context and --set field:context must match"
            );
        }
    }
    Ok(())
}

fn validate_kubernetes_context(context: &str) -> Result<()> {
    if context.trim().is_empty()
        || context.chars().any(char::is_whitespace)
        || context.chars().any(char::is_control)
    {
        bail!(
            "Kubernetes context must be non-empty and contain no whitespace or control characters"
        );
    }
    Ok(())
}

fn context_report_needed(
    template: &str,
    shell_supplied: bool,
    remote_host_supplied: bool,
    environment_supplied: bool,
    kubernetes_context_supplied: bool,
    missing_context_form: bool,
) -> bool {
    missing_context_form
        || !shell_supplied
        || wayexpand_core::template_variables(template)
            .iter()
            .filter_map(|variable| context_variable_name(variable))
            .any(|name| match name {
                "shell" => !shell_supplied,
                "remote_host" => !remote_host_supplied,
                "environment" => !environment_supplied,
                "kubernetes_context" => !kubernetes_context_supplied,
                "project_root" | "container" => true,
                _ => false,
            })
}

fn reported_context_if_available(
    should_query: bool,
) -> Result<Option<wayexpand_core::SessionContextReport>> {
    if !should_query {
        return Ok(None);
    }
    let (Ok(session_id), Ok(terminal_id)) =
        (env::var("WAYEXPAND_CONTEXT_SESSION_ID"), terminal_id())
    else {
        return Ok(None);
    };
    let response = context_operation(wayexpand_core::DaemonOperation::ContextGet {
        terminal_id,
        session_id,
    })?;
    let status: serde_json::Value = serde_json::from_str(&response)
        .map_err(|_| daemon_error("daemon returned an unreadable context response"))?;
    context_from_status(&status)
}

fn context_from_status(
    status: &serde_json::Value,
) -> Result<Option<wayexpand_core::SessionContextReport>> {
    if status.get("active").and_then(serde_json::Value::as_bool) != Some(true) {
        return Ok(None);
    }
    if status
        .get("expires_in_seconds")
        .and_then(serde_json::Value::as_u64)
        .is_none_or(|seconds| seconds == 0)
    {
        return Ok(None);
    }
    let report: wayexpand_core::SessionContextReport = serde_json::from_value(
        status
            .get("context")
            .cloned()
            .ok_or_else(|| daemon_error("daemon omitted the terminal context"))?,
    )
    .map_err(|_| daemon_error("daemon returned an invalid terminal context"))?;
    report
        .validate()
        .map_err(|_| daemon_error("daemon returned an invalid terminal context"))?;
    Ok(Some(report))
}

fn prompt_context_field(field: &wayexpand_core::FormField) -> Result<String> {
    use std::io::{BufRead, Write};
    let mut terminal = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            usage_error("missing snippet fields; pass --set NAME=VALUE or run from a terminal")
        })?;
    write!(terminal, "{}: ", field.label)?;
    terminal.flush()?;
    let mut value = String::new();
    std::io::BufReader::new(terminal).read_line(&mut value)?;
    while value.ends_with(['\n', '\r']) {
        value.pop();
    }
    if value.is_empty() {
        usage_bail!(
            "field {:?} needs a value; pass --set NAME=VALUE",
            field.label
        );
    }
    Ok(value)
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

#[cfg(test)]
mod tests {
    use super::{
        context_form_prefill, context_from_status, context_report_needed, context_variable_name,
        validate_context_selection, validate_kubernetes_context,
    };

    #[test]
    fn detects_context_dependencies_with_or_without_shell_quoting() {
        assert_eq!(
            context_variable_name("context:remote_host"),
            Some("remote_host")
        );
        assert_eq!(
            context_variable_name("shellq:context:kubernetes_context"),
            Some("kubernetes_context")
        );
        assert_eq!(
            context_variable_name("shellq:field:namespace=default"),
            None
        );
        assert_eq!(context_variable_name("clipboard"), None);
    }

    #[test]
    fn explicit_kubernetes_preview_does_not_depend_on_daemon_context() {
        let template = "kubectl --context {{shellq:context:kubernetes_context}} logs";
        assert!(!context_report_needed(
            template, true, false, false, true, false
        ));
        assert!(context_report_needed(
            template, true, false, false, false, false
        ));
        assert!(context_report_needed(
            template, false, false, false, true, false
        ));
        assert!(context_report_needed(
            "{{shellq:context|required}}",
            true,
            false,
            false,
            false,
            true
        ));
        assert!(!context_report_needed(
            "{{shellq:context|required}}",
            true,
            false,
            false,
            true,
            false
        ));
    }

    #[test]
    fn fresh_kubernetes_context_prefills_only_the_context_form() {
        let session = wayexpand_core::SessionContextReport {
            kubernetes_context: Some("staging-west".into()),
            ..wayexpand_core::SessionContextReport::default()
        };
        assert_eq!(
            context_form_prefill("field:context", &session),
            Some("staging-west".into())
        );
        assert_eq!(context_form_prefill("field:namespace", &session), None);
    }

    #[test]
    fn explicit_and_form_contexts_must_not_conflict() {
        assert!(validate_context_selection(Some("staging"), Some("staging")).is_ok());
        assert!(validate_context_selection(Some("staging"), Some("production")).is_err());
        assert!(validate_context_selection(Some("staging"), None).is_ok());
    }

    #[test]
    fn kubernetes_context_rejects_empty_whitespace_and_controls() {
        for invalid in [
            "",
            "   ",
            "staging production",
            "staging\tproduction",
            "staging\nproduction",
            "staging\u{1b}[31m",
        ] {
            assert!(validate_kubernetes_context(invalid).is_err(), "{invalid:?}");
        }
        assert!(validate_kubernetes_context("staging-west").is_ok());
    }

    #[test]
    fn expired_or_inactive_session_context_never_prefills_a_cluster() {
        let inactive = serde_json::json!({
            "active": false,
            "expires_in_seconds": 0,
        });
        assert!(context_from_status(&inactive).unwrap().is_none());
        let expired = serde_json::json!({
            "active": true,
            "expires_in_seconds": 0,
            "context": { "kubernetes_context": "production" },
        });
        assert!(context_from_status(&expired).unwrap().is_none());
        let valid = serde_json::json!({
            "active": true,
            "expires_in_seconds": 10,
            "context": serde_json::to_value(wayexpand_core::SessionContextReport {
                terminal_id: "tty-1-2".into(),
                session_id: "0123456789abcdef0123456789abcdef".into(),
                kubernetes_context: Some("staging".into()),
                ..wayexpand_core::SessionContextReport::default()
            })
            .unwrap(),
        });
        assert_eq!(
            context_from_status(&valid)
                .unwrap()
                .and_then(|session| session.kubernetes_context),
            Some("staging".into())
        );
        let placeholder = wayexpand_core::SessionContextReport::default();
        assert_eq!(context_form_prefill("field:context", &placeholder), None);
    }
}
