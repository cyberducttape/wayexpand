//! CI-oriented, non-executing configuration self-test.

use super::Args;
use crate::*;

const SCHEMA_VERSION: u32 = 1;

fn finding(
    id: &str,
    component: &str,
    message: impl Into<String>,
    repair_available: bool,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "component": component,
        "message": message.into(),
        "repair_available": repair_available,
    })
}

fn config_finding_id(error: &wayexpand_core::ConfigError) -> (&'static str, &'static str) {
    use wayexpand_core::ConfigError as E;
    match error {
        E::Parse(_) | E::InvalidUtf8 { .. } => ("WX-CONFIG-001", "toml"),
        E::DuplicateTrigger { .. } | E::DuplicateExpansionId { .. } => {
            ("WX-CONFIG-002", "triggers")
        }
        E::InvalidTemplate { .. } => ("WX-CONFIG-003", "templates"),
        E::InvalidCommand { .. } | E::InvalidHotkey { .. } => ("WX-CONFIG-004", "declarations"),
        E::InvalidPolicyConfig(_) => ("WX-POLICY-001", "policy"),
        _ => ("WX-CONFIG-001", "configuration"),
    }
}

pub(crate) fn check_command(args: Args) -> Result<()> {
    let mut all = false;
    let mut json = false;
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "--all" => all = true,
            "--json" => json = true,
            value if value.starts_with('-') => {
                usage_bail!("usage: wayexpand check --all [--json] [config]")
            }
            value if path.is_none() => path = Some(PathBuf::from(value)),
            _ => usage_bail!("usage: wayexpand check --all [--json] [config]"),
        }
    }
    if !all {
        usage_bail!("usage: wayexpand check --all [--json] [config]");
    }

    let config_path = path.unwrap_or_else(default_config_path);
    let mut errors = Vec::<serde_json::Value>::new();
    let mut warnings = Vec::<serde_json::Value>::new();
    let mut checks = Vec::<serde_json::Value>::new();
    let loaded = Config::load_versioned(&config_path);
    let config = match loaded {
        Ok(loaded) => {
            checks.push(serde_json::json!({
                "id":"WX-CHECK-TOML", "component":"configuration", "status":"pass",
                "message":"TOML syntax, schema, expansion identifiers, duplicate triggers, aliases, and static templates are valid"
            }));
            Some(loaded.config)
        }
        Err(error) => {
            let (id, component) = config_finding_id(&error);
            errors.push(finding(id, component, error.safe_summary(), false));
            checks.push(serde_json::json!({
                "id":"WX-CHECK-TOML", "component":"configuration", "status":"fail",
                "message":"configuration could not pass the secure parser and static validator"
            }));
            None
        }
    };

    if let Some(current) = config.as_ref() {
        match Config::validate_library_files(&config_path) {
            Ok(()) => checks.push(serde_json::json!({
                "id":"WX-CHECK-FILES", "component":"referenced_files", "status":"pass",
                "message":"referenced snippet library files are present and valid"
            })),
            Err(error) => {
                errors.push(finding(
                    "WX-CONFIG-005",
                    "referenced_files",
                    error.safe_summary(),
                    false,
                ));
                checks.push(serde_json::json!({
                    "id":"WX-CHECK-FILES", "component":"referenced_files", "status":"fail",
                    "message":"one or more referenced files could not be validated"
                }));
            }
        }
        let enabled = current
            .expansion
            .iter()
            .filter(|entry| entry.is_keyboard_matchable())
            .count();
        let disabled = current
            .expansion
            .iter()
            .filter(|entry| !entry.enabled)
            .count();
        let generate_only = current
            .expansion
            .iter()
            .filter(|entry| {
                entry.enabled && entry.delivery == wayexpand_core::ExpansionDelivery::GenerateOnly
            })
            .count();
        let app_restricted = current
            .expansion
            .iter()
            .filter(|entry| entry.is_keyboard_matchable() && !entry.app_filter.is_empty())
            .count();
        if enabled == 0 {
            warnings.push(finding(
                "WX-CONFIG-006",
                "expansions",
                "no enabled expansions are available to match",
                false,
            ));
        }
        if disabled > 0 {
            warnings.push(finding(
                "WX-CONFIG-008",
                "expansions",
                format!("{disabled} expansion(s) are disabled and cannot match until enabled"),
                false,
            ));
        }
        checks.push(serde_json::json!({
            "id":"WX-CHECK-REACHABILITY", "component":"expansions", "status":if enabled == 0 {"warning"} else {"pass"},
            "enabled":enabled, "disabled":disabled, "generate_only":generate_only,
            "application_restricted":app_restricted,
            "message":if enabled == 0 && generate_only > 0 {"no keyboard triggers; generate-only templates are available through `wayexpand new`"} else if enabled == 0 {"no enabled keyboard triggers"} else {"enabled triggers are unique and reachable by the matcher; application-filtered entries require a matching focused application"}
        }));
        let declared_commands = current
            .expansion
            .iter()
            .filter(|entry| entry.command.is_some())
            .count()
            + current.hotkey.len();
        checks.push(serde_json::json!({
            "id":"WX-CHECK-DYNAMIC", "component":"dynamic_commands", "status":"pass",
            "declarations_validated":declared_commands,
            "executed":false,
            "message":"dynamic command declarations were statically validated; no command was executed"
        }));
        checks.push(serde_json::json!({
            "id":"WX-CHECK-VERSION", "component":"version_compatibility", "status":"pass",
            "message":"configuration was accepted by the current WayExpand schema"
        }));

        match crate::doctor::policy::load_policy() {
            Ok(policy) => {
                let mut candidate = current.clone();
                match candidate.apply_administrator_policy(&policy) {
                    Ok(()) => {
                        checks.push(serde_json::json!({
                            "id":"WX-CHECK-POLICY", "component":"policy", "status":"pass",
                            "message":"configuration complies with the effective administrator policy"
                        }));
                        match ExpansionEngine::new(candidate) {
                            Ok(_) => checks.push(serde_json::json!({
                                "id":"WX-CHECK-MATCHER", "component":"matcher", "status":"pass",
                                "message":"the validated effective configuration builds a matcher"
                            })),
                            Err(error) => errors.push(finding(
                                "WX-CONFIG-007",
                                "matcher",
                                error.safe_summary(),
                                false,
                            )),
                        }
                    }
                    Err(error) => errors.push(finding(
                        "WX-POLICY-002",
                        "policy",
                        error.safe_summary(),
                        false,
                    )),
                }
            }
            Err(error) => errors.push(finding(
                "WX-POLICY-001",
                "policy",
                format!("could not load effective policy: {error}"),
                false,
            )),
        }
    }

    let healthy = errors.is_empty();
    let report = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "healthy": healthy,
        "config": config_path,
        "checks": checks,
        "errors": errors,
        "warnings": warnings,
        "dynamic_commands_executed": false,
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "WayExpand configuration self-test: {}",
            if healthy { "PASS" } else { "FAIL" }
        );
        for item in report["checks"].as_array().into_iter().flatten() {
            println!(
                "{} {}: {}",
                item["status"].as_str().unwrap_or("unknown").to_uppercase(),
                item["component"].as_str().unwrap_or("check"),
                item["message"].as_str().unwrap_or("")
            );
        }
        for item in report["errors"].as_array().into_iter().flatten() {
            println!(
                "ERROR {} [{}]: {}",
                item["id"].as_str().unwrap_or("WX-UNKNOWN"),
                item["component"].as_str().unwrap_or("unknown"),
                item["message"].as_str().unwrap_or("")
            );
        }
        for item in report["warnings"].as_array().into_iter().flatten() {
            println!(
                "WARNING {} [{}]: {}",
                item["id"].as_str().unwrap_or("WX-UNKNOWN"),
                item["component"].as_str().unwrap_or("unknown"),
                item["message"].as_str().unwrap_or("")
            );
        }
    }
    if healthy {
        Ok(())
    } else {
        Err(config_error("configuration self-test found errors"))
    }
}
