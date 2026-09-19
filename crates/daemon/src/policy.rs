/// Organization policy enforcement for the daemon.
///
/// Loads policies from /etc/wayexpand/policy.toml and enforces them
/// at runtime, blocking unsafe operations and logging violations to journald.
use serde::Deserialize;
use std::{os::unix::fs::MetadataExt, path::Path};
use tracing::{error, warn};
use wayexpand_core::OrganizationPolicy;

const POLICY_PATH: &str = "/etc/wayexpand/policy.toml";

/// Load organization policy from /etc/wayexpand/policy.toml
///
/// Returns the policy if it exists, or a default (permissive) policy if not.
/// An existing policy that cannot be securely read or parsed is fatal.
pub fn load_policy() -> Result<OrganizationPolicy, String> {
    match load_policy_internal() {
        Ok(policy) => {
            if policy.is_active() {
                tracing::info!(
                    "organization policy loaded: safe_mode={}, backends={:?}",
                    policy.safe_mode,
                    if policy.allowed_backends.is_empty() {
                        "(all)".to_string()
                    } else {
                        format!("{:?}", policy.allowed_backends)
                    }
                );
            }
            Ok(policy)
        }
        Err(e) => Err(e),
    }
}

fn load_policy_internal() -> Result<OrganizationPolicy, String> {
    let path = Path::new(POLICY_PATH);

    // Policy file is optional; no file = default policy
    if !path.exists() {
        return Ok(OrganizationPolicy::default());
    }

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("could not inspect {}: {}", POLICY_PATH, e))?;
    if !metadata.file_type().is_file() {
        return Err(format!("{} is not a regular file", POLICY_PATH));
    }
    if metadata.uid() != 0 {
        return Err(format!("{} must be owned by root", POLICY_PATH));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "{} must not be group- or world-writable",
            POLICY_PATH
        ));
    }

    // Read and parse policy file
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {}", POLICY_PATH, e))?;

    parse_policy_content(&content)
}

fn parse_policy_content(content: &str) -> Result<OrganizationPolicy, String> {
    let file: PolicyFile =
        toml::from_str(content).map_err(|e| format!("invalid policy TOML: {}", e))?;

    // Check for [organization] table first (documented enterprise format)
    if let Some(policy) = file.organization {
        return Ok(policy);
    }

    // Fall back to flat format for backward compatibility
    // Try to deserialize entire file as OrganizationPolicy
    match toml::from_str::<OrganizationPolicy>(content) {
        Ok(policy) => Ok(policy),
        Err(e) => Err(format!(
            "policy file must contain either [organization] table or flat policy fields: {}",
            e
        )),
    }
}

/// Check for policy violations on an expansion (for detection and logging).
/// Returns the violation message if any policy constraint is violated.
fn expansion_policy_violations(
    policy: &OrganizationPolicy,
    replacement_size: usize,
    has_command: bool,
    backend: &str,
) -> Option<String> {
    let mut violations = Vec::new();

    if has_command && policy.disable_commands {
        violations.push("command execution is disabled by organization policy".to_string());
    }

    if !policy.replacement_size_allowed(replacement_size) {
        violations.push(format!(
            "replacement size {} bytes exceeds policy limit of {} bytes",
            replacement_size, policy.max_replacement_size
        ));
    }

    if !policy.backend_allowed(backend) {
        violations.push(format!(
            "backend '{}' is not in allowed list: {:?}",
            backend, policy.allowed_backends
        ));
    }

    if violations.is_empty() {
        None
    } else {
        Some(violations.join("; "))
    }
}

/// Check if an expansion should be allowed under the current policy.
///
/// When safe_mode is true, any policy violation prevents execution (error is returned).
/// When safe_mode is false, violations are logged as warnings but execution proceeds (Ok is returned).
#[allow(dead_code)] // Used in unit tests
pub fn check_expansion_allowed(
    policy: &OrganizationPolicy,
    replacement_size: usize,
    has_command: bool,
    backend: &str,
) -> Result<(), String> {
    if let Some(violation) = expansion_policy_violations(policy, replacement_size, has_command, backend)
    {
        if policy.safe_mode {
            // In safe_mode, violations are enforced (prevent expansion)
            return Err(violation);
        }
        // In audit mode (safe_mode=false), violations are warnings (expansion proceeds)
        // Caller should log via log_violation()
    }
    Ok(())
}

/// Check if a hotkey should be allowed under the current policy.
/// When safe_mode is true, disabled hotkeys prevent execution.
/// When safe_mode is false, disabled hotkeys are logged as warnings but execution proceeds.
pub fn check_hotkey_allowed(policy: &OrganizationPolicy) -> Result<(), String> {
    if policy.disable_hotkeys {
        let msg = "hotkeys are disabled by organization policy".to_string();
        if policy.safe_mode {
            return Err(msg);
        }
        // In audit mode, violation is logged but hotkey proceeds
    }
    Ok(())
}

/// Check expansion for policy violations and log them if present.
/// Returns whether the expansion should be blocked (true when safe_mode=true and violations exist).
pub fn check_and_log_expansion_violations(
    policy: &OrganizationPolicy,
    replacement_size: usize,
    has_command: bool,
    backend: &str,
) -> bool {
    if let Some(violation) = expansion_policy_violations(policy, replacement_size, has_command, backend)
    {
        log_violation(policy, &violation);
        // Return true (block) only in safe_mode
        policy.safe_mode
    } else {
        false // No violations, don't block
    }
}

/// Log a policy violation to journald with the configured prefix
pub fn log_violation(policy: &OrganizationPolicy, violation: &str) {
    let message = format!("[{}] {}", policy.audit_prefix, violation);

    if policy.safe_mode {
        // In safe_mode, violations are errors
        error!("{}", message);
    } else {
        // Otherwise just warnings
        warn!("{}", message);
    }
}

// Internal wrapper for parsing policy files with [organization] table
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct PolicyFile {
    #[serde(default)]
    organization: Option<OrganizationPolicy>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_expansion_allows_normal_case() {
        let policy = OrganizationPolicy::default();
        assert!(check_expansion_allowed(&policy, 1024, false, "libei").is_ok());
    }

    #[test]
    fn check_expansion_blocks_commands_in_safe_mode() {
        let policy = OrganizationPolicy {
            safe_mode: true,
            disable_commands: true,
            ..Default::default()
        };
        assert!(check_expansion_allowed(&policy, 1024, true, "libei").is_err());
    }

    #[test]
    fn check_expansion_allows_commands_in_audit_mode() {
        let policy = OrganizationPolicy {
            safe_mode: false,
            disable_commands: true,
            ..Default::default()
        };
        // In audit mode, violation is detected but expansion proceeds
        assert!(check_expansion_allowed(&policy, 1024, true, "libei").is_ok());
    }

    #[test]
    fn check_expansion_blocks_size_when_exceeded_in_safe_mode() {
        let policy = OrganizationPolicy {
            safe_mode: true,
            max_replacement_size: 1000,
            ..Default::default()
        };
        assert!(check_expansion_allowed(&policy, 2000, false, "libei").is_err());
    }

    #[test]
    fn check_expansion_blocks_disallowed_backend_in_safe_mode() {
        let policy = OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["libei".to_string()],
            ..Default::default()
        };
        assert!(check_expansion_allowed(&policy, 1024, false, "input-method").is_err());
    }

    #[test]
    fn check_hotkey_blocks_when_disabled_in_safe_mode() {
        let policy = OrganizationPolicy {
            safe_mode: true,
            disable_hotkeys: true,
            ..Default::default()
        };
        assert!(check_hotkey_allowed(&policy).is_err());
    }

    #[test]
    fn violations_blocked_in_safe_mode_only() {
        let policy_safe = OrganizationPolicy {
            safe_mode: true,
            disable_commands: true,
            ..Default::default()
        };
        let policy_audit = OrganizationPolicy {
            safe_mode: false,
            disable_commands: true,
            ..Default::default()
        };

        // In safe_mode, violations should block
        assert!(check_and_log_expansion_violations(&policy_safe, 1024, true, "libei"));
        // In audit mode, violations should not block
        assert!(!check_and_log_expansion_violations(&policy_audit, 1024, true, "libei"));
    }

    #[test]
    fn load_policy_with_organization_table() {
        // Test documented enterprise format: [organization] table
        let toml_content = r#"
[organization]
safe_mode = true
disable_commands = false
disable_hotkeys = false
disable_title_matching = false
max_replacement_size = 65536
allowed_backends = ["libei"]
allowed_packs = []
audit_prefix = "wayexpand"
"#;
        let policy = parse_policy_content(toml_content);
        assert!(policy.is_ok());
        let policy = policy.unwrap();
        assert!(policy.safe_mode);
        assert_eq!(policy.max_replacement_size, 65536);
    }

    #[test]
    fn load_policy_accepts_published_ansible_shape() {
        let policy = parse_policy_content(
            r#"
[organization]
safe_mode = true
disable_hotkeys = false
disable_title_matching = false
max_replacement_size = 65536
allowed_backends = ["input-method", "libei"]
"#,
        )
        .expect("the documented Ansible policy must load");

        assert!(policy.safe_mode);
        assert_eq!(policy.allowed_backends, ["input-method", "libei"]);
    }
}
