//! Action Broker Configuration - Action definitions and policies.
//!
//! Defines the configuration schema for available actions and their permissions.

pub use crate::path_security::{is_root_owner, is_user_or_root_owner};
use crate::path_security::{
    resolve_audit_path, validate_absolute_program, validate_audit_path, validate_working_directory,
};
use crate::protocol::MAX_ACTION_ID_BYTES;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs};

/// Configuration for a single action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionConfig {
    /// The command to execute.
    pub program: String,

    /// Fixed arguments to pass to the program.
    #[serde(default)]
    pub args: Vec<String>,

    /// Optional strict parameter schema. Each parameter replaces exactly one
    /// configured argv slot; values are never interpolated into another token.
    #[serde(default)]
    pub parameters: HashMap<String, ActionParameter>,

    /// Maximum execution time in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    /// Environment variable names whose values are read from the broker's
    /// own environment. Client requests cannot override these values.
    #[serde(default)]
    pub server_env: Vec<String>,

    /// Environment variable names that a same-UID client may provide.
    /// Values are still restricted to this allowlist and dangerous loader /
    /// interpreter variables are rejected unless explicitly opted in.
    #[serde(default)]
    pub client_forward_env: Vec<String>,

    /// Permit variables such as LD_PRELOAD or PATH for a deliberately
    /// specialized deployment. Disabled by default.
    #[serde(default)]
    pub allow_dangerous_env: bool,

    /// Whether to enable the action's minimal inherited environment baseline.
    /// This never means inheriting the complete daemon environment.
    #[serde(default)]
    pub inherit_env: bool,

    /// Working directory for the action.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Send uncaptured action stdout/stderr to the broker's own stdout/stderr
    /// (normally the service journal). Disabled by default: output a client
    /// did not ask to capture is discarded, because command output can carry
    /// tokens, API responses, or infrastructure data.
    #[serde(default)]
    pub log_output: bool,

    /// Whether this action is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Optional description of what this action does.
    #[serde(default)]
    pub description: Option<String>,

    /// Optional child-only Landlock filesystem policy. When present, the
    /// action is refused unless the running kernel can enforce the profile.
    #[serde(default)]
    pub sandbox: Option<ActionSandbox>,
}

/// Input contract for one named broker action parameter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActionParameter {
    /// Index in `ActionConfig.args` that this value replaces.
    pub arg_index: usize,
    /// `enum` requires an exact allowlisted value; `text` accepts bounded
    /// UTF-8 without control characters, with a broker-configured maximum.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub max_length: Option<usize>,
}

impl ActionParameter {
    fn validate(&self) -> Result<(), String> {
        match self.kind.as_str() {
            "enum" => {
                if self.values.len() < 2
                    || self.values.len() > 128
                    || self.values.iter().any(|value| {
                        value.is_empty()
                            || value.chars().count() > 256
                            || value.chars().any(char::is_control)
                    })
                    || self.max_length.is_some()
                {
                    return Err("enum parameters need 2..=128 safe values and no max_length".into());
                }
            }
            "text" => {
                if !self.values.is_empty()
                    || !self
                        .max_length
                        .is_some_and(|length| (1..=4096).contains(&length))
                {
                    return Err("text parameters need max_length in 1..=4096 and no values".into());
                }
            }
            _ => return Err("parameter type must be `enum` or `text`".into()),
        }
        Ok(())
    }

    fn accepts(&self, value: &str) -> bool {
        match self.kind.as_str() {
            "enum" => self.values.iter().any(|allowed| allowed == value),
            // A leading `-` would let a free-text value become an option of
            // the configured program (`--kubeconfig=...`, `-o /path`).
            "text" => self.max_length.is_some_and(|maximum| {
                !value.is_empty()
                    && !value.starts_with('-')
                    && value.chars().count() <= maximum
                    && !value.chars().any(char::is_control)
            }),
            _ => false,
        }
    }
}

/// Per-action filesystem grants. Paths are canonicalized when the broker
/// loads its policy. The action also receives read access to its executable,
/// standard runtime directories, `/proc`, and its working directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSandbox {
    /// Additional files or directories the action may read and execute.
    #[serde(default)]
    pub read_paths: Vec<String>,
    /// Files or directories the action may modify or create entries beneath.
    #[serde(default)]
    pub write_paths: Vec<String>,
    /// Deny creation of IP, packet, and netlink sockets in this action.
    /// AF_UNIX remains available for local IPC. The packaged broker service
    /// already denies IP networking globally; this also applies when a
    /// sandboxed broker is launched outside that systemd unit.
    #[serde(default)]
    pub deny_network: bool,
    /// Deny device access for this action. Device paths are excluded by the
    /// Landlock allowlist; device ioctls and Unix-socket descriptor reception
    /// are blocked by seccomp. This cannot be combined with inherited output.
    #[serde(default)]
    pub deny_devices: bool,
}

impl ActionSandbox {
    fn validate(&self) -> Result<(), String> {
        const MAX_PATHS_PER_ACCESS: usize = 64;
        for (label, paths) in [
            ("read_paths", &self.read_paths),
            ("write_paths", &self.write_paths),
        ] {
            if paths.len() > MAX_PATHS_PER_ACCESS {
                return Err(format!(
                    "sandbox.{} exceeds {} paths",
                    label, MAX_PATHS_PER_ACCESS
                ));
            }
            if paths
                .iter()
                .any(|path| !std::path::Path::new(path).is_absolute())
            {
                return Err(format!("sandbox.{} paths must be absolute", label));
            }
        }
        Ok(())
    }
}

/// Upper bound for a configured action timeout (one hour). Larger values are
/// rejected so deadline arithmetic can never overflow `Instant`.
pub const MAX_ACTION_TIMEOUT_MS: u64 = 3_600_000;

fn default_timeout_ms() -> u64 {
    10000 // 10 seconds
}

fn default_enabled() -> bool {
    true
}

impl ActionConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.log_output
            && self
                .sandbox
                .as_ref()
                .is_some_and(|sandbox| sandbox.deny_devices)
        {
            return Err("sandbox.deny_devices cannot be combined with log_output".into());
        }
        if self.program.is_empty() || self.program.contains('\0') {
            return Err("program cannot be empty".to_string());
        }
        if !self.parameters.is_empty() && is_interpreter_program(&self.program) {
            return Err(
                "parameterized actions cannot invoke a shell or language interpreter".into(),
            );
        }
        if self.args.iter().any(|argument| argument.contains('\0')) {
            return Err("args cannot contain NUL characters".to_string());
        }
        if self.parameters.len() > wayexpand_broker_protocol::MAX_ACTION_PARAMETERS {
            return Err("too many action parameters".into());
        }
        let mut occupied_slots = std::collections::HashSet::new();
        for (name, parameter) in &self.parameters {
            if name.is_empty()
                || name.len() > 32
                || !name.as_bytes()[0].is_ascii_lowercase()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            {
                return Err(format!("invalid action parameter name `{name}`"));
            }
            if parameter.arg_index >= self.args.len() || !occupied_slots.insert(parameter.arg_index)
            {
                return Err(format!(
                    "parameter `{name}` has an invalid or duplicate arg_index"
                ));
            }
            parameter
                .validate()
                .map_err(|reason| format!("parameter `{name}`: {reason}"))?;
        }
        if self.server_env.len() > 64 || self.client_forward_env.len() > 64 {
            return Err("environment allowlist is too large".to_string());
        }
        for (label, names) in [
            ("server_env", &self.server_env),
            ("client_forward_env", &self.client_forward_env),
        ] {
            if names.iter().any(|name| {
                name.is_empty()
                    || name.contains('=')
                    || name.contains('\0')
                    || (!self.allow_dangerous_env && is_dangerous_environment_name(name))
            }) {
                return Err(format!(
                    "{} contains an invalid or dangerous environment name",
                    label
                ));
            }
        }
        if self.timeout_ms == 0 {
            return Err("timeout_ms must be > 0".to_string());
        }
        if self.timeout_ms > MAX_ACTION_TIMEOUT_MS {
            return Err(format!("timeout_ms must be <= {}", MAX_ACTION_TIMEOUT_MS));
        }
        Ok(())
    }

    /// Validate submitted values and return the fixed argv with configured
    /// slots replaced. This performs no shell parsing or string interpolation.
    pub fn arguments_for(
        &self,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<Vec<String>, String> {
        let submitted_bytes = values
            .iter()
            .map(|(name, value)| name.len().saturating_add(value.len()))
            .sum::<usize>();
        if values.len() > wayexpand_broker_protocol::MAX_ACTION_PARAMETERS
            || submitted_bytes > wayexpand_broker_protocol::MAX_ACTION_PARAMETER_BYTES
            || values.len() != self.parameters.len()
            || values
                .keys()
                .any(|name| !self.parameters.contains_key(name))
        {
            return Err("submitted parameter names do not match the action schema".into());
        }
        let mut arguments = self.args.clone();
        for (name, schema) in &self.parameters {
            let value = values
                .get(name)
                .ok_or_else(|| format!("required parameter `{name}` is missing"))?;
            if !schema.accepts(value) {
                return Err(format!(
                    "parameter `{name}` does not satisfy its configured schema"
                ));
            }
            arguments[schema.arg_index] = value.clone();
        }
        Ok(arguments)
    }
}

fn is_interpreter_program(program: &str) -> bool {
    let path = std::path::Path::new(program);
    // Check the resolved target too: a symlink named `report` can point at
    // `/bin/sh`.
    std::iter::once(path.to_path_buf())
        .chain(std::fs::canonicalize(path).ok())
        .any(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_interpreter_basename)
        })
}

fn is_interpreter_basename(basename: &str) -> bool {
    let basename = basename.to_ascii_lowercase();
    // Versioned names such as `python3.12`, `ruby3.2`, `perl5.38`, `node18`.
    let unversioned =
        basename.trim_end_matches(|character: char| character.is_ascii_digit() || character == '.');
    matches!(
        unversioned,
        "sh" | "bash"
            | "dash"
            | "ash"
            | "zsh"
            | "fish"
            | "ksh"
            | "mksh"
            | "csh"
            | "tcsh"
            | "busybox"
            | "env"
            | "xargs"
            | "sudo"
            | "doas"
            | "nohup"
            | "setsid"
            | "nice"
            | "timeout"
            | "stdbuf"
            | "python"
            | "pypy"
            | "perl"
            | "ruby"
            | "node"
            | "nodejs"
            | "deno"
            | "bun"
            | "php"
            | "lua"
            | "luajit"
            | "tclsh"
            | "wish"
            | "awk"
            | "gawk"
            | "mawk"
            | "nawk"
            | "pwsh"
    )
}

fn is_dangerous_environment_name(name: &str) -> bool {
    matches!(
        name,
        "LD_PRELOAD"
            | "LD_LIBRARY_PATH"
            | "PYTHONPATH"
            | "PERL5LIB"
            | "RUBYOPT"
            | "BASH_ENV"
            | "ENV"
            | "PATH"
    )
}

/// Complete broker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerConfig {
    /// Optional wrapper accepted by the documented configuration format.
    /// Values in this table are copied into the flat runtime settings by
    /// [`BrokerConfig::from_toml`].
    #[serde(default)]
    pub broker: Option<BrokerSettings>,

    /// Configured actions keyed by action_id.
    #[serde(default)]
    pub actions: HashMap<String, ActionConfig>,

    /// Whether to require absolute command paths.
    /// Defaults to true; if false, bare program names are resolved through
    /// the broker process's PATH and the selected executable is deployment-
    /// dependent.
    #[serde(default = "default_require_absolute_paths")]
    pub require_absolute_paths: bool,

    /// Whether to globally disable an action's minimal inherited environment
    /// and require explicit allowlisted request variables instead.
    #[serde(default = "default_strict_env")]
    pub strict_env: bool,

    /// Default working directory if action doesn't specify one.
    #[serde(default)]
    pub default_cwd: Option<String>,

    /// Optional privacy-preserving JSONL execution audit sink.
    #[serde(default)]
    pub audit_path: Option<String>,

    /// Require each action audit event to be written before returning success.
    /// This is intended for managed deployments; it adds action latency and
    /// fails the caller when the audit sink cannot accept the event.
    #[serde(default)]
    pub audit_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerSettings {
    /// Defaults to true. Set false only when PATH-based resolution is an
    /// intentional deployment choice.
    #[serde(default = "default_require_absolute_paths")]
    pub require_absolute_paths: bool,
    #[serde(default = "default_strict_env")]
    pub strict_env: bool,
    #[serde(default)]
    pub default_cwd: Option<String>,
    #[serde(default)]
    pub audit_path: Option<String>,
    #[serde(default)]
    pub audit_required: bool,
}

fn default_strict_env() -> bool {
    true
}

fn default_require_absolute_paths() -> bool {
    true
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            broker: None,
            actions: HashMap::new(),
            require_absolute_paths: true,
            strict_env: true,
            default_cwd: None,
            audit_path: None,
            audit_required: false,
        }
    }
}

impl BrokerConfig {
    /// Load configuration from TOML file.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        let mut config: Self = toml::from_str(content)?;
        if let Some(settings) = config.broker.take() {
            config.require_absolute_paths = settings.require_absolute_paths;
            config.strict_env = settings.strict_env;
            config.default_cwd = settings.default_cwd;
            config.audit_path = settings.audit_path;
            config.audit_required = settings.audit_required;
        }
        Ok(config)
    }

    /// Get action configuration by ID.
    pub fn get_action(&self, action_id: &str) -> Option<&ActionConfig> {
        self.actions.get(action_id)
    }

    /// Validate entire configuration.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(cwd) = &self.default_cwd {
            validate_working_directory("default_cwd", cwd)?;
        }
        if let Some(path) = &self.audit_path {
            validate_audit_path(path)?;
        }
        for (id, action) in &self.actions {
            if !crate::executor::valid_action_id(id) {
                return Err(format!(
                    "action id must be 1-{} ASCII letters, digits, '.', '_' or '-'",
                    MAX_ACTION_ID_BYTES
                ));
            }
            action
                .validate()
                .map_err(|e| format!("action '{}': {}", id, e))?;

            if let Some(cwd) = &action.cwd {
                validate_working_directory(&format!("action '{}' cwd", id), cwd)?;
            }

            if let Some(sandbox) = &action.sandbox {
                if !self.require_absolute_paths {
                    return Err(format!(
                        "action '{}': sandbox profiles require require_absolute_paths=true",
                        id
                    ));
                }
                sandbox
                    .validate()
                    .map_err(|error| format!("action '{}': {}", id, error))?;
            }

            if self.require_absolute_paths {
                validate_absolute_program(id, &action.program)?;
            }
        }
        Ok(())
    }

    /// Validate the policy and replace every absolute program with its
    /// canonical path. This makes the executable selected at policy load
    /// explicit instead of retaining a symlink or `..` spelling until spawn.
    pub fn validate_and_canonicalize(&mut self) -> Result<(), String> {
        self.validate()?;
        if let Some(path) = self.audit_path.clone() {
            self.audit_path = Some(resolve_audit_path(&path)?.to_string_lossy().into_owned());
        }
        if self.require_absolute_paths {
            for (id, action) in &mut self.actions {
                let canonical = fs::canonicalize(&action.program)
                    .map_err(|error| format!("cannot canonicalize action program: {error}"))?
                    .to_string_lossy()
                    .into_owned();
                validate_absolute_program(id, &canonical)?;
                action.program = canonical;
            }
        }
        if let Some(cwd) = &mut self.default_cwd {
            let canonical = fs::canonicalize(&*cwd)
                .map_err(|error| format!("cannot canonicalize default_cwd: {error}"))?
                .to_string_lossy()
                .into_owned();
            validate_working_directory("default_cwd", &canonical)?;
            *cwd = canonical;
        }
        for (id, action) in &mut self.actions {
            if let Some(cwd) = &mut action.cwd {
                let canonical = fs::canonicalize(&*cwd)
                    .map_err(|error| format!("cannot canonicalize action '{}' cwd: {error}", id))?
                    .to_string_lossy()
                    .into_owned();
                validate_working_directory(&format!("action '{}' cwd", id), &canonical)?;
                *cwd = canonical;
            }
            if let Some(sandbox) = &mut action.sandbox {
                for (label, paths) in [
                    ("read_paths", &mut sandbox.read_paths),
                    ("write_paths", &mut sandbox.write_paths),
                ] {
                    for path in paths.iter_mut() {
                        *path = crate::path_security::canonicalize_sandbox_path(id, label, path)?;
                    }
                }
            }
        }
        self.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_ids_are_safe_for_polkit_prompt_details() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "cluster.read".into(),
            ActionConfig {
                parameters: std::collections::HashMap::new(),
                sandbox: None,
                program: "/usr/bin/true".into(),
                args: vec![],
                timeout_ms: 1000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
            },
        );
        assert!(config.validate().is_ok());
        let action = config.actions.remove("cluster.read").unwrap();
        config.actions.insert("cluster $(id)".into(), action);
        assert!(config.validate().unwrap_err().contains("ASCII letters"));
    }

    #[test]
    fn action_config_validation() {
        let config = ActionConfig {
            parameters: std::collections::HashMap::new(),
            sandbox: None,
            program: "/usr/bin/kubectl".to_string(),
            args: vec!["get".to_string()],
            timeout_ms: 5000,
            server_env: vec!["KUBECONFIG".to_string()],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: false,
            enabled: true,
            description: Some("Get Kubernetes resources".to_string()),
        };
        assert!(config.validate().is_ok());
        assert!(config.is_enabled());
    }

    #[test]
    fn action_parameter_schema_replaces_only_fixed_argument_slots() {
        let mut action = ActionConfig {
            parameters: std::collections::HashMap::new(),
            sandbox: None,
            program: "/usr/bin/kubectl".to_string(),
            args: vec!["get".into(), "pods".into(), "production".into()],
            timeout_ms: 5000,
            server_env: vec![],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: false,
            enabled: true,
            description: None,
        };
        action.parameters.insert(
            "environment".into(),
            ActionParameter {
                arg_index: 2,
                kind: "enum".into(),
                values: vec!["production".into(), "staging".into()],
                max_length: None,
            },
        );
        assert!(action.validate().is_ok());
        let submitted = [("environment".into(), "staging".into())]
            .into_iter()
            .collect();
        assert_eq!(
            action.arguments_for(&submitted).unwrap(),
            ["get", "pods", "staging"]
        );
        let rejected = [("environment".into(), "production; rm -rf /".into())]
            .into_iter()
            .collect();
        assert!(action.arguments_for(&rejected).is_err());
        let unknown = [("other".into(), "staging".into())].into_iter().collect();
        assert!(action.arguments_for(&unknown).is_err());
    }

    #[test]
    fn parameter_schema_rejects_duplicate_and_out_of_range_argument_slots() {
        let mut action = ActionConfig {
            parameters: std::collections::HashMap::new(),
            sandbox: None,
            program: "/usr/bin/true".into(),
            args: vec!["fixed".into()],
            timeout_ms: 5000,
            server_env: vec![],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: false,
            enabled: true,
            description: None,
        };
        let schema = ActionParameter {
            arg_index: 0,
            kind: "text".into(),
            values: vec![],
            max_length: Some(80),
        };
        action.parameters.insert("first".into(), schema.clone());
        action.parameters.insert("second".into(), schema.clone());
        assert!(action.validate().is_err());
        action.parameters.remove("second");
        action.parameters.get_mut("first").unwrap().arg_index = 1;
        assert!(action.validate().is_err());
    }

    #[test]
    fn parameterized_actions_cannot_launch_shells_or_interpreters() {
        let action = ActionConfig {
            parameters: [(
                "value".into(),
                ActionParameter {
                    arg_index: 0,
                    kind: "text".into(),
                    values: vec![],
                    max_length: Some(20),
                },
            )]
            .into_iter()
            .collect(),
            sandbox: None,
            program: "/bin/sh".into(),
            args: vec!["-c".into()],
            timeout_ms: 1000,
            server_env: vec![],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: false,
            enabled: true,
            description: None,
        };
        assert!(action.validate().is_err());
        for program in [
            "/usr/bin/python3.12",
            "/usr/bin/ruby3.2",
            "/usr/bin/node18",
            "/usr/bin/xargs",
            "/bin/busybox",
        ] {
            assert!(is_interpreter_program(program), "{program}");
        }
        assert!(!is_interpreter_program("/usr/bin/kubectl"));
        assert!(!is_interpreter_program("/usr/bin/k3s"));
    }

    #[cfg(unix)]
    #[test]
    fn parameterized_actions_reject_symlinks_to_shells() {
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-broker-interpreter-link-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let link = directory.join("report");
        std::os::unix::fs::symlink("/bin/sh", &link).unwrap();
        assert!(is_interpreter_program(&link.to_string_lossy()));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn text_parameters_cannot_become_program_options() {
        let parameter = ActionParameter {
            arg_index: 0,
            kind: "text".into(),
            values: vec![],
            max_length: Some(64),
        };
        assert!(parameter.accepts("platform"));
        assert!(parameter.accepts("a-b"));
        assert!(!parameter.accepts("--kubeconfig=/tmp/attacker"));
        assert!(!parameter.accepts("-oyaml"));
    }

    fn config_with_timeout(timeout_ms: u64) -> ActionConfig {
        ActionConfig {
            parameters: std::collections::HashMap::new(),
            sandbox: None,
            program: "/bin/true".to_string(),
            args: vec![],
            timeout_ms,
            server_env: vec![],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: false,
            enabled: true,
            description: None,
        }
    }

    #[test]
    fn action_config_timeout_bounds() {
        assert!(config_with_timeout(0).validate().is_err());
        assert!(config_with_timeout(1).validate().is_ok());
        assert!(config_with_timeout(MAX_ACTION_TIMEOUT_MS)
            .validate()
            .is_ok());
        assert!(config_with_timeout(MAX_ACTION_TIMEOUT_MS + 1)
            .validate()
            .is_err());
        assert!(config_with_timeout(u64::MAX).validate().is_err());
    }

    #[test]
    fn action_config_timeout_u64_max_rejected_from_toml() {
        let parsed: ActionConfig =
            toml::from_str("program = \"/bin/true\"\ntimeout_ms = 18446744073709551615\n")
                .expect("u64::MAX is representable in TOML parsing");
        assert!(parsed.validate().is_err());
    }

    #[test]
    fn action_config_empty_program() {
        let config = ActionConfig {
            parameters: std::collections::HashMap::new(),
            sandbox: None,
            program: String::new(),
            args: vec![],
            timeout_ms: 5000,
            server_env: vec![],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: false,
            enabled: true,
            description: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn action_config_rejects_process_argument_and_environment_nuls() {
        let mut config = ActionConfig {
            parameters: std::collections::HashMap::new(),
            sandbox: None,
            program: "/usr/bin/echo".to_string(),
            args: vec!["ok\0bad".to_string()],
            timeout_ms: 5000,
            server_env: vec![],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: false,
            enabled: true,
            description: None,
        };
        assert!(config.validate().is_err());

        config.args = vec![];
        config.client_forward_env = vec!["BAD=NAME".to_string()];
        assert!(config.validate().is_err());

        config.client_forward_env = vec!["BAD\0NAME".to_string()];
        assert!(config.validate().is_err());
    }

    #[test]
    fn action_config_rejects_dangerous_environment_by_default() {
        let mut config = ActionConfig {
            parameters: std::collections::HashMap::new(),
            sandbox: None,
            program: "/usr/bin/echo".to_string(),
            args: vec![],
            timeout_ms: 5000,
            server_env: vec!["LD_PRELOAD".to_string()],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: false,
            enabled: true,
            description: None,
        };
        assert!(config.validate().is_err());
        config.allow_dangerous_env = true;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn broker_config_from_toml() {
        let toml_str = r#"
[broker]
require_absolute_paths = true
strict_env = true

[actions."test_action"]
program = "/usr/bin/echo"
args = ["hello"]
timeout_ms = 5000
"#;
        let config = BrokerConfig::from_toml(toml_str).unwrap();
        assert!(config.require_absolute_paths);
        assert!(config.get_action("test_action").is_some());
    }

    #[test]
    fn broker_config_parses_parameterized_argument_slots() {
        let config = BrokerConfig::from_toml(
            r#"
require_absolute_paths = false

[actions."cluster-list"]
program = "/usr/bin/echo"
args = ["--namespace", "default"]

[actions."cluster-list".parameters.namespace]
type = "text"
arg_index = 1
max_length = 63
"#,
        )
        .unwrap();
        let action = config.get_action("cluster-list").unwrap();
        let values = [("namespace".into(), "platform".into())]
            .into_iter()
            .collect();
        assert_eq!(
            action.arguments_for(&values).unwrap(),
            ["--namespace", "platform"]
        );
    }

    #[test]
    fn broker_requires_absolute_program_paths_by_default() {
        let top_level = BrokerConfig::from_toml(
            r#"
[actions."test"]
program = "/usr/bin/echo"
"#,
        )
        .unwrap();
        assert!(top_level.require_absolute_paths);

        let wrapped = BrokerConfig::from_toml(
            r#"
[broker]
strict_env = true

[actions."test"]
program = "/usr/bin/echo"
"#,
        )
        .unwrap();
        assert!(wrapped.require_absolute_paths);
    }

    #[test]
    fn broker_can_explicitly_opt_out_of_absolute_program_paths() {
        let config = BrokerConfig::from_toml(
            r#"
require_absolute_paths = false

[actions."test"]
program = "echo"
"#,
        )
        .unwrap();
        assert!(!config.require_absolute_paths);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn broker_config_validation() {
        let mut config = BrokerConfig {
            require_absolute_paths: true,
            ..Default::default()
        };
        config.actions.insert(
            "test".to_string(),
            ActionConfig {
                parameters: std::collections::HashMap::new(),
                sandbox: None,
                program: "echo".to_string(), // Not absolute
                args: vec![],
                timeout_ms: 5000,
                server_env: vec![],
                client_forward_env: vec![],
                allow_dangerous_env: false,
                inherit_env: false,
                cwd: None,
                log_output: false,
                enabled: true,
                description: None,
            },
        );
        assert!(config.validate().is_err());
    }

    #[test]
    fn broker_config_rejects_unknown_fields() {
        let toml_str = r#"
require_absolute_paths = true
strict_env = true
nonexistent_field = true

[actions."test"]
program = "/usr/bin/echo"
"#;
        assert!(
            BrokerConfig::from_toml(toml_str).is_err(),
            "unknown fields must be rejected"
        );
    }

    #[test]
    fn broker_config_rejects_legacy_args_prefix_name() {
        let toml_str = r#"
[actions."test"]
program = "/usr/bin/echo"
args_prefix = ["hello"]
"#;
        assert!(BrokerConfig::from_toml(toml_str).is_err());
    }

    #[test]
    fn broker_config_accepts_audit_path() {
        let config = BrokerConfig::from_toml(
            "[broker]\naudit_path = \"/tmp/wayexpand-actions.jsonl\"\naudit_required = true",
        )
        .unwrap();
        assert_eq!(
            config.audit_path.as_deref(),
            Some("/tmp/wayexpand-actions.jsonl")
        );
        assert!(config.audit_required);
    }

    #[test]
    fn broker_config_accepts_documented_broker_table() {
        let toml_str = r#"
[broker]
require_absolute_paths = true
strict_env = true

[actions."test"]
program = "/usr/bin/echo"
"#;
        let config = BrokerConfig::from_toml(toml_str).unwrap();
        assert!(config.require_absolute_paths);
        assert!(config.strict_env);
    }

    #[test]
    fn broker_help_example_config_parses() {
        let toml_str = r#"
[broker]
require_absolute_paths = true
strict_env = true

[actions."example"]
program = "/usr/bin/example"
args = []
timeout_ms = 5000
client_forward_env = ["HOME"]
enabled = true
"#;
        let config =
            BrokerConfig::from_toml(toml_str).expect("help-text example must parse successfully");
        assert!(config.require_absolute_paths);
        assert!(config.strict_env);
        assert!(config.get_action("example").is_some());
    }

    #[test]
    fn action_config_parses_and_validates_a_landlock_profile() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-sandbox-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let toml = format!(
            "[actions.report]\nprogram = '/usr/bin/true'\n[actions.report.sandbox]\nread_paths = ['{}']\nwrite_paths = ['{}']\ndeny_network = true\ndeny_devices = true\n",
            root.display(),
            root.display(),
        );
        let mut config = BrokerConfig::from_toml(&toml).unwrap();
        config.validate_and_canonicalize().unwrap();
        let sandbox = config
            .get_action("report")
            .unwrap()
            .sandbox
            .as_ref()
            .unwrap();
        assert_eq!(sandbox.read_paths, [root.to_string_lossy().as_ref()]);
        assert_eq!(sandbox.write_paths, [root.to_string_lossy().as_ref()]);
        assert!(sandbox.deny_network);
        assert!(sandbox.deny_devices);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_sandbox_profiles_default_to_network_allowed() {
        let sandbox: ActionSandbox = toml::from_str("read_paths = []\nwrite_paths = []").unwrap();
        assert!(!sandbox.deny_network);
        assert!(!sandbox.deny_devices);
    }

    #[test]
    fn device_restriction_rejects_inherited_log_output() {
        let mut action = ActionConfig {
            parameters: std::collections::HashMap::new(),
            sandbox: Some(ActionSandbox {
                deny_devices: true,
                ..Default::default()
            }),
            program: "/usr/bin/true".into(),
            args: vec![],
            timeout_ms: 1000,
            server_env: vec![],
            client_forward_env: vec![],
            allow_dangerous_env: false,
            inherit_env: false,
            cwd: None,
            log_output: true,
            enabled: true,
            description: None,
        };
        assert!(action
            .validate()
            .unwrap_err()
            .contains("deny_devices cannot be combined with log_output"));
        action.log_output = false;
        assert!(action.validate().is_ok());
    }

    #[test]
    fn sandbox_paths_must_be_absolute_and_cannot_grant_root() {
        let relative = ActionSandbox {
            read_paths: vec!["data".into()],
            write_paths: vec![],
            deny_network: false,
            deny_devices: false,
        };
        assert!(relative.validate().unwrap_err().contains("absolute"));
        assert!(
            crate::path_security::canonicalize_sandbox_path("test", "read_paths", "/")
                .unwrap_err()
                .contains("filesystem root")
        );
        assert!(
            crate::path_security::canonicalize_sandbox_path("test", "read_paths", "/dev")
                .unwrap_err()
                .contains("device paths")
        );
    }

    #[test]
    fn absolute_program_validation_requires_a_safe_executable() {
        use std::os::unix::fs::PermissionsExt;

        let source = std::env::current_exe().unwrap();
        let executable = std::env::temp_dir().join(format!(
            "wayexpand-safe-action-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::copy(&source, &executable).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = BrokerConfig {
            actions: HashMap::from([(
                "self".to_string(),
                ActionConfig {
                    parameters: std::collections::HashMap::new(),
                    sandbox: None,
                    program: executable.to_string_lossy().into_owned(),
                    args: vec![],
                    timeout_ms: 1000,
                    server_env: vec![],
                    client_forward_env: vec![],
                    allow_dangerous_env: false,
                    inherit_env: false,
                    cwd: None,
                    log_output: false,
                    enabled: true,
                    description: None,
                },
            )]),
            ..BrokerConfig::default()
        };

        config.validate_and_canonicalize().unwrap();
        assert_eq!(
            config.get_action("self").unwrap().program,
            fs::canonicalize(&executable).unwrap().to_string_lossy()
        );
        fs::remove_file(executable).unwrap();
    }

    #[test]
    fn absolute_program_validation_rejects_missing_and_non_executable_paths() {
        let mut missing = BrokerConfig {
            actions: HashMap::from([(
                "missing".to_string(),
                ActionConfig {
                    parameters: std::collections::HashMap::new(),
                    sandbox: None,
                    program: "/definitely/missing/wayexpand-action".to_string(),
                    args: vec![],
                    timeout_ms: 1000,
                    server_env: vec![],
                    client_forward_env: vec![],
                    allow_dangerous_env: false,
                    inherit_env: false,
                    cwd: None,
                    log_output: false,
                    enabled: true,
                    description: None,
                },
            )]),
            ..BrokerConfig::default()
        };
        assert!(missing.validate_and_canonicalize().is_err());

        let mut directory = BrokerConfig {
            actions: HashMap::from([(
                "directory".to_string(),
                ActionConfig {
                    parameters: std::collections::HashMap::new(),
                    sandbox: None,
                    program: std::env::current_exe()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    args: vec![],
                    timeout_ms: 1000,
                    server_env: vec![],
                    client_forward_env: vec![],
                    allow_dangerous_env: false,
                    inherit_env: false,
                    cwd: None,
                    log_output: false,
                    enabled: true,
                    description: None,
                },
            )]),
            ..BrokerConfig::default()
        };
        assert!(directory.validate_and_canonicalize().is_err());
    }

    #[test]
    fn working_directory_validation_requires_a_private_absolute_directory() {
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        let relative = BrokerConfig {
            default_cwd: Some("relative/path".to_string()),
            ..BrokerConfig::default()
        };
        assert!(relative.validate().is_err());

        // Accepting a directory needs every ancestor owned by this user or
        // root. Package builders put HOME under a build tree owned by
        // another account (Launchpad's /build), where the broker rightly
        // refuses it; there is nowhere private to test acceptance then.
        let home = std::env::var("HOME").unwrap();
        if let Err(reason) = crate::path_security::validate_working_directory("HOME", &home) {
            eprintln!("skipping the accepted-directory case: {reason}");
            return;
        }
        let root = Path::new(&home).join(format!(
            ".wayexpand-private-cwd-root-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = root.join("nested");
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = BrokerConfig {
            default_cwd: Some(directory.to_string_lossy().into_owned()),
            ..BrokerConfig::default()
        };

        config.validate_and_canonicalize().unwrap();
        let expected = fs::canonicalize(&directory).unwrap();
        assert_eq!(config.default_cwd.as_deref(), expected.to_str());
        fs::remove_dir(directory).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
