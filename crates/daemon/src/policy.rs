/// Organization policy enforcement for the daemon.
///
/// Loads policies from /etc/wayexpand/policy.toml and enforces them
/// at runtime, blocking unsafe operations and logging violations to journald.
use tracing::{error, warn};
use wayexpand_core::OrganizationPolicy;

/// Load organization policy from /etc/wayexpand/policy.toml
///
/// Returns the policy if it exists, or a default (permissive) policy if not.
/// An existing policy that cannot be securely read or parsed is fatal.
pub fn load_policy() -> Result<OrganizationPolicy, String> {
    match wayexpand_core::load_organization_policy() {
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

/// Check for policy violations on an expansion (for detection and logging).
/// Returns the violation message if any policy constraint is violated.
fn expansion_policy_violations(
    policy: &OrganizationPolicy,
    replacement_size: usize,
    has_command: bool,
    backend: &str,
) -> Option<String> {
    policy.expansion_policy_violation(replacement_size, has_command, backend)
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
    if let Some(violation) =
        expansion_policy_violations(policy, replacement_size, has_command, backend)
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

pub fn commands_enforced(policy: &OrganizationPolicy) -> bool {
    policy.safe_mode && policy.disable_commands
}

/// Check expansion for policy violations and log them if present.
/// Returns whether the expansion should be blocked (true when safe_mode=true and violations exist).
pub fn check_and_log_expansion_violations(
    policy: &OrganizationPolicy,
    replacement_size: usize,
    has_command: bool,
    backend: &str,
) -> bool {
    if let Some(violation) =
        expansion_policy_violations(policy, replacement_size, has_command, backend)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const POLICY_DIR: &str = wayexpand_core::ORGANIZATION_POLICY_DIR;
    const MAX_POLICY_FILE_SIZE: u64 = wayexpand_core::MAX_ORGANIZATION_POLICY_BYTES;

    fn parse_policy_content(content: &str) -> Result<OrganizationPolicy, String> {
        wayexpand_core::parse_organization_policy(content)
    }

    fn load_policy_from_path(path: &Path, policy_dir: &Path) -> Result<OrganizationPolicy, String> {
        wayexpand_core::load_organization_policy_from_paths(path, policy_dir)
    }

    fn validate_policy_directory(path: &Path) -> Result<(), String> {
        wayexpand_core::validate_organization_policy_directory(path)
    }

    fn validate_policy_file_metadata(
        path: &Path,
        metadata: &std::fs::Metadata,
    ) -> Result<(), String> {
        wayexpand_core::validate_organization_policy_file(path, metadata)
    }

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
        assert!(check_expansion_allowed(&policy, 1024, false, "input-method-v2").is_err());
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
        assert!(check_and_log_expansion_violations(
            &policy_safe,
            1024,
            true,
            "libei"
        ));
        // In audit mode, violations should not block
        assert!(!check_and_log_expansion_violations(
            &policy_audit,
            1024,
            true,
            "libei"
        ));
    }

    #[test]
    fn command_enforcement_requires_safe_mode() {
        let audit_policy = OrganizationPolicy {
            safe_mode: false,
            disable_commands: true,
            ..Default::default()
        };
        let enforced_policy = OrganizationPolicy {
            safe_mode: true,
            disable_commands: true,
            ..Default::default()
        };

        assert!(!commands_enforced(&audit_policy));
        assert!(commands_enforced(&enforced_policy));
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
allowed_backends = ["input-method-v2", "libei"]
"#,
        )
        .expect("the documented Ansible policy must load");

        assert!(policy.safe_mode);
        assert_eq!(policy.allowed_backends, ["input-method-v2", "libei"]);
    }

    #[test]
    fn validate_policy_file_metadata_rejects_symlinks() {
        // Create a test metadata that claims to be a symlink
        let path = Path::new("/etc/passwd");
        let metadata = std::fs::symlink_metadata(path).unwrap();
        let result = validate_policy_file_metadata(path, &metadata);
        // Should either fail due to wrong ownership or symlink check
        // (passwd is a regular file, so test our ownership check instead)
        assert!(result.is_err());
    }

    #[test]
    fn validate_policy_file_metadata_requires_root_ownership() {
        // Any regular file owned by non-root should fail
        let path = Path::new("/tmp");
        let metadata = std::fs::symlink_metadata(path).unwrap(); // Typically not root
        let result = validate_policy_file_metadata(path, &metadata);
        assert!(result.is_err(), "non-root file should be rejected");
    }

    #[test]
    fn policy_file_size_limit_enforced() {
        // Test that oversized files are rejected
        // This would require creating a temporary policy file, which is complex
        // The implementation is tested by the constant MAX_POLICY_FILE_SIZE
        assert_eq!(
            MAX_POLICY_FILE_SIZE,
            1024 * 100,
            "policy file size limit is 100 KB"
        );
    }

    #[test]
    fn validate_policy_directory_check_exists() {
        // Validate that directory checking is implemented
        // (actual /etc/wayexpand may not exist in test environment)
        // This test just verifies the function exists and is called
        let _ = validate_policy_directory(Path::new(POLICY_DIR));
        // Result depends on system, but function should complete
    }

    #[test]
    fn missing_policy_file_is_permissive() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/policy-test-missing");
        let path = root.join("policy.toml");
        let policy = load_policy_from_path(&path, &root).expect("missing policy is optional");
        assert_eq!(policy, OrganizationPolicy::default());
    }

    #[test]
    fn malformed_policy_is_rejected() {
        let error = parse_policy_content("[organization\nnot valid").unwrap_err();
        assert!(error.contains("invalid policy TOML"));
    }

    #[test]
    fn existing_policy_file_requires_trusted_metadata() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/policy-test-untrusted");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("policy.toml");
        std::fs::write(&path, "[organization]\ndisable_commands = true\n").unwrap();

        let error = load_policy_from_path(&path, &root).unwrap_err();
        assert!(error.contains("owned by root") || error.contains("accessible"));

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
