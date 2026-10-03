//! Administrator-managed organization policy and its enforcement view.

use super::*;

/// Organization-managed policy for compliance and security.
///
/// Root-owned policies enforce constraints on user expansions, preventing
/// accidental or malicious use in sensitive contexts.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct OrganizationPolicy {
    /// Enable strict policy enforcement. When true, any policy violation
    /// is logged as an error and prevents the expansion from executing.
    /// When false, violations are logged as warnings but expansions proceed.
    /// This allows organizations to audit policy behavior before full enforcement.
    pub safe_mode: bool,

    /// Disable command execution entirely. Overrides individual expansion
    /// command settings. Useful for locked-down environments.
    pub disable_commands: bool,

    /// Disable hotkey execution. Hotkeys still parse but refuse to run.
    pub disable_hotkeys: bool,

    /// Require command-backed expansions and hotkeys to use absolute program
    /// paths. This avoids PATH-dependent command resolution in managed fleets.
    pub require_absolute_commands: bool,

    /// Disable title fallback for app-filtered expansions. When no compositor
    /// app ID is available, matching fails closed instead of using the
    /// user-editable window title. App IDs remain eligible for matching.
    pub disable_title_matching: bool,

    /// Permit weak app-filter operators (`app_id_glob` and `title_contains`)
    /// while safe mode is enabled. Exact app-ID matching remains the default
    /// and is the only form allowed without this explicit override.
    pub allow_weak_app_filters: bool,

    /// Refuse startup unless the selected injector can replace text as one
    /// externally atomic transaction. Enforced only in safe mode, like the
    /// other administrator-owned requirements.
    pub require_atomic_replace: bool,

    /// Refuse startup unless the selected input source reports sensitive-field
    /// focus (password, PIN, or equivalent).
    pub require_sensitive_focus: bool,

    /// Maximum replacement size in bytes. Replacements larger than this
    /// are rejected. Prevents DoS via huge expansions. 0 = unlimited.
    pub max_replacement_size: usize,

    /// Allowed output backends. If non-empty, only these backends are allowed.
    /// Examples: "libei", "input-method-v2", "ibus", "wlroots", "none"
    pub allowed_backends: Vec<String>,

    /// Allowed curated packs. If non-empty, only these packs are allowed
    /// in ~/.local/share/wayexpand/packs/. Pack names must match directory names.
    pub allowed_packs: Vec<String>,

    /// Policy violation audit log. When violations occur, they're logged
    /// to journald with this prefix for easy filtering.
    pub audit_prefix: String,
}

impl Default for OrganizationPolicy {
    fn default() -> Self {
        Self {
            safe_mode: false,
            disable_commands: false,
            disable_hotkeys: false,
            require_absolute_commands: false,
            disable_title_matching: false,
            allow_weak_app_filters: false,
            require_atomic_replace: false,
            require_sensitive_focus: false,
            max_replacement_size: 0,
            allowed_backends: Vec::new(),
            allowed_packs: Vec::new(),
            audit_prefix: "wayexpand-policy".to_string(),
        }
    }
}

impl OrganizationPolicy {
    /// Return the policy values that are allowed to block behavior.
    ///
    /// Audit mode deliberately retains the original policy for violation
    /// reporting, but contributes no enforcement values to the engine. This
    /// keeps policy detection and policy enforcement separate at every
    /// backend boundary.
    pub fn effective_enforcement_policy(&self) -> Self {
        if self.safe_mode {
            return self.clone();
        }

        let mut effective = self.clone();
        effective.disable_commands = false;
        effective.disable_hotkeys = false;
        effective.require_absolute_commands = false;
        effective.disable_title_matching = false;
        effective.require_atomic_replace = false;
        effective.require_sensitive_focus = false;
        effective.max_replacement_size = 0;
        effective.allowed_backends.clear();
        effective.allowed_packs.clear();
        effective
    }

    /// Check if any policies are active
    pub fn is_active(&self) -> bool {
        self.safe_mode
            || self.disable_commands
            || self.disable_hotkeys
            || self.require_absolute_commands
            || self.disable_title_matching
            || self.allow_weak_app_filters
            || self.require_atomic_replace
            || self.require_sensitive_focus
            || self.max_replacement_size > 0
            || !self.allowed_backends.is_empty()
            || !self.allowed_packs.is_empty()
    }

    /// Check if policy allows a backend
    pub fn backend_allowed(&self, backend: &str) -> bool {
        if self.allowed_backends.is_empty() {
            true
        } else {
            self.allowed_backends.iter().any(|b| b == backend)
        }
    }

    /// Return a human-readable capability violation for a selected deployment.
    pub fn capability_violation(
        &self,
        injector: crate::InjectorCapabilities,
        sensitive_focus: bool,
    ) -> Option<String> {
        self.capability_violation_for_source(
            injector,
            crate::InputSourceCapabilities {
                sensitive_focus,
                ..crate::InputSourceCapabilities::default()
            },
        )
    }

    /// Check guarantees against the explicit capture-source contract.
    pub fn capability_violation_for_source(
        &self,
        injector: crate::InjectorCapabilities,
        source: crate::InputSourceCapabilities,
    ) -> Option<String> {
        if self.require_atomic_replace && !injector.atomic_replace {
            return Some(
                "selected injector cannot guarantee atomic replacement transactions".into(),
            );
        }
        if self.require_sensitive_focus && !source.sensitive_focus {
            return Some(
                "selected input source cannot report password or sensitive-field focus".into(),
            );
        }
        None
    }

    /// Explain a command-path policy violation. Callers enforce the result in
    /// safe mode and log it while allowing execution in audit mode.
    pub fn command_path_violation(&self, program: &str) -> Option<String> {
        (self.require_absolute_commands && !Path::new(program).is_absolute())
            .then(|| "command program must be an absolute path by organization policy".to_string())
    }

    /// Whether a command-path policy violation should block behavior. Audit
    /// mode reports the same violation but deliberately permits execution.
    pub fn command_path_is_blocked(&self, program: &str) -> bool {
        self.safe_mode && self.command_path_violation(program).is_some()
    }

    /// Check if policy allows a pack
    pub fn pack_allowed(&self, pack_name: &str) -> bool {
        if self.allowed_packs.is_empty() {
            true
        } else {
            self.allowed_packs.iter().any(|p| p == pack_name)
        }
    }

    /// Check if replacement size is allowed
    pub fn replacement_size_allowed(&self, size: usize) -> bool {
        if self.max_replacement_size == 0 {
            true
        } else {
            size <= self.max_replacement_size
        }
    }

    /// Return the complete, content-free explanation for an expansion policy
    /// violation. All input paths use this method so the daemon and IBus do
    /// not drift into different enforcement behavior.
    pub fn expansion_policy_violation(
        &self,
        replacement_size: usize,
        has_command: bool,
        backend: &str,
    ) -> Option<String> {
        let mut violations = Vec::new();
        if has_command && self.disable_commands {
            violations.push("command execution is disabled by organization policy".to_string());
        }
        if !self.replacement_size_allowed(replacement_size) {
            violations.push(format!(
                "replacement size {} bytes exceeds policy limit of {} bytes",
                replacement_size, self.max_replacement_size
            ));
        }
        if !self.backend_allowed(backend) {
            violations.push(format!(
                "backend '{}' is not in allowed list: {:?}",
                backend, self.allowed_backends
            ));
        }
        (!violations.is_empty()).then(|| violations.join("; "))
    }
}
