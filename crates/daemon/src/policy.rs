/// Organization policy enforcement for the daemon.
///
/// Loads policies from /etc/wayexpand/policy.toml and enforces them
/// at runtime, blocking unsafe operations and logging violations to journald.
use std::path::Path;
use tracing::{error, warn};
use wayexpand_core::OrganizationPolicy;

const POLICY_PATH: &str = "/etc/wayexpand/policy.toml";

/// Load organization policy from /etc/wayexpand/policy.toml
///
/// Returns the policy if it exists, or a default (permissive) policy if not.
/// Policy file errors are logged as warnings but don't prevent daemon startup.
pub fn load_policy() -> OrganizationPolicy {
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
            policy
        }
        Err(e) => {
            tracing::warn!("could not load organization policy: {}", e);
            OrganizationPolicy::default()
        }
    }
}

fn load_policy_internal() -> Result<OrganizationPolicy, String> {
    let path = Path::new(POLICY_PATH);

    // Policy file is optional; no file = default policy
    if !path.exists() {
        return Ok(OrganizationPolicy::default());
    }

    // Read and parse policy file
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {}", POLICY_PATH, e))?;

    let policy: OrganizationPolicy =
        toml::from_str(&content).map_err(|e| format!("invalid policy TOML: {}", e))?;

    Ok(policy)
}

/// Check if an expansion should be allowed under the current policy
pub fn check_expansion_allowed(
    policy: &OrganizationPolicy,
    replacement_size: usize,
    has_command: bool,
    backend: &str,
) -> Result<(), String> {
    // Check if commands are allowed
    if has_command && policy.disable_commands {
        return Err("command execution is disabled by organization policy".to_string());
    }

    // Check if replacement size is within limits
    if !policy.replacement_size_allowed(replacement_size) {
        return Err(format!(
            "replacement size {} bytes exceeds policy limit of {} bytes",
            replacement_size, policy.max_replacement_size
        ));
    }

    // Check if backend is allowed
    if !policy.backend_allowed(backend) {
        return Err(format!(
            "backend '{}' is not in allowed list: {:?}",
            backend, policy.allowed_backends
        ));
    }

    Ok(())
}

/// Check if a hotkey should be allowed under the current policy
pub fn check_hotkey_allowed(policy: &OrganizationPolicy) -> Result<(), String> {
    if policy.disable_hotkeys {
        return Err("hotkeys are disabled by organization policy".to_string());
    }
    Ok(())
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

    #[test]
    fn check_expansion_allows_normal_case() {
        let policy = OrganizationPolicy::default();
        assert!(check_expansion_allowed(&policy, 1024, false, "libei").is_ok());
    }

    #[test]
    fn check_expansion_blocks_commands_when_disabled() {
        let policy = OrganizationPolicy {
            disable_commands: true,
            ..Default::default()
        };
        assert!(check_expansion_allowed(&policy, 1024, true, "libei").is_err());
    }

    #[test]
    fn check_expansion_blocks_size_when_exceeded() {
        let policy = OrganizationPolicy {
            max_replacement_size: 1000,
            ..Default::default()
        };
        assert!(check_expansion_allowed(&policy, 2000, false, "libei").is_err());
    }

    #[test]
    fn check_expansion_blocks_disallowed_backend() {
        let policy = OrganizationPolicy {
            allowed_backends: vec!["libei".to_string()],
            ..Default::default()
        };
        assert!(check_expansion_allowed(&policy, 1024, false, "input-method").is_err());
    }

    #[test]
    fn check_hotkey_blocks_when_disabled() {
        let policy = OrganizationPolicy {
            disable_hotkeys: true,
            ..Default::default()
        };
        assert!(check_hotkey_allowed(&policy).is_err());
    }
}
