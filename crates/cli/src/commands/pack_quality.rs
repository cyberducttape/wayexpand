//! CI-oriented pack linting and fixture tests. Pack imports strip executable
//! entries before a fixture is rendered, so these checks never run actions.

use super::config::resolve_pack_path;
use crate::args::take_flag;
use crate::*;
use serde::Deserialize;
use std::{collections::BTreeMap, path::Component, sync::Arc};
use wayexpand_core::{
    form_fields, import_pack, inspect_pack, render_template_with_cursor, ExpansionEngine,
    FormValues, InputEvent, TemplateContext,
};

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
    let operational_fields = operational_field_findings(&inspection.expansions);
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
) -> Vec<serde_json::Value> {
    expansions
        .iter()
        .filter(|expansion| looks_operational(&expansion.replacement))
        .flat_map(|expansion| {
            form_fields(&expansion.replacement)
                .unwrap_or_default()
                .into_iter()
                .filter(|field| {
                    field.validation.is_none()
                        && !field.free_text
                        && !matches!(field.kind, wayexpand_core::FormFieldKind::Choice { .. })
                })
                .map(|field| {
                    serde_json::json!({
                        "file": expansion.file,
                        "trigger": expansion.trigger,
                        "field": field.label,
                        "key": field.key,
                        "reason": "operational interpolation needs a domain validator or explicit free_text=true justification",
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn looks_operational(replacement: &str) -> bool {
    const COMMANDS: &[&str] = &[
        "ansible",
        "ansible-playbook",
        "ansible-inventory",
        "apt",
        "awk",
        "aws",
        "bash",
        "curl",
        "docker",
        "find",
        "git",
        "gh",
        "grep",
        "helm",
        "iptables",
        "journalctl",
        "kubectl",
        "mysql",
        "nc",
        "nginx",
        "openssl",
        "podman",
        "psql",
        "resolvectl",
        "rpm",
        "rsync",
        "sed",
        "sh",
        "ssh",
        "systemctl",
        "tee",
        "terraform",
        "ufw",
    ];
    let lower = replacement.to_ascii_lowercase();
    lower.contains("{{shellq:")
        || lower
            .split(|character: char| {
                !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-')
            })
            .any(|token| COMMANDS.contains(&token))
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
