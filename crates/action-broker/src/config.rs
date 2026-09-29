//! Action Broker Configuration - Action definitions and policies.
//!
//! Defines the configuration schema for available actions and their permissions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Configuration for a single action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionConfig {
    /// The command to execute.
    pub program: String,

    /// Fixed arguments to pass to the program.
    #[serde(default)]
    pub args: Vec<String>,

    /// Maximum execution time in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    /// Environment variables to pass to the action.
    #[serde(default)]
    pub pass_env: Vec<String>,

    /// Whether to enable the action's minimal inherited environment baseline.
    /// This never means inheriting the complete daemon environment.
    #[serde(default)]
    pub inherit_env: bool,

    /// Working directory for the action.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Whether this action is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Optional description of what this action does.
    #[serde(default)]
    pub description: Option<String>,
}

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
        if self.program.is_empty() {
            return Err("program cannot be empty".to_string());
        }
        if self.timeout_ms == 0 {
            return Err("timeout_ms must be > 0".to_string());
        }
        Ok(())
    }
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
        for (id, action) in &self.actions {
            action
                .validate()
                .map_err(|e| format!("action '{}': {}", id, e))?;

            if let Some(cwd) = &action.cwd {
                validate_working_directory(&format!("action '{}' cwd", id), cwd)?;
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
        }
        Ok(())
    }
}

fn validate_working_directory(label: &str, directory: &str) -> Result<(), String> {
    let path = Path::new(directory);
    if !path.is_absolute() {
        return Err(format!("{} must be an absolute path", label));
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("{} '{}' cannot be inspected: {}", label, directory, error))?;
    if !metadata.is_dir() {
        return Err(format!("{} '{}' is not a directory", label, directory));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.mode() & 0o022 != 0 {
            return Err(format!(
                "{} '{}' is writable by group or other users",
                label, directory
            ));
        }
        let uid = rustix::process::geteuid().as_raw();
        if metadata.uid() != uid && metadata.uid() != 0 {
            return Err(format!(
                "{} '{}' is not owned by the current user or root",
                label, directory
            ));
        }
    }
    Ok(())
}

fn validate_absolute_program(action_id: &str, program: &str) -> Result<(), String> {
    let path = Path::new(program);
    if !path.is_absolute() {
        return Err(format!(
            "action '{}': program must be absolute path when require_absolute_paths=true",
            action_id
        ));
    }

    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "action '{}': program '{}' cannot be inspected: {}",
            action_id, program, error
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "action '{}': program '{}' is not a regular file",
            action_id, program
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.mode() & 0o111 == 0 {
            return Err(format!(
                "action '{}': program '{}' is not executable",
                action_id, program
            ));
        }
        if metadata.mode() & 0o022 != 0 {
            return Err(format!(
                "action '{}': program '{}' is writable by group or other users",
                action_id, program
            ));
        }
        let uid = rustix::process::geteuid().as_raw();
        if metadata.uid() != uid && metadata.uid() != 0 {
            return Err(format!(
                "action '{}': program '{}' is not owned by the current user or root",
                action_id, program
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_config_validation() {
        let config = ActionConfig {
            program: "/usr/bin/kubectl".to_string(),
            args: vec!["get".to_string()],
            timeout_ms: 5000,
            pass_env: vec!["KUBECONFIG".to_string()],
            inherit_env: false,
            cwd: None,
            enabled: true,
            description: Some("Get Kubernetes resources".to_string()),
        };
        assert!(config.validate().is_ok());
        assert!(config.is_enabled());
    }

    #[test]
    fn action_config_empty_program() {
        let config = ActionConfig {
            program: String::new(),
            args: vec![],
            timeout_ms: 5000,
            pass_env: vec![],
            inherit_env: false,
            cwd: None,
            enabled: true,
            description: None,
        };
        assert!(config.validate().is_err());
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
                program: "echo".to_string(), // Not absolute
                args: vec![],
                timeout_ms: 5000,
                pass_env: vec![],
                inherit_env: false,
                cwd: None,
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
    fn broker_config_rejects_unimplemented_audit_settings() {
        for toml_str in [
            "audit_enabled = true",
            "audit_path = \"/var/log/wayexpand-actions.log\"",
            "[broker]\naudit_enabled = true",
            "[broker]\naudit_path = \"/var/log/wayexpand-actions.log\"",
        ] {
            assert!(
                BrokerConfig::from_toml(toml_str).is_err(),
                "unimplemented audit setting must be rejected: {toml_str}"
            );
        }
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
pass_env = ["HOME"]
enabled = true
"#;
        let config =
            BrokerConfig::from_toml(toml_str).expect("help-text example must parse successfully");
        assert!(config.require_absolute_paths);
        assert!(config.strict_env);
        assert!(config.get_action("example").is_some());
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
                    program: executable.to_string_lossy().into_owned(),
                    args: vec![],
                    timeout_ms: 1000,
                    pass_env: vec![],
                    inherit_env: false,
                    cwd: None,
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
                    program: "/definitely/missing/wayexpand-action".to_string(),
                    args: vec![],
                    timeout_ms: 1000,
                    pass_env: vec![],
                    inherit_env: false,
                    cwd: None,
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
                    program: std::env::current_exe()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    args: vec![],
                    timeout_ms: 1000,
                    pass_env: vec![],
                    inherit_env: false,
                    cwd: None,
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

        let directory =
            std::env::temp_dir().join(format!("wayexpand-private-cwd-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = BrokerConfig {
            default_cwd: Some(directory.to_string_lossy().into_owned()),
            ..BrokerConfig::default()
        };

        config.validate_and_canonicalize().unwrap();
        let expected = fs::canonicalize(&directory).unwrap();
        assert_eq!(config.default_cwd.as_deref(), expected.to_str());

        config.default_cwd = Some("relative/path".to_string());
        assert!(config.validate().is_err());
        fs::remove_dir(directory).unwrap();
    }
}
