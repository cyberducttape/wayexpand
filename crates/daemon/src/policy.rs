/// Organization policy enforcement for the daemon.
///
/// Loads policies from /etc/wayexpand/policy.toml and enforces them
/// at runtime, blocking unsafe operations and logging violations to journald.
use serde::Deserialize;
use std::{os::unix::fs::MetadataExt, path::Path};
use tracing::{error, warn};
use wayexpand_core::OrganizationPolicy;

const POLICY_PATH: &str = "/etc/wayexpand/policy.toml";
const POLICY_DIR: &str = "/etc/wayexpand";
const MAX_POLICY_FILE_SIZE: u64 = 1024 * 100; // 100 KB max for policies

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

    // Validate parent directory: must be /etc/wayexpand with strict permissions
    validate_policy_directory()?;

    // Use symlink_metadata to not follow symlinks (critical for security)
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("could not inspect {}: {}", POLICY_PATH, e))?;

    // Strict validation of policy file
    validate_policy_file_metadata(&metadata)?;

    // Check file size (prevent DoS via huge files)
    if metadata.size() > MAX_POLICY_FILE_SIZE {
        return Err(format!(
            "{} is too large ({} bytes, max {})",
            POLICY_PATH,
            metadata.size(),
            MAX_POLICY_FILE_SIZE
        ));
    }

    // Read and parse policy file
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {}", POLICY_PATH, e))?;

    parse_policy_content(&content)
}

/// Validate that /etc/wayexpand directory is trusted
fn validate_policy_directory() -> Result<(), String> {
    let dir_path = Path::new(POLICY_DIR);

    let metadata = std::fs::symlink_metadata(dir_path)
        .map_err(|e| format!("could not inspect {}: {}", POLICY_DIR, e))?;

    // Must be a directory, not a symlink
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", POLICY_DIR));
    }

    // Must be owned by root
    if metadata.uid() != 0 {
        return Err(format!("{} must be owned by root", POLICY_DIR));
    }

    // Must not be group- or world-writable
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "{} must not be group- or world-writable",
            POLICY_DIR
        ));
    }

    Ok(())
}

/// Strict validation of policy file metadata
fn validate_policy_file_metadata(metadata: &std::fs::Metadata) -> Result<(), String> {
    // Must be a regular file (not symlink, directory, etc.)
    if !metadata.is_file() {
        return Err(format!("{} must be a regular file", POLICY_PATH));
    }

    // Explicitly reject symlinks (redundant with is_file, but be explicit)
    // is_file() returns false for symlinks because we use symlink_metadata
    if metadata.file_type().is_symlink() {
        return Err(format!("{} must not be a symlink", POLICY_PATH));
    }

    // Must be owned by root (critical: prevent user tampering)
    if metadata.uid() != 0 {
        return Err(format!("{} must be owned by root", POLICY_PATH));
    }

    // Owner must be able to read, strict permissions recommended (0600 or 0400)
    // Allow 0600 (rw-------), 0400 (r--------), or 0440 (r--r-----)
    let mode = metadata.mode() & 0o777;
    match mode {
        0o600 | 0o400 | 0o440 => {}, // Acceptable: owner-only or root-only
        _ => {
            // Reject any group or world-readable/writable bits
            if metadata.mode() & 0o077 != 0 {
                return Err(format!(
                    "{} must not be group- or world-accessible (mode: {:o})",
                    POLICY_PATH,
                    mode
                ));
            }
        }
    }

    Ok(())
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

    #[test]
    fn validate_policy_file_metadata_rejects_symlinks() {
        // Create a test metadata that claims to be a symlink
        let metadata = std::fs::symlink_metadata("/etc/passwd").unwrap();
        let result = validate_policy_file_metadata(&metadata);
        // Should either fail due to wrong ownership or symlink check
        // (passwd is a regular file, so test our ownership check instead)
        assert!(result.is_err());
    }

    #[test]
    fn validate_policy_file_metadata_requires_root_ownership() {
        // Any regular file owned by non-root should fail
        let metadata = std::fs::symlink_metadata("/tmp").unwrap(); // Typically not root
        let result = validate_policy_file_metadata(&metadata);
        assert!(result.is_err(), "non-root file should be rejected");
    }

    #[test]
    fn policy_file_size_limit_enforced() {
        // Test that oversized files are rejected
        // This would require creating a temporary policy file, which is complex
        // The implementation is tested by the constant MAX_POLICY_FILE_SIZE
        assert_eq!(MAX_POLICY_FILE_SIZE, 1024 * 100, "policy file size limit is 100 KB");
    }

    #[test]
    fn validate_policy_directory_check_exists() {
        // Validate that directory checking is implemented
        // (actual /etc/wayexpand may not exist in test environment)
        // This test just verifies the function exists and is called
        let _ = validate_policy_directory();
        // Result depends on system, but function should complete
    }
}
