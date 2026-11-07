//! CI-oriented pack linting and fixture tests. Pack imports strip executable
//! entries before a fixture is rendered, so these checks never run actions.

use super::config::resolve_pack_path;
use crate::args::take_flag;
use crate::*;
use serde::Deserialize;
use std::{collections::BTreeMap, path::Component, sync::Arc};
use wayexpand_core::{
    form_fields, import_pack, inspect_pack, render_template_with_cursor, template_variables,
    ExpansionEngine, FormFieldKind, FormFieldValidation, FormValues, InputEvent, TemplateContext,
};

const OPERATIONAL_COMMANDS: &[&str] = &[
    "ansible",
    "ansible-playbook",
    "ansible-inventory",
    "apt",
    "apt-get",
    "apk",
    "awk",
    "aws",
    "az",
    "bash",
    "cat",
    "certbot",
    "cpapi2",
    "curl",
    "docker",
    "dig",
    "dnf",
    "echo",
    "firewall-cmd",
    "firewalld",
    "find",
    "gcloud",
    "git",
    "gh",
    "grep",
    "helm",
    "iptables",
    "journalctl",
    "ip",
    "kubectl",
    "kustomize",
    "mariadb",
    "mysql",
    "mysqladmin",
    "nc",
    "nginx",
    "nft",
    "nomad",
    "openssl",
    "pacman",
    "pg_dump",
    "printf",
    "podman",
    "psql",
    "resolvectl",
    "redis-cli",
    "service",
    "sudo",
    "rpm",
    "rsync",
    "sed",
    "sh",
    "ss",
    "ssh",
    "systemctl",
    "systemd-run",
    "tee",
    "terraform",
    "ufw",
    "uapi",
    "vault",
    "whmapi1",
    "yum",
    "zypper",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestFile {
    format_version: u32,
    case: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestCase {
    trigger: String,
    expected_output: String,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    values: BTreeMap<String, String>,
    #[serde(default)]
    context: Option<TestContext>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestContext {
    shell: String,
    #[serde(default)]
    remote_host: Option<String>,
    #[serde(default)]
    environment: Option<String>,
    #[serde(default)]
    project_root: Option<String>,
    #[serde(default)]
    kubernetes_context: Option<String>,
    #[serde(default)]
    container: Option<String>,
}

pub(crate) fn pack_quality_command(action: &str, args: Vec<String>) -> Result<()> {
    let mut args = args;
    let json = take_json_flag(&mut args);
    let require_signature = take_flag(&mut args, "--require-signature");
    let signers =
        take_option(&mut args, "--signers").map_err(|error| usage_error(error.to_string()))?;
    if args.len() != 1 || args[0].starts_with('-') {
        usage_bail!("usage: wayexpand pack {action} <directory> [--json] [--require-signature] [--signers FILE]");
    }
    if action == "test" && (require_signature || signers.is_some()) {
        usage_bail!("pack test accepts only --json");
    }
    let path = resolve_pack_path(Path::new(&args[0]));
    match action {
        "lint" => lint_pack(&path, json, require_signature, signers),
        "test" => test_pack(&path, json),
        _ => usage_bail!("unknown pack quality command {action:?}"),
    }
}

fn lint_pack(
    path: &Path,
    json: bool,
    require_signature: bool,
    signers_override: Option<String>,
) -> Result<()> {
    let inspection = match inspect_pack(path) {
        Ok(inspection) => inspection,
        Err(error) => {
            let report = serde_json::json!({
                "schema_version": 1,
                "command": "pack lint",
                "ok": false,
                "exit_code": 1,
                "path": path,
                "checks": [{"name": "pack structure", "status": "fail", "message": error.to_string()}],
            });
            if json {
                println!("{report}");
            }
            bail!("pack lint failed: {error}");
        }
    };
    let manifest = &inspection.manifest;
    let mut checks = Vec::new();
    add_check(
        &mut checks,
        "pack structure",
        true,
        "manifest and snippet files parse; declared capabilities match content",
    );
    let config_check = import_pack(path).map(|_| ());
    add_check(
        &mut checks,
        "merged configuration",
        config_check.is_ok(),
        config_check
            .as_ref()
            .err()
            .map(|error| format!("merged snippets are invalid: {error}"))
            .as_deref()
            .unwrap_or("the merged static snippet configuration is valid"),
    );
    let declared_tools = manifest
        .compatibility
        .as_ref()
        .map(|compatibility| compatibility.tools.as_slice())
        .unwrap_or_default();
    let operational_fields = operational_field_findings(&inspection.expansions, declared_tools);
    let operational_field_message = if operational_fields.is_empty() {
        "operational fields have a domain validator, constrained choices, or explicit free_text=true justification".to_owned()
    } else {
        format!(
            "{} operational field(s) need validate=... or explicit free_text=true; see field_findings",
            operational_fields.len()
        )
    };
    add_check(
        &mut checks,
        "operational field validation",
        operational_fields.is_empty(),
        &operational_field_message,
    );
    add_check(
        &mut checks,
        "maintainer",
        manifest
            .maintainer
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty()),
        "maintainer metadata is required for reviewed distribution",
    );
    add_check(
        &mut checks,
        "compatibility",
        manifest.compatibility.is_some(),
        "shell, platform, and tool compatibility metadata is required",
    );
    add_check(
        &mut checks,
        "risk classification",
        manifest.risk.is_some(),
        "risk level and summary are required",
    );
    add_check(
        &mut checks,
        "review record",
        manifest.review.is_some(),
        "reviewer, date, and review summary are required",
    );
    let trust = match super::pack_trust::PackTrust::load(signers_override) {
        Ok(trust) => trust,
        Err(error) => {
            add_check(
                &mut checks,
                "organization policy",
                false,
                &format!("could not load trusted pack policy: {error:#}"),
            );
            let report = serde_json::json!({
                "schema_version": 1,
                "command": "pack lint",
                "ok": false,
                "exit_code": 3,
                "pack": {"id": manifest.id, "version": manifest.version, "fingerprint": inspection.fingerprint, "path": path},
                "signature": {"status":"not_checked"},
                "checks": checks,
                "field_findings": operational_fields,
            });
            if json {
                println!("{report}");
            }
            return Err(error);
        }
    };
    let fixture_result = lint_fixture(path, manifest.verification.as_ref(), &inspection);
    add_check(
        &mut checks,
        "test fixture",
        fixture_result.is_ok(),
        match &fixture_result {
            Ok(message) | Err(message) => message,
        },
    );
    let policy_result = trust.admit(&inspection, true);
    add_check(
        &mut checks,
        "organization policy",
        policy_result.is_ok(),
        policy_result
            .as_ref()
            .err()
            .map(|error| format!("{error:#}"))
            .as_deref()
            .unwrap_or("pack contents are admitted by the active organization policy"),
    );
    let signature = match trust.verify(path) {
        Ok(Some(signer)) => serde_json::json!({"status":"verified", "signer": signer}),
        Ok(None) if require_signature => {
            add_check(
                &mut checks,
                "signature",
                false,
                "a trusted pack signature is required by --require-signature",
            );
            serde_json::json!({"status":"required_missing_or_unverified"})
        }
        Ok(None) => {
            add_check(
                &mut checks,
                "signature",
                true,
                "signature is optional under current policy; verify before deployment",
            );
            serde_json::json!({"status":"unsigned_or_not_required"})
        }
        Err(error) => {
            add_check(
                &mut checks,
                "signature",
                false,
                &format!("signature verification failed: {error}"),
            );
            serde_json::json!({"status":"failed", "reason": error.to_string()})
        }
    };
    let ok = checks.iter().all(|check| check["status"] == "pass");
    let report = serde_json::json!({
        "schema_version": 1,
        "command": "pack lint",
        "ok": ok,
        "exit_code": if ok { 0 } else { 1 },
        "pack": {"id": manifest.id, "version": manifest.version, "fingerprint": inspection.fingerprint, "path": path},
        "signature": signature,
        "checks": checks,
        "field_findings": operational_fields,
    });
    if json {
        println!("{report}");
    } else {
        println!("Pack lint: {} {}", manifest.id, manifest.version);
        for check in report["checks"].as_array().into_iter().flatten() {
            println!(
                "{}  {} — {}",
                if check["status"] == "pass" {
                    "PASS"
                } else {
                    "FAIL"
                },
                check["name"].as_str().unwrap_or("check"),
                check["message"].as_str().unwrap_or("")
            );
        }
        println!("Fingerprint: {}", inspection.fingerprint);
        println!(
            "Signature: {}",
            signature["status"].as_str().unwrap_or("unknown")
        );
    }
    if !ok {
        bail!("pack lint failed; see checks above");
    }
    Ok(())
}

fn operational_field_findings(
    expansions: &[wayexpand_core::PackExpansionReview],
    declared_tools: &[String],
) -> Vec<serde_json::Value> {
    expansions
        .iter()
        .filter(|expansion| looks_operational_with_tools(&expansion.replacement, declared_tools))
        .flat_map(|expansion| {
            let shell_context = has_shell_code_fence(&expansion.replacement)
                || contains_known_command(
                    &expansion.replacement.to_ascii_lowercase(),
                    OPERATIONAL_COMMANDS,
                    declared_tools,
                );
            let raw_fields = raw_field_interpolations(&expansion.replacement);
            let mut findings: Vec<_> = form_fields(&expansion.replacement)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|field| {
                    let choice = matches!(field.kind, FormFieldKind::Choice { .. });
                    let missing_review = field.validation.is_none() && !field.free_text && !choice;
                    let raw_shell_injection = shell_context
                        && raw_fields.contains(&field.key)
                        && !field
                            .validation
                            .is_some_and(shell_token_safe_validation);
                    if !missing_review && !raw_shell_injection {
                        return None;
                    }
                    let reason = match (missing_review, raw_shell_injection) {
                        (true, true) => "operational field needs a domain validator or free_text=true, and shell command values must use {{shellq:...}} or a shell-token-safe validator",
                        (true, false) => "operational interpolation needs a domain validator or explicit free_text=true justification",
                        (false, true) => "raw shell interpolation can change command meaning; use {{shellq:...}} or a shell-token-safe validator",
                        (false, false) => unreachable!(),
                    };
                    Some(serde_json::json!({
                        "file": expansion.file,
                        "trigger": expansion.trigger,
                        "field": field.label,
                        "key": field.key,
                        "reason": reason,
                    }))
                })
                .collect();
            if shell_context {
                for variable in template_variables(&expansion.replacement) {
                    let unsafe_source = variable == "clipboard"
                        || variable.starts_with("env:")
                        || variable.starts_with("context:");
                    if unsafe_source {
                        findings.push(serde_json::json!({
                            "file": expansion.file,
                            "trigger": expansion.trigger,
                            "field": variable,
                            "reason": "unquoted environment, clipboard, or session context can change shell command meaning; use {{shellq:env:NAME}}, {{shellq:context:FIELD}}, or a validated form field",
                        }));
                    }
                }
            }
            findings
        })
        .collect()
}

fn raw_field_interpolations(template: &str) -> std::collections::HashSet<String> {
    let mut raw_fields = std::collections::HashSet::new();
    for variable in template_variables(template) {
        if !variable.starts_with("field:") && !variable.starts_with("prompt:") {
            continue;
        }
        let wrapped = format!("{{{{{variable}}}}}");
        let Ok(fields) = form_fields(&wrapped) else {
            continue;
        };
        let Some(field) = fields.first() else {
            continue;
        };
        raw_fields.insert(field.key.clone());
    }
    raw_fields
}

fn shell_token_safe_validation(validation: FormFieldValidation) -> bool {
    matches!(
        validation,
        FormFieldValidation::DnsLabel
            | FormFieldValidation::DnsSubdomain
            | FormFieldValidation::Hostname
            | FormFieldValidation::Port
            | FormFieldValidation::SystemdUnit
            | FormFieldValidation::PositiveInteger
            | FormFieldValidation::LinuxUser
            | FormFieldValidation::AnsibleRoleName
            | FormFieldValidation::ComposeService
            | FormFieldValidation::ContainerImage
            | FormFieldValidation::KubernetesName
            | FormFieldValidation::KubernetesDuration
            | FormFieldValidation::Duration
            | FormFieldValidation::ContainerName
    )
}

fn has_shell_code_fence(replacement: &str) -> bool {
    const SHELL_FORMATS: &[&str] = &[
        "bash", "console", "dash", "fish", "ksh", "sh", "shell", "zsh",
    ];
    replacement.lines().any(|line| {
        let line = line.trim_start();
        let Some(marker @ ('`' | '~')) = line.chars().next() else {
            return false;
        };
        let length = line
            .chars()
            .take_while(|character| *character == marker)
            .count();
        if length < 3 {
            return false;
        }
        let language = line[marker.len_utf8() * length..]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        SHELL_FORMATS.contains(&language.as_str())
    })
}

fn looks_operational_with_tools(replacement: &str, declared_tools: &[String]) -> bool {
    let lower = replacement.to_ascii_lowercase();
    has_operational_code_fence(&lower)
        || lower.contains("{{shellq:")
        || contains_known_command(&lower, OPERATIONAL_COMMANDS, declared_tools)
        || lower.lines().any(|line| {
            let line = line.trim_start();
            ["select ", "insert into ", "update ", "delete from "]
                .iter()
                .any(|statement| line.starts_with(statement))
                || ["execstart=", "listen ", "image:", "replicas:", "hosts:"]
                    .iter()
                    .any(|directive| line.starts_with(directive))
        })
}

fn contains_known_command(replacement: &str, commands: &[&str], declared_tools: &[String]) -> bool {
    const WRAPPERS: &[&str] = &[
        "command", "doas", "env", "exec", "if", "nohup", "sudo", "then", "time", "while",
    ];
    replacement.lines().any(|line| {
        line.split([';', '|', '&']).any(|segment| {
            let mut skip_next = false;
            for token in segment.split_whitespace() {
                let token = token.trim_start_matches(['#', '$', '>', '*', '!']);
                if token.is_empty() {
                    continue;
                }
                if skip_next {
                    skip_next = false;
                    continue;
                }
                if matches!(
                    token,
                    "-u" | "-g" | "-h" | "-C" | "-D" | "--user" | "--group" | "--host" | "--chdir"
                ) {
                    skip_next = true;
                    continue;
                }
                if token.contains('=') || token.starts_with('-') {
                    continue;
                }
                let command = token.rsplit('/').next().unwrap_or(token);
                if WRAPPERS.contains(&command) {
                    continue;
                }
                return commands.contains(&command)
                    || declared_tools
                        .iter()
                        .any(|tool| tool.eq_ignore_ascii_case(command));
            }
            false
        })
    })
}

fn has_operational_code_fence(replacement: &str) -> bool {
    const OPERATIONAL_FORMATS: &[&str] = &[
        "bash",
        "console",
        "compose",
        "conf",
        "dash",
        "docker",
        "dockerfile",
        "fish",
        "hcl",
        "ini",
        "json",
        "ksh",
        "nginx",
        "powershell",
        "pwsh",
        "sh",
        "shell",
        "sql",
        "systemd",
        "terraform",
        "tf",
        "toml",
        "yaml",
        "yml",
        "zsh",
    ];
    replacement.lines().any(|line| {
        let line = line.trim_start();
        let Some(marker @ ('`' | '~')) = line.chars().next() else {
            return false;
        };
        let fence_length = line
            .chars()
            .take_while(|character| *character == marker)
            .count();
        if fence_length < 3 {
            return false;
        }
        let fence = &line[marker.len_utf8() * fence_length..];
        fence
            .split_whitespace()
            .next()
            .is_some_and(|language| OPERATIONAL_FORMATS.contains(&language))
    })
}

fn lint_fixture(
    pack: &Path,
    verification: Option<&wayexpand_core::PackVerification>,
    inspection: &wayexpand_core::PackInspection,
) -> std::result::Result<String, String> {
    let verification =
        verification.ok_or_else(|| "[verification].test_cases is required".to_owned())?;
    let cases_path = safe_pack_file(pack, &verification.test_cases, "tests")
        .map_err(|error| format!("invalid test fixture: {error:#}"))?;
    let source = std::fs::read_to_string(cases_path)
        .map_err(|error| format!("could not read test fixture: {error}"))?;
    let cases: TestFile =
        toml::from_str(&source).map_err(|error| format!("invalid test fixture TOML: {error}"))?;
    if cases.format_version != 1 || cases.case.is_empty() {
        return Err("test fixture requires format_version=1 and at least one [[case]]".into());
    }
    for case in &cases.case {
        if case.trigger.trim().is_empty() || case.trigger.chars().any(char::is_control) {
            return Err("test case trigger is empty or contains control characters".into());
        }
        safe_pack_file(pack, &case.expected_output, "examples")
            .map_err(|error| format!("invalid expected output: {error:#}"))?;
        if !inspection
            .expansions
            .iter()
            .any(|expansion| expansion.trigger == case.trigger)
        {
            return Err(format!(
                "fixture trigger {:?} does not identify a pack expansion",
                case.trigger
            ));
        }
    }
    Ok(format!(
        "{} case(s) and expected-output paths are valid",
        cases.case.len()
    ))
}

fn test_pack(path: &Path, json: bool) -> Result<()> {
    let (inspection, config, _) = match import_pack(path) {
        Ok(result) => result,
        Err(error) => {
            let report = serde_json::json!({
                "schema_version": 1,
                "command": "pack test",
                "ok": false,
                "exit_code": 1,
                "cases": [],
                "error": error.to_string(),
            });
            if json {
                println!("{report}");
            }
            bail!("pack test could not load pack: {error}");
        }
    };
    let Some(verification) = inspection.manifest.verification.as_ref() else {
        return report_test_failure(
            json,
            &inspection.manifest.id,
            "pack has no [verification].test_cases declaration",
        );
    };
    let cases_path = match safe_pack_file(path, &verification.test_cases, "tests") {
        Ok(path) => path,
        Err(error) => {
            return report_test_failure(json, &inspection.manifest.id, &error.to_string())
        }
    };
    let source = match std::fs::read_to_string(&cases_path) {
        Ok(source) => source,
        Err(error) => {
            return report_test_failure(
                json,
                &inspection.manifest.id,
                &format!("could not read test cases: {error}"),
            )
        }
    };
    let test_file: TestFile = match toml::from_str::<TestFile>(&source) {
        Ok(test_file) if test_file.format_version == 1 && !test_file.case.is_empty() => test_file,
        Ok(_) => {
            return report_test_failure(
                json,
                &inspection.manifest.id,
                "test cases require format_version=1 and at least one [[case]]",
            )
        }
        Err(error) => {
            return report_test_failure(
                json,
                &inspection.manifest.id,
                &format!("invalid test fixture TOML: {error}"),
            )
        }
    };

    let mut outcomes = Vec::with_capacity(test_file.case.len());
    for case in &test_file.case {
        let expected_path = match safe_pack_file(path, &case.expected_output, "examples") {
            Ok(path) => path,
            Err(error) => {
                outcomes.push(failed_case(case, &error.to_string()));
                continue;
            }
        };
        let expected = match std::fs::read_to_string(&expected_path) {
            Ok(expected) => expected,
            Err(error) => {
                outcomes.push(failed_case(
                    case,
                    &format!("could not read expected output: {error}"),
                ));
                continue;
            }
        };
        let Some(expansion) = config
            .expansion
            .iter()
            .find(|expansion| expansion.answers_to(&case.trigger))
        else {
            outcomes.push(failed_case(
                case,
                "trigger does not identify an expansion in the pack",
            ));
            continue;
        };
        if !expansion.enabled {
            outcomes.push(failed_case(
                case,
                "fixture trigger belongs to a disabled expansion",
            ));
            continue;
        }
        let actual = match render_case(&config, expansion, case) {
            Ok(actual) => actual,
            Err(error) => {
                outcomes.push(failed_case(case, &error.to_string()));
                continue;
            }
        };
        if actual == expected {
            outcomes.push(serde_json::json!({"trigger": case.trigger, "status": "pass", "expected_output": case.expected_output}));
        } else {
            outcomes.push(serde_json::json!({"trigger": case.trigger, "status": "fail", "expected_output": case.expected_output, "expected": expected, "actual": actual, "message": "generated output does not match the fixture"}));
        }
    }
    let ok = outcomes.iter().all(|case| case["status"] == "pass");
    let report = serde_json::json!({
        "schema_version": 1,
        "command": "pack test",
        "ok": ok,
        "exit_code": if ok { 0 } else { 1 },
        "pack": {"id": inspection.manifest.id, "version": inspection.manifest.version, "fingerprint": inspection.fingerprint},
        "passed": outcomes.iter().filter(|case| case["status"] == "pass").count(),
        "failed": outcomes.iter().filter(|case| case["status"] == "fail").count(),
        "cases": outcomes,
    });
    if json {
        println!("{report}");
    } else {
        println!(
            "Pack test: {} {}",
            inspection.manifest.id, inspection.manifest.version
        );
        for case in report["cases"].as_array().into_iter().flatten() {
            println!(
                "{}  {} — {}",
                if case["status"] == "pass" {
                    "PASS"
                } else {
                    "FAIL"
                },
                case["trigger"].as_str().unwrap_or("unknown"),
                case["message"].as_str().unwrap_or("output matches fixture")
            );
        }
        println!("{} passed; {} failed", report["passed"], report["failed"]);
    }
    if !ok {
        bail!("pack tests failed; see case results above");
    }
    Ok(())
}

fn render_case(
    config: &wayexpand_core::Config,
    expansion: &wayexpand_core::ExpansionConfig,
    case: &TestCase,
) -> Result<String> {
    let fields = form_fields(&expansion.replacement)
        .map_err(|error| config_error(format!("{}: {error}", expansion.trigger)))?;
    if fields.is_empty() {
        let mut isolated = config.clone();
        isolated.expansion = vec![expansion.clone()];
        isolated.hotkey.clear();
        isolated.expansion[0].command = None;
        let mut engine = ExpansionEngine::new(isolated).map_err(|error| {
            config_error(format!(
                "test configuration invalid: {}",
                error.safe_summary()
            ))
        })?;
        if let Some(context) = &case.context {
            let mut template_context = config.template_context(None);
            template_context.session_context = Some(test_session_context(context));
            engine.set_template_context(template_context);
        }
        if let Some(app_id) = case.app_id.as_deref() {
            engine.set_current_window(Some(wayexpand_core::WindowContext {
                app_id: Some(app_id.to_owned()),
                title: None,
                instance_id: None,
            }));
        }
        let mut matches = engine.process(InputEvent::Text(case.trigger.clone()));
        matches.extend(engine.process(InputEvent::EndOfInput));
        return matches
            .last()
            .map(|result| result.insert.as_str().to_owned())
            .context("fixture trigger did not match or was blocked by snippet requirements");
    }

    let mut form_values = FormValues::default();
    for field in &fields {
        let value = case
            .values
            .get(&field.key)
            .cloned()
            .or_else(|| field_default(&field.kind));
        if let Some(value) = value {
            field.validate_value(&value).map_err(|error| {
                config_error(format!("invalid fixture value for {}: {error}", field.key))
            })?;
            form_values.insert(field.key.clone(), value);
        } else if field.required {
            bail!("fixture is missing required value for field {}", field.key);
        }
    }
    if case
        .values
        .keys()
        .any(|key| !fields.iter().any(|field| &field.key == key))
    {
        bail!("fixture includes a value for an unknown form field");
    }
    let mut context: TemplateContext = config.template_context(None);
    if let Some(session) = &case.context {
        context.session_context = Some(test_session_context(session));
    }
    context.fields = Arc::new(form_values);
    render_template_with_cursor(&expansion.replacement, &context)
        .map(|(rendered, _)| rendered)
        .map_err(|error| config_error(format!("template render failed: {error}")))
}

fn test_session_context(context: &TestContext) -> wayexpand_core::SessionContextReport {
    wayexpand_core::SessionContextReport {
        terminal_id: "tty-1-2".into(),
        session_id: "0123456789abcdef0123456789abcdef".into(),
        shell: Some(context.shell.clone()),
        remote_host: context.remote_host.clone(),
        environment: context.environment.clone(),
        project_root: context.project_root.clone(),
        kubernetes_context: context.kubernetes_context.clone(),
        container: context.container.clone(),
    }
}

fn field_default(kind: &wayexpand_core::FormFieldKind) -> Option<String> {
    match kind {
        wayexpand_core::FormFieldKind::Text { default }
        | wayexpand_core::FormFieldKind::Date { default } => {
            (!default.is_empty()).then(|| default.clone())
        }
        wayexpand_core::FormFieldKind::Choice { default, .. } => default.clone(),
    }
}

fn safe_pack_file(root: &Path, relative: &str, required_directory: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || !path.starts_with(required_directory)
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("fixture path must be a safe relative file under {required_directory}/");
    }
    let mut current = root.to_owned();
    for component in path.components() {
        let Component::Normal(name) = component else {
            bail!("unsafe fixture path")
        };
        current.push(name);
        let metadata = std::fs::symlink_metadata(&current)
            .with_context(|| format!("could not inspect fixture {}", current.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("fixture path contains a symlink: {}", current.display());
        }
        if current != root.join(path) && !metadata.is_dir() {
            bail!(
                "fixture path parent is not a directory: {}",
                current.display()
            );
        }
        if current == root.join(path) && !metadata.is_file() {
            bail!("fixture is not a regular file: {}", current.display());
        }
    }
    Ok(root.join(path))
}

fn add_check(checks: &mut Vec<serde_json::Value>, name: &str, passed: bool, message: &str) {
    checks.push(serde_json::json!({"name": name, "status": if passed {"pass"} else {"fail"}, "message": message}));
}

fn failed_case(case: &TestCase, message: &str) -> serde_json::Value {
    serde_json::json!({"trigger": case.trigger, "status": "fail", "expected_output": case.expected_output, "message": message})
}

fn report_test_failure<T>(json: bool, pack_id: &str, message: &str) -> Result<T> {
    if json {
        println!(
            "{}",
            serde_json::json!({"schema_version":1,"command":"pack test","ok":false,"exit_code":1,"pack":{"id":pack_id},"passed":0,"failed":1,"cases":[],"error":message})
        );
    }
    bail!("pack test failed: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_detection_covers_common_sysadmin_command_families() {
        for command in [
            "nft add rule inet filter input",
            "ip route get 192.0.2.1",
            "ss -lntp",
            "firewall-cmd --reload",
            "mariadb --host=db-primary",
            "certbot certificates",
            "kubectl get pods",
            "whmapi1 listaccts",
            "uapi --user=admin",
            "systemd-run --scope",
            "pg_dump production",
            "sudo apt-get install nginx",
            "dnf upgrade --refresh",
            "pacman -Syu",
            "zypper refresh",
            "systemctl restart nginx",
            "kubectl config current-context",
            "vault status",
            "gcloud compute instances list",
            "az vm list",
        ] {
            assert!(
                looks_operational_with_tools(
                    &format!("```sh\n{command} {{field:target=example|required}}\n```"),
                    &[],
                ),
                "command family was not recognized: {command}"
            );
        }
    }

    #[test]
    fn unknown_commands_and_operational_config_fences_require_field_review() {
        for (index, replacement) in [
            "```bash\ninternalctl inspect {{field:target}}\n```",
            "~~~~zsh\ninternalctl inspect {{field:target}}\n~~~~",
            "````powershell title=example\ninternalctl inspect {{field:target}}\n````",
            "```hcl\nresource \"aws_instance\" \"web\" { ami = {{field:ami}} }\n```",
            "~~~yaml\napiVersion: v1\ndata: {{field:config}}\n~~~",
        ]
        .into_iter()
        .enumerate()
        {
            let expansion = wayexpand_core::PackExpansionReview {
                file: "snippets/custom-tool.toml".into(),
                id: format!("custom_tool_{index}"),
                trigger: format!(";custom{index}"),
                description: "Run an internal operations tool".into(),
                replacement: replacement.into(),
                command: None,
            };

            let findings = operational_field_findings(&[expansion], &[]);
            assert_eq!(findings.len(), 1, "fence or format {replacement:?}");
            assert!(findings[0]["key"].as_str().is_some());
        }
    }

    #[test]
    fn operational_fields_require_validation_or_an_explicit_free_text_reason() {
        let unreviewed = wayexpand_core::PackExpansionReview {
            file: "snippets/firewall.toml".into(),
            id: "firewall".into(),
            trigger: ";firewall".into(),
            description: "Firewall rule".into(),
            replacement: "nft add rule {{field:chain=filter}}".into(),
            command: None,
        };
        let justified = wayexpand_core::PackExpansionReview {
            replacement:
                "nft add rule {{shellq:comment=maintenance window|required|free_text=true}}".into(),
            ..unreviewed.clone()
        };
        let validated = wayexpand_core::PackExpansionReview {
            replacement: "firewall-cmd --zone={{field:zone=public|required|validate=dns-label}}"
                .into(),
            ..unreviewed.clone()
        };

        let findings = operational_field_findings(&[unreviewed, justified, validated], &[]);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["key"], "field:chain");
    }

    #[test]
    fn shell_commands_require_escaped_or_shell_token_safe_form_values() {
        let path = wayexpand_core::PackExpansionReview {
            file: "snippets/inspect.toml".into(),
            id: "inspect_path".into(),
            trigger: ";inspect-path".into(),
            description: "Inspect a path".into(),
            replacement: "sudo cat {{field:path=/var/log|required|validate=absolute-path}}".into(),
            command: None,
        };
        let url = wayexpand_core::PackExpansionReview {
            replacement: "curl {{field:url=https://example.net|required|validate=url}}".into(),
            ..path.clone()
        };
        let free_text = wayexpand_core::PackExpansionReview {
            replacement: "echo {{field:value=message|required|free_text=true}}".into(),
            ..path.clone()
        };
        let quoted_path = wayexpand_core::PackExpansionReview {
            replacement: "sudo cat {{shellq:path=/var/log|required|validate=absolute-path}}".into(),
            ..path.clone()
        };
        let safe_unit = wayexpand_core::PackExpansionReview {
            replacement:
                "systemctl status {{field:unit=nginx.service|required|validate=systemd-unit}}"
                    .into(),
            ..path.clone()
        };

        let findings =
            operational_field_findings(&[path, url, free_text, quoted_path, safe_unit], &[]);
        assert_eq!(findings.len(), 3);
        assert!(findings
            .iter()
            .all(|finding| finding["reason"].as_str().unwrap().contains("shell")));
        assert!(findings
            .iter()
            .any(|finding| finding["key"] == "field:path"));
        assert!(findings.iter().any(|finding| finding["key"] == "field:url"));
        assert!(findings
            .iter()
            .any(|finding| finding["key"] == "field:value"));
    }

    #[test]
    fn shell_commands_require_quoted_environment_context_and_clipboard_values() {
        let base = wayexpand_core::PackExpansionReview {
            file: "snippets/diagnostics.toml".into(),
            id: "diagnostic".into(),
            trigger: ";diagnostic".into(),
            description: "Diagnostic command".into(),
            replacement: String::new(),
            command: None,
        };
        let examples = [
            "ssh {{env:TARGET}}",
            "kubectl get pods -n {{context:environment}}",
            "echo {{clipboard}}",
        ];
        let expansions: Vec<_> = examples
            .iter()
            .enumerate()
            .map(|(index, replacement)| wayexpand_core::PackExpansionReview {
                replacement: (*replacement).into(),
                ..wayexpand_core::PackExpansionReview {
                    id: format!("diagnostic_{index}"),
                    trigger: format!(";diagnostic-{index}"),
                    ..base.clone()
                }
            })
            .collect();
        let findings = operational_field_findings(&expansions, &[]);
        assert_eq!(findings.len(), 3);

        let safe = wayexpand_core::PackExpansionReview {
            replacement: "ssh {{shellq:env:TARGET}} {{shellq:context:remote_host}}".into(),
            ..base
        };
        let safe_findings = operational_field_findings(&[safe], &[]);
        assert!(safe_findings.is_empty(), "{safe_findings:?}");
    }

    #[test]
    fn narrative_service_fields_are_not_mistaken_for_commands() {
        let support_notice = wayexpand_core::PackExpansionReview {
            file: "snippets/replies.toml".into(),
            id: "maintenance_notice".into(),
            trigger: ";maintenance-notice".into(),
            description: "Announce a maintenance window".into(),
            replacement: "Scheduled maintenance: {{field:service=service name}}".into(),
            command: None,
        };
        assert!(operational_field_findings(&[support_notice], &[]).is_empty());

        let command = wayexpand_core::PackExpansionReview {
            file: "snippets/service.toml".into(),
            id: "service_restart".into(),
            trigger: ";restart-service".into(),
            description: "Restart a service".into(),
            replacement: "sudo -u root service {{field:unit}} restart".into(),
            command: None,
        };
        assert_eq!(operational_field_findings(&[command], &[]).len(), 1);
    }

    #[test]
    fn ip_and_socket_diagnostic_fields_are_not_skipped_by_lint() {
        let unvalidated_ip = wayexpand_core::PackExpansionReview {
            file: "snippets/network.toml".into(),
            id: "network".into(),
            trigger: ";route".into(),
            description: "Inspect a route".into(),
            replacement: "ip route get {{field:destination}}".into(),
            command: None,
        };
        let unvalidated_socket = wayexpand_core::PackExpansionReview {
            replacement: "ss -lntp {{field:port}}".into(),
            ..unvalidated_ip.clone()
        };

        let findings = operational_field_findings(&[unvalidated_ip, unvalidated_socket], &[]);
        assert_eq!(findings.len(), 2);
        assert!(findings
            .iter()
            .any(|finding| finding["key"] == "field:destination"));
        assert!(findings
            .iter()
            .any(|finding| finding["key"] == "field:port"));
    }

    #[test]
    fn declared_pack_tools_extend_operational_field_detection() {
        let expansion = wayexpand_core::PackExpansionReview {
            file: "snippets/storage.toml".into(),
            id: "storage".into(),
            trigger: ";disk".into(),
            description: "Inspect a filesystem".into(),
            replacement: "df -hT -- {{field:path=/var|required}}".into(),
            command: None,
        };
        let tools = vec!["df".to_owned()];

        let findings = operational_field_findings(&[expansion], &tools);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["key"], "field:path");
    }

    #[test]
    fn reviewed_catalog_packs_pass_lint_and_output_tests() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs");
        for entry in std::fs::read_dir(root).expect("curated packs exist") {
            let path = entry.expect("pack entry").path();
            if path.join("wayexpand-pack.toml").is_file() {
                let inspection = inspect_pack(&path).expect("pack is structurally valid");
                assert!(
                    inspection.manifest.maintainer.is_some(),
                    "{}",
                    path.display()
                );
                test_pack_fixture_direct(&path)
                    .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
            }
        }
    }

    fn test_pack_fixture_direct(path: &Path) -> Result<()> {
        let (inspection, config, _) = import_pack(path)?;
        let verification = inspection
            .manifest
            .verification
            .context("missing verification")?;
        let fixture = safe_pack_file(path, &verification.test_cases, "tests")?;
        let file: TestFile = toml::from_str(&std::fs::read_to_string(fixture)?)?;
        if file.format_version != 1 || file.case.is_empty() {
            bail!("invalid cases")
        }
        for case in &file.case {
            let expected = safe_pack_file(path, &case.expected_output, "examples")?;
            let expected = std::fs::read_to_string(expected)?;
            let expansion = config
                .expansion
                .iter()
                .find(|expansion| expansion.answers_to(&case.trigger))
                .context("trigger missing")?;
            let actual = render_case(&config, expansion, case)?;
            if expected != actual {
                bail!("{} output differs from fixture", case.trigger)
            }
        }
        Ok(())
    }

    #[test]
    fn fixture_paths_reject_parent_traversal_and_symlinks() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-pack-quality-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("tests/cases.toml"), "fixture").unwrap();
        assert!(safe_pack_file(&root, "tests/../outside.toml", "tests").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("cases.toml", root.join("tests/link.toml")).unwrap();
            assert!(safe_pack_file(&root, "tests/link.toml", "tests").is_err());
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
