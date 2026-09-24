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
    // Policy file must not be world-accessible. It may be:
    // - 0600 (root read/write only)
    // - 0400 (root read-only)
    // - 0440 (root read-only, group-readable for unprivileged services)
    // This allows unprivileged WayExpand user services to read organization
    // policy without compromising security through world-readable access.
    if metadata.mode() & 0o007 != 0 {
        return Err(format!(
            "{} must not be world-accessible (mode: {:o})",
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
}
