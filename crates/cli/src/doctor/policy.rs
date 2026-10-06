//! Organization policy and runtime capability reporting.

use crate::*;

pub(crate) fn load_policy() -> Result<OrganizationPolicy> {
    wayexpand_core::load_organization_policy().map_err(|error| anyhow::anyhow!(error))
}

pub(crate) fn absolute_command_policy_diagnostic(
    policy: &OrganizationPolicy,
) -> Option<&'static str> {
    policy
        .require_absolute_commands
        .then_some(if policy.safe_mode {
            "Require absolute command paths: enforced"
        } else {
            "Require absolute command paths: audit only"
        })
}

pub(crate) fn print_policy_diagnostics_json(
    policy_result: &Result<OrganizationPolicy>,
) -> serde_json::Value {
    let policy_json = match policy_result {
        Ok(policy) => {
            serde_json::json!({
                "valid": true,
                "safe_mode": policy.safe_mode,
                "disable_commands": policy.disable_commands,
                "require_absolute_commands": policy.require_absolute_commands,
                "require_atomic_replace": policy.require_atomic_replace,
                "minimum_replacement_guarantee": policy
                    .minimum_replacement_guarantee
                    .map(wayexpand_core::ReplacementGuarantee::as_str),
                "require_sensitive_focus": policy.require_sensitive_focus,
                "disable_hotkeys": policy.disable_hotkeys,
                "disable_title_matching": policy.disable_title_matching,
                "max_replacement_size": policy.max_replacement_size,
                "allowed_backends": policy.allowed_backends,
                "allowed_packs": policy.allowed_packs,
                "audit_prefix": policy.audit_prefix,
                "is_active": policy.is_active(),
            })
        }
        Err(error) => {
            serde_json::json!({
                "valid": false,
                "error": error.to_string(),
            })
        }
    };

    serde_json::json!({
        "path": wayexpand_core::ORGANIZATION_POLICY_PATH,
        "exists": Path::new(wayexpand_core::ORGANIZATION_POLICY_PATH).exists(),
        "policy": policy_json,
    })
}

pub(crate) fn print_capabilities_diagnostics() {
    println!("\nBackend capabilities:");
    let all_caps = all_capabilities();
    for caps in all_caps {
        let env_support = if caps.environment_compatible() {
            "yes"
        } else {
            "no"
        };
        println!(
            "  {}: environment compatible: {}",
            caps.backend_name, env_support
        );
        println!("      live protocol probe: reported separately above (not inferred here)");
        println!("    Features: {}", caps.feature_summary);
        for limitation in caps.limitations {
            println!("    Limitation: {limitation}");
        }
        println!(
            "    Max replacement: {}",
            if caps.max_replacement_size == 0 {
                "unlimited".to_string()
            } else {
                format!("{} bytes", caps.max_replacement_size)
            }
        );
    }
}

pub(crate) fn print_capabilities_diagnostics_json() -> serde_json::Value {
    let all_caps = all_capabilities();
    let caps_json: Vec<_> = all_caps
        .iter()
        .map(|caps| {
            serde_json::json!({
                "backend": caps.backend_name,
                "environment_compatible": caps.environment_compatible(),
                "multiline": caps.multiline,
                "exclusive_capture": caps.exclusive_capture,
                "text_method": caps.text_method.to_string(),
                "max_replacement_size": caps.max_replacement_size,
                "feature_summary": caps.feature_summary,
                "limitations": caps.limitations,
            })
        })
        .collect();
    serde_json::Value::Array(caps_json)
}

pub(crate) fn print_policy_diagnostics() -> bool {
    if !Path::new(wayexpand_core::ORGANIZATION_POLICY_PATH).exists() {
        println!(
            "Organization policy: {} (not found, using default permissive policy)",
            wayexpand_core::ORGANIZATION_POLICY_PATH
        );
        return true;
    }

    match load_policy() {
        Ok(policy) => {
            println!(
                "Organization policy: {} (valid)",
                wayexpand_core::ORGANIZATION_POLICY_PATH
            );
            if policy.is_active() {
                println!(
                    "  Safe mode: {}",
                    if policy.safe_mode {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                if policy.disable_commands {
                    println!("  Disable commands: enabled");
                }
                if let Some(diagnostic) = absolute_command_policy_diagnostic(&policy) {
                    println!("  {diagnostic}");
                }
                if policy.disable_hotkeys {
                    println!("  Disable hotkeys: enabled");
                }
                if policy.disable_title_matching {
                    println!("  Disable title matching: enabled");
                }
                if policy.max_replacement_size > 0 {
                    println!(
                        "  Max replacement size: {} bytes",
                        policy.max_replacement_size
                    );
                }
                if !policy.allowed_backends.is_empty() {
                    println!("  Allowed backends: {:?}", policy.allowed_backends);
                }
                if !policy.allowed_packs.is_empty() {
                    println!("  Allowed packs: {:?}", policy.allowed_packs);
                }
                if !policy.audit_prefix.is_empty() {
                    println!("  Audit prefix: {}", policy.audit_prefix);
                }
            } else {
                println!("  (all constraints disabled, using defaults)");
            }
            true
        }
        Err(error) => {
            println!(
                "Organization policy: {} (invalid: {})",
                wayexpand_core::ORGANIZATION_POLICY_PATH,
                error
            );
            false
        }
    }
}
