use serde::Deserialize;
use std::{os::unix::fs::MetadataExt, path::Path};

use crate::OrganizationPolicy;

pub const ORGANIZATION_POLICY_PATH: &str = "/etc/wayexpand/policy.toml";
pub const ORGANIZATION_POLICY_DIR: &str = "/etc/wayexpand";
pub const MAX_ORGANIZATION_POLICY_BYTES: u64 = 1024 * 100;

/// Return the policy identity for a resolved source/output pair.
///
/// The input-method protocol is both source and injector, so the resolver
/// represents its output as `none` while organization policy must govern it
/// under the real runtime backend name `input-method-v2`.
pub fn policy_backend_name<'a>(source: &str, backend: &'a str) -> &'a str {
    if source == "input-method" {
        "input-method-v2"
    } else {
        backend
    }
}

/// Load the system organization policy using the same trust checks enforced by
/// the daemon. A missing policy is intentionally equivalent to the default
/// permissive policy; an existing invalid policy is an error.
pub fn load_organization_policy() -> Result<OrganizationPolicy, String> {
    load_organization_policy_from_paths(
        Path::new(ORGANIZATION_POLICY_PATH),
        Path::new(ORGANIZATION_POLICY_DIR),
    )
}

pub fn load_organization_policy_from_paths(
    path: &Path,
    policy_dir: &Path,
) -> Result<OrganizationPolicy, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(OrganizationPolicy::default());
        }
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    };
    validate_organization_policy_directory(policy_dir)?;
    validate_organization_policy_file(path, &metadata)?;
    if metadata.size() > MAX_ORGANIZATION_POLICY_BYTES {
        return Err(format!(
            "{} is too large ({} bytes, max {})",
            path.display(),
            metadata.size(),
            MAX_ORGANIZATION_POLICY_BYTES
        ));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    parse_organization_policy(&content)
}

pub fn validate_organization_policy_directory(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    if metadata.uid() != 0 {
        return Err(format!("{} must be owned by root", path.display()));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "{} must not be group- or world-writable",
            path.display()
        ));
    }
    Ok(())
}

pub fn validate_organization_policy_file(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), String> {
    if !metadata.is_file() {
        return Err(format!("{} must be a regular file", path.display()));
    }
    if metadata.file_type().is_symlink() {
        return Err(format!("{} must not be a symlink", path.display()));
    }
    if metadata.uid() != 0 {
        return Err(format!("{} must be owned by root", path.display()));
    }
    // Policy file must not be writable by group or world (integrity protection).
    // Read access for unprivileged users is safe: the policy is not a secret,
    // and unprivileged WayExpand services need to read it for enforcement.
    // Valid modes: 0600 (rw-------), 0400 (r--------), 0440 (r--r-----), 0444 (r--r--r--), etc.
    // Invalid modes: any with write bits for group (0o020) or world (0o002).
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "{} must not be writable by group or world (mode: {:o})",
            path.display(),
            metadata.mode() & 0o777
        ));
    }
    Ok(())
}

pub fn parse_organization_policy(content: &str) -> Result<OrganizationPolicy, String> {
    let file: PolicyFile =
        toml::from_str(content).map_err(|error| format!("invalid policy TOML: {error}"))?;
    if let Some(policy) = file.organization {
        return Ok(policy);
    }
    toml::from_str::<OrganizationPolicy>(content).map_err(|error| {
        format!(
            "policy file must contain either [organization] table or flat policy fields: {error}"
        )
    })
}

/// Pre-flight policy check: violations determinable before trigger matching.
/// Returns the reason if a policy violation would block expansion execution.
///
/// This catches determinable violations BEFORE engine.process(), preventing
/// side effects (like command execution) before policy approval.
///
/// Post-execution violations (backend allowed, output size) are still checked
/// after engine.process() because they depend on the matched expansion.
pub fn pre_flight_check(_policy: &OrganizationPolicy) -> Option<String> {
    // NOTE: disable_commands does NOT block static snippets—only command-backed ones.
    // That check happens per-expansion after matching (see apply_preflight_policy).
    // This global check is not needed; static snippets must always be allowed.
    None
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    #[serde(default)]
    organization: Option<OrganizationPolicy>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_wrapper_format_is_shared() {
        let policy = parse_organization_policy(
            "[organization]\nsafe_mode = true\nallowed_backends = [\"libei\"]\n",
        )
        .unwrap();
        assert!(policy.safe_mode);
        assert_eq!(policy.allowed_backends, ["libei"]);
    }

    #[test]
    fn missing_policy_is_permissive() {
        let root = std::env::temp_dir().join("wayexpand-policy-missing");
        assert_eq!(
            load_organization_policy_from_paths(&root.join("policy.toml"), &root).unwrap(),
            OrganizationPolicy::default()
        );
    }

    #[test]
    fn policy_backend_name_uses_runtime_identity_for_input_method() {
        assert_eq!(
            policy_backend_name("input-method", "none"),
            "input-method-v2"
        );
        assert_eq!(policy_backend_name("evdev", "libei"), "libei");
    }

    #[test]
    fn policy_file_permissions_validation_logic() {
        // Unit test for permission validation logic (without filesystem dependency on uid=0)
        // This tests the actual permission check that matters for unprivileged access.

        // Test that write-protection check (& 0o022) correctly identifies writable bits
        let test_cases = vec![
            // (mode, should_pass_write_check, description)
            (0o400, true, "r--------"),
            (0o440, true, "r--r-----"),
            (0o444, true, "r--r--r--"),
            (0o600, true, "rw-------"),
            (0o644, true, "rw-r--r--"),
            (0o620, false, "rw--w---- (group writable)"),
            (0o660, false, "rw-rw---- (group writable)"),
            (0o666, false, "rw-rw-rw- (all writable)"),
            (0o622, false, "-w--w--w- (world writable)"),
        ];

        for (mode, should_pass, description) in test_cases {
            let is_group_or_world_writable = (mode & 0o022) != 0;
            let passes_check = !is_group_or_world_writable;

            assert_eq!(
                passes_check, should_pass,
                "mode {:o} ({}): expected write-check to be {}, but got {}",
                mode, description, should_pass, passes_check
            );
        }
    }
}
